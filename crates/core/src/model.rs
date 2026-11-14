use file_trans_discovery::DiscoveredDevice;
use file_trans_protocol::Device;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Peer {
    pub id: String,
    pub name: String,
    pub address: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Activity {
    pub id: String,
    pub peer_id: String,
    pub direction: String,
    pub kind: String,
    pub content: String,
    pub status: String,
    pub size: u64,
    pub transferred: u64,
    pub path: Option<String>,
    pub error: Option<String>,
    pub timestamp: u64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Pairing {
    pub id: String,
    pub peer: Peer,
    pub direction: String,
    pub code: String,
    pub status: String,
    pub error: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub local: Device,
    pub listen_address: String,
    pub local_addresses: Vec<String>,
    pub download_dir: String,
    pub discovered: Vec<DiscoveredDevice>,
    pub discovery_error: Option<String>,
    pub peers: Vec<Peer>,
    pub pairings: Vec<Pairing>,
    pub activities: Vec<Activity>,
}

pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

impl Activity {
    pub fn new(
        id: String,
        peer_id: &str,
        direction: &str,
        kind: &str,
        content: String,
        size: u64,
    ) -> Self {
        Self {
            id,
            peer_id: peer_id.into(),
            direction: direction.into(),
            kind: kind.into(),
            content,
            status: "connecting".into(),
            size,
            transferred: 0,
            path: None,
            error: None,
            timestamp: now(),
        }
    }
}
