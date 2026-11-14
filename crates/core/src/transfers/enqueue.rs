use super::{
    model::{Direction, Record, TransferKey},
    source,
};
use crate::Engine;
use anyhow::{ensure, Context, Result};
use std::{path::PathBuf, sync::Arc};

impl Engine {
    /// 原生文件选择和系统拖拽共用入口；类型判定放在可信后端。
    pub async fn send_path(self: &Arc<Self>, peer_id: &str, path: PathBuf) -> Result<String> {
        let metadata = tokio::fs::symlink_metadata(&path)
            .await
            .context("路径不可用")?;
        ensure!(
            crate::folders::ordinary(&metadata),
            "不支持链接、重解析点或特殊文件"
        );
        self.enqueue_path(peer_id, path, metadata.is_dir()).await
    }

    pub async fn send_file(self: &Arc<Self>, peer_id: &str, path: PathBuf) -> Result<String> {
        self.enqueue_path(peer_id, path, false).await
    }

    pub async fn send_folder(self: &Arc<Self>, peer_id: &str, path: PathBuf) -> Result<String> {
        self.enqueue_path(peer_id, path, true).await
    }

    async fn enqueue_path(
        self: &Arc<Self>,
        peer_id: &str,
        path: PathBuf,
        folder: bool,
    ) -> Result<String> {
        ensure!(!self.cancel.is_cancelled(), "应用正在关闭");
        self.store.peer(peer_id)?;
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .context("名称无效")?
            .to_owned();
        crate::files::safe_file_name(&name)?;
        if folder {
            crate::folders::component(&name)?;
        }
        let metadata = tokio::fs::symlink_metadata(&path)
            .await
            .context("源路径不可用")?;
        ensure!(
            crate::folders::ordinary(&metadata)
                && metadata.is_dir() == folder
                && (folder || metadata.len() <= 8 * 1024_u64.pow(4)),
            "请选择普通文件或文件夹，文件最多 8 TiB"
        );
        let path = tokio::fs::canonicalize(path)
            .await
            .context("源路径不可用")?;
        let key = TransferKey {
            id: uuid::Uuid::new_v4().to_string(),
            peer_id: peer_id.into(),
            direction: Direction::Outgoing,
        };
        let mut record = Record::new(key.clone(), name, if folder { 0 } else { metadata.len() });
        if folder {
            record.activity.kind = "folder".into();
        }
        record.activity.path = Some(path.to_string_lossy().into_owned());
        record.source_modified = source::modified(&metadata);
        record.activity.status = "preparing".into();
        self.store.insert_transfer(&record)?;
        self.schedule_transfer(&key)?;
        Ok(key.id)
    }
}
