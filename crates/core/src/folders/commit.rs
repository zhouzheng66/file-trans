use super::{archive, manifest};
use crate::{transfers::model::Record, Engine};
use anyhow::{ensure, Context, Result};
use std::path::{Path, PathBuf};

fn staging(record: &Record) -> Result<PathBuf> {
    Ok(record
        .partial_path
        .as_ref()
        .context("目录快照丢失")?
        .with_extension("tree"))
}

pub(crate) async fn remove_tree(record: &Record) -> Result<()> {
    if let Some(path) = record.partial_path.as_ref().filter(|_| record.is_folder()) {
        let path = path.with_extension("tree");
        match tokio::fs::symlink_metadata(&path).await {
            Ok(metadata) => {
                ensure!(
                    manifest::ordinary(&metadata) && metadata.is_dir(),
                    "临时展开目录类型无效"
                );
                tokio::fs::remove_dir_all(path).await?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

pub(crate) async fn verified_final(record: &Record) -> Result<bool> {
    let Some(path) = &record.final_path else {
        return Ok(false);
    };
    let work = async {
        let entries = manifest::scan(path, &|| false).await?;
        let (size, hash) =
            archive::encode(path, &entries, &mut tokio::io::sink(), &|| false).await?;
        Ok::<_, anyhow::Error>(size == record.activity.size && Some(hash) == record.sha256)
    }
    .await;
    Ok(work.unwrap_or(false))
}

pub(crate) async fn commit(engine: &Engine, record: &Record) -> Result<PathBuf> {
    let directory = record.destination.as_ref().context("接收目录丢失")?;
    let archive = record.partial_path.as_ref().context("目录快照丢失")?;
    let tree = staging(record)?;
    remove_tree(record).await?;
    tokio::fs::create_dir(&tree).await?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        tokio::fs::set_permissions(&tree, std::fs::Permissions::from_mode(0o700)).await?;
    }
    let stop = || {
        engine.cancel.is_cancelled()
            || engine
                .store
                .transfer(&record.key)
                .ok()
                .flatten()
                .is_none_or(|current| {
                    current.cancelled || current.user_paused || current.peer_paused
                })
    };
    let result = async {
        archive::unpack(archive, &tree, &stop).await?;
        for index in 0..10_000 {
            let name = if index == 0 {
                record.activity.content.clone()
            } else {
                format!("{} ({index})", record.activity.content)
            };
            let destination = directory.join(name);
            // 也避开同名普通文件和失效链接；Windows 对这些冲突未必返回 AlreadyExists。
            match tokio::fs::symlink_metadata(&destination).await {
                Ok(_) => continue,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
            engine.store.update_transfer(&record.key, |current| {
                ensure!(
                    !engine.cancel.is_cancelled()
                        && !current.cancelled
                        && !current.user_paused
                        && !current.peer_paused,
                    "目录提交前已暂停或取消"
                );
                current.committing = true;
                current.final_path = Some(destination.clone());
                current.state("verifying");
                Ok(())
            })?;
            match rename_without_overwrite(&tree, &destination) {
                Ok(()) => return Ok(destination),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error).context("提交接收文件夹失败"),
            }
        }
        anyhow::bail!("同名目录过多")
    }
    .await;
    if result.is_err() {
        remove_tree(record).await?;
    }
    result
}

fn rename_without_overwrite(from: &Path, to: &Path) -> std::io::Result<()> {
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        use rustix::fs::{renameat_with, RenameFlags, CWD};
        // 不回退为可能覆盖既有空目录的普通 Unix rename。
        Ok(renameat_with(CWD, from, CWD, to, RenameFlags::NOREPLACE)?)
    }
    #[cfg(windows)]
    {
        std::fs::rename(from, to)
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
    {
        let _ = (from, to);
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "此平台暂不支持目录安全提交",
        ))
    }
}
