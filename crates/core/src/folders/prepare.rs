use super::{archive, manifest};
use crate::{transfers::model::Record, Engine};
use anyhow::{ensure, Context, Result};
use std::path::Path;

pub(crate) async fn prepare(engine: &Engine, record: &Record) -> Result<bool> {
    let _permit = tokio::select! {
        _ = engine.cancel.cancelled() => return Ok(false),
        permit = engine.outbound.clone().acquire_owned() => permit?,
    };
    let root = Path::new(record.activity.path.as_deref().context("源目录路径丢失")?);
    ensure!(
        !engine.transfer_cache.starts_with(root),
        "源文件夹不能包含应用快照缓存，请选择更具体的子目录"
    );
    let stop = || {
        engine.cancel.is_cancelled()
            || engine
                .store
                .transfer(&record.key)
                .ok()
                .flatten()
                .is_none_or(|current| current.cancelled || current.user_paused)
    };
    let work = async {
        let entries = manifest::scan(root, &stop).await?;
        tokio::fs::create_dir_all(&engine.transfer_cache).await?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            tokio::fs::set_permissions(
                &engine.transfer_cache,
                std::fs::Permissions::from_mode(0o700),
            )
            .await?;
        }
        let temporary = tempfile::Builder::new()
            .prefix(".building-")
            .tempfile_in(&engine.transfer_cache)?;
        let mut output = tokio::fs::File::from_std(temporary.reopen()?);
        let encoded = archive::encode(root, &entries, &mut output, &stop).await;
        // 即使准备被暂停，也等磁盘写入结束后再释放临时文件。
        tokio::io::AsyncWriteExt::flush(&mut output).await?;
        let (size, hash) = encoded?;
        output.sync_data().await.context("保存目录快照失败")?;
        drop(output);
        if stop() {
            return Ok(false);
        }
        let path = engine
            .transfer_cache
            .join(format!("{}.bundle", record.key.id));
        temporary
            .persist(&path)
            .map_err(|error| error.error)
            .context("保存目录快照失败")?;
        // 关闭全部写入句柄、完成重命名后再取时间戳，避免平台延迟更新时间造成误报。
        let modified = crate::transfers::source::modified(&tokio::fs::metadata(&path).await?);
        let saved = engine.store.update_transfer(&record.key, |current| {
            ensure!(!current.cancelled, "目录任务已取消");
            current.source_path = Some(path.clone());
            current.source_modified = modified;
            current.sha256 = Some(hash);
            current.activity.size = size;
            current.state("connecting");
            Ok(())
        });
        if saved.is_err() {
            let _ = tokio::fs::remove_file(path).await;
        }
        saved?;
        Ok(true)
    }
    .await;
    if stop() {
        Ok(false)
    } else {
        work
    }
}

pub(crate) async fn cleanup(record: &Record) -> Result<()> {
    if record.is_folder()
        && ["completed", "rejected", "cancelled"].contains(&record.activity.status.as_str())
    {
        if let Some(path) = &record.source_path {
            match tokio::fs::remove_file(path).await {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
        }
    }
    Ok(())
}
