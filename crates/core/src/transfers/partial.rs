use super::{model::Record, source};
use crate::{files::safe_file_name, Engine};
use anyhow::{ensure, Context, Result};
use sha2::Sha256;
use std::path::{Path, PathBuf};
use tempfile::{NamedTempFile, TempPath};
use tokio::{
    fs::{File, OpenOptions},
    io::AsyncWriteExt,
};

pub(crate) async fn open(engine: &Engine, record: &mut Record) -> Result<(File, Sha256)> {
    let directory = record.destination.as_ref().context("接收目录丢失")?;
    let staging = directory
        .join(".file-trans-partials")
        .join(&engine.local.id);
    tokio::fs::create_dir_all(&staging).await?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        tokio::fs::set_permissions(&staging, std::fs::Permissions::from_mode(0o700)).await?;
    }
    let path = staging.join(format!("{}-{}.part", record.key.peer_id, record.key.id));
    if let Some(expected) = &record.partial_path {
        ensure!(*expected == path, "断点路径不匹配");
    }
    if let Ok(metadata) = tokio::fs::symlink_metadata(&path).await {
        ensure!(
            metadata.is_file() && !metadata.is_symlink(),
            "断点文件类型无效"
        );
    } else {
        ensure!(record.activity.transferred == 0, "断点文件已丢失，无法续传");
    }
    let mut options = OpenOptions::new();
    options
        .read(true)
        .write(true)
        .create(record.activity.transferred == 0);
    #[cfg(unix)]
    {
        options.mode(0o600);
    }
    let mut file = options.open(&path).await?;
    ensure!(
        file.metadata().await?.len() >= record.activity.transferred,
        "断点文件被截断"
    );
    file.set_len(record.activity.transferred).await?;
    let hash = source::prefix(&mut file, record.activity.transferred).await?;
    ensure!(
        source::digest(&hash) == record.prefix_sha256,
        "断点内容已损坏，请取消后重新发送"
    );
    engine.store.update_transfer(&record.key, |record| {
        record.partial_path = Some(path);
        Ok(())
    })?;
    Ok((file, hash))
}

pub(crate) async fn checkpoint(
    engine: &Engine,
    record: &Record,
    file: &mut File,
    position: u64,
    hash: &Sha256,
    state: &str,
) -> Result<Record> {
    file.flush().await?;
    file.sync_data().await.context("保存文件断点失败")?;
    engine.store.update_transfer(&record.key, |record| {
        record.activity.transferred = position;
        record.prefix_sha256 = source::digest(hash);
        record.state(state);
        Ok(())
    })
}

pub(crate) async fn verified_final(record: &Record) -> Result<bool> {
    if record.is_folder() {
        return crate::folders::verified_final(record).await;
    }
    let Some(path) = &record.final_path else {
        return Ok(false);
    };
    let Ok(metadata) = tokio::fs::symlink_metadata(path).await else {
        return Ok(false);
    };
    if !metadata.is_file() || metadata.is_symlink() || metadata.len() != record.activity.size {
        return Ok(false);
    }
    let mut file = File::open(path).await?;
    Ok(Some(source::digest(
        &source::prefix(&mut file, record.activity.size).await?,
    )) == record.sha256)
}

pub(crate) async fn commit(engine: &Engine, record: &Record) -> Result<PathBuf> {
    let directory = record.destination.as_ref().context("接收目录丢失")?;
    let path = record.partial_path.as_ref().context("断点文件不存在")?;
    let name = safe_file_name(&record.activity.content)?;
    for index in 0..10_000 {
        let destination = directory.join(candidate(&name, index));
        engine.store.update_transfer(&record.key, |record| {
            ensure!(
                !record.cancelled && !record.user_paused && !record.peer_paused,
                "文件提交前已暂停或取消"
            );
            record.committing = true;
            record.final_path = Some(destination.clone());
            record.state("verifying");
            Ok(())
        })?;
        // TempPath 在失败时必须 keep，不能因句柄析构删除可续传文件。
        let std_file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)?;
        let temporary = NamedTempFile::from_parts(std_file, TempPath::try_from_path(path.clone())?);
        match temporary.persist_noclobber(&destination) {
            Ok(_) => return Ok(destination),
            Err(error) => {
                let kind = error.error.kind();
                let cause = error.error;
                error
                    .file
                    .into_temp_path()
                    .keep()
                    .context("保留断点文件失败")?;
                if kind != std::io::ErrorKind::AlreadyExists {
                    return Err(cause).context("提交接收文件失败");
                }
            }
        }
    }
    anyhow::bail!("同名文件过多")
}

pub(crate) async fn remove(record: &Record) -> Result<()> {
    crate::folders::remove_tree(record).await?;
    if let Some(path) = &record.partial_path {
        match tokio::fs::remove_file(path).await {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error).context("清理断点文件失败"),
        }
    }
    Ok(())
}

fn candidate(name: &str, index: usize) -> String {
    if index == 0 {
        return name.into();
    }
    let path = Path::new(name);
    let stem = path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("file");
    match path.extension().and_then(|value| value.to_str()) {
        Some(extension) => format!("{stem} ({index}).{extension}"),
        None => format!("{stem} ({index})"),
    }
}
