use crate::model::Activity;
use anyhow::{ensure, Result};
pub use file_trans_protocol::Direction;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransferKey {
    pub id: String,
    pub peer_id: String,
    pub direction: Direction,
}

impl TransferKey {
    pub fn token(&self) -> String {
        format!("{}:{}:{}", self.direction.as_str(), self.peer_id, self.id)
    }
    pub(crate) fn validate(&self) -> Result<()> {
        uuid::Uuid::parse_str(&self.id)?;
        ensure!(
            self.peer_id.len() == 64 && self.peer_id.bytes().all(|byte| byte.is_ascii_hexdigit()),
            "设备指纹无效"
        );
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct Record {
    pub key: TransferKey,
    pub activity: Activity,
    pub sha256: Option<String>,
    pub source_modified: Option<u64>,
    #[serde(default)]
    pub source_path: Option<PathBuf>,
    pub partial_path: Option<PathBuf>,
    pub destination: Option<PathBuf>,
    pub final_path: Option<PathBuf>,
    pub prefix_sha256: String,
    pub accepted: bool,
    pub user_paused: bool,
    pub peer_paused: bool,
    pub cancelled: bool,
    pub committing: bool,
    #[serde(default)]
    pub control_sequence: u64,
    #[serde(default)]
    pub peer_control_sequence: u64,
}

impl Record {
    pub fn is_folder(&self) -> bool {
        self.activity.kind == "folder"
    }
    pub fn terminal(&self) -> bool {
        ["completed", "rejected", "cancelled", "failed"].contains(&self.activity.status.as_str())
    }
    pub fn state(&mut self, state: &str) {
        self.activity.status = if self.cancelled {
            "cancelled"
        } else if self.user_paused {
            "paused"
        } else if self.peer_paused {
            "paused_by_peer"
        } else {
            state
        }
        .into();
    }
    pub fn new(key: TransferKey, name: String, size: u64) -> Self {
        let mut activity = Activity::new(
            key.id.clone(),
            &key.peer_id,
            key.direction.as_str(),
            "file",
            name,
            size,
        );
        activity.resumable = true;
        Self {
            key,
            activity,
            sha256: None,
            source_modified: None,
            source_path: None,
            partial_path: None,
            destination: None,
            final_path: None,
            prefix_sha256: empty_hash(),
            accepted: false,
            user_paused: false,
            peer_paused: false,
            cancelled: false,
            committing: false,
            control_sequence: 0,
            peer_control_sequence: 0,
        }
    }
}

pub(crate) struct PayloadIdentity {
    pub file: file_trans_protocol::FileIdentity,
    pub folder: bool,
}

pub(crate) fn empty_hash() -> String {
    use sha2::Digest;
    hex::encode(sha2::Sha256::digest([]))
}
