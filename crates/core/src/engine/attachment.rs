use super::Engine;
use crate::{
    folders::ordinary,
    transfers::{Direction, TransferKey},
};
use anyhow::{ensure, Context, Result};
use std::{fs, path::PathBuf};

impl Engine {
    /// 根据聊天记录定位发送原件或已落盘的接收附件，兼容只有活动记录的旧传输。
    pub fn local_attachment_path(&self, key: &TransferKey) -> Result<PathBuf> {
        let activity = self.store.activity(key)?.context("找不到附件记录")?;
        ensure!(
            matches!(activity.kind.as_str(), "file" | "folder"),
            "该消息没有附件"
        );
        ensure!(
            key.direction == Direction::Outgoing || activity.status == "completed",
            "文件尚未接收完成"
        );
        let path = std::path::absolute(activity.path.context("附件记录缺少本地路径")?)?;
        let metadata = fs::symlink_metadata(&path).context("无法访问附件，文件可能已移动或删除")?;
        ensure!(
            ordinary(&metadata) && metadata.is_dir() == (activity.kind == "folder"),
            "附件类型已改变，请检查原位置"
        );
        Ok(path)
    }
}
