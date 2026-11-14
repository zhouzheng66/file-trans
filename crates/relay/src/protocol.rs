use anyhow::{ensure, Result};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub const MAX_CONTROL: usize = 16 * 1024;
pub const MAX_PACKET: usize = 64 * 1024;
pub const VERSION: u16 = 1;
pub const CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(12);

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMessage {
    Invite { request_id: String },
    Open { request_id: String, target: Target },
    Reject { session_id: String },
    Heartbeat,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum Target {
    Peer(String),
    Invite(String),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMessage {
    Registered {
        device_id: String,
        version: u16,
    },
    Invite {
        request_id: String,
        code: String,
        expires_at: u64,
    },
    Opened {
        request_id: String,
        session_id: String,
        ticket: String,
        peer_id: String,
    },
    Incoming {
        session_id: String,
        ticket: String,
        peer_id: String,
        invite: Option<String>,
    },
    Error {
        request_id: String,
        message: String,
    },
    Heartbeat,
}

impl ServerMessage {
    pub fn request_id(&self) -> Option<&str> {
        match self {
            Self::Invite { request_id, .. }
            | Self::Opened { request_id, .. }
            | Self::Error { request_id, .. } => Some(request_id),
            _ => None,
        }
    }
}

#[derive(Serialize, Deserialize)]
pub struct Attach {
    pub session_id: String,
    pub ticket: String,
}

pub async fn read_json<T: DeserializeOwned>(io: &mut (impl AsyncRead + Unpin)) -> Result<T> {
    let length = io.read_u32().await? as usize;
    ensure!(length > 0 && length <= MAX_CONTROL, "中继控制消息大小无效");
    let mut bytes = vec![0; length];
    io.read_exact(&mut bytes).await?;
    Ok(serde_json::from_slice(&bytes)?)
}

pub async fn write_json<T: Serialize>(
    io: &mut (impl AsyncWrite + Unpin),
    message: &T,
) -> Result<()> {
    let bytes = serde_json::to_vec(message)?;
    ensure!(bytes.len() <= MAX_CONTROL, "中继控制消息过大");
    io.write_u32(bytes.len() as u32).await?;
    io.write_all(&bytes).await?;
    io.flush().await?;
    Ok(())
}

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
