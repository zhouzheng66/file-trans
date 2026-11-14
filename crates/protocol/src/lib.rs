use anyhow::{bail, ensure, Result};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub const VERSION: u16 = 1;
pub const MAX_TEXT: usize = 64 * 1024;
pub const MAX_CONTROL: usize = 512 * 1024;
pub const CHUNK_SIZE: usize = 256 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Device {
    pub id: String,
    pub name: String,
    pub port: u16,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Message {
    PairRequest {
        version: u16,
        device: Device,
    },
    PairChallenge {
        request_id: String,
        device: Device,
    },
    PairDecision {
        accepted: bool,
    },
    PairResult {
        accepted: bool,
    },
    Ping {
        version: u16,
    },
    Pong {
        device: Device,
    },
    Text {
        version: u16,
        id: String,
        text: String,
    },
    FileOffer {
        version: u16,
        id: String,
        name: String,
        size: u64,
    },
    FileDecision {
        accepted: bool,
    },
    FileEnd {
        sha256: String,
    },
    Ack {
        id: String,
    },
    Error {
        message: String,
    },
}

#[derive(Debug)]
pub enum Frame {
    Control(Message),
    Data(Vec<u8>),
}

pub async fn write_message<W: AsyncWrite + Unpin>(io: &mut W, message: &Message) -> Result<()> {
    let data = serde_json::to_vec(message)?;
    ensure!(data.len() <= MAX_CONTROL, "控制消息过大");
    write_frame(io, 1, &data).await
}

pub async fn write_chunk<W: AsyncWrite + Unpin>(io: &mut W, data: &[u8]) -> Result<()> {
    ensure!(
        !data.is_empty() && data.len() <= CHUNK_SIZE,
        "文件块大小无效"
    );
    write_frame(io, 2, data).await
}

async fn write_frame<W: AsyncWrite + Unpin>(io: &mut W, kind: u8, data: &[u8]) -> Result<()> {
    io.write_u8(kind).await?;
    io.write_u32(data.len() as u32).await?;
    io.write_all(data).await?;
    io.flush().await?;
    Ok(())
}

pub async fn read_frame<R: AsyncRead + Unpin>(io: &mut R) -> Result<Frame> {
    let kind = io.read_u8().await?;
    let length = io.read_u32().await? as usize;
    let maximum = match kind {
        1 => MAX_CONTROL,
        2 => CHUNK_SIZE,
        _ => bail!("未知帧类型"),
    };
    ensure!(length > 0 && length <= maximum, "数据帧长度无效");
    let mut data = vec![0; length];
    io.read_exact(&mut data).await?;
    match kind {
        1 => Ok(Frame::Control(serde_json::from_slice(&data)?)),
        _ => Ok(Frame::Data(data)),
    }
}

pub async fn read_message<R: AsyncRead + Unpin>(io: &mut R) -> Result<Message> {
    match read_frame(io).await? {
        Frame::Control(Message::Error { message }) => bail!("对端：{message}"),
        Frame::Control(message) => Ok(message),
        Frame::Data(_) => bail!("预期控制消息，却收到文件块"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn rejects_oversized_header_before_allocating_body() {
        let bytes = [1, 0xff, 0xff, 0xff, 0xff];
        assert!(read_frame(&mut bytes.as_slice()).await.is_err());
    }

    #[tokio::test]
    async fn accepts_maximum_text_even_when_json_escapes_every_character() {
        let text = "\u{0001}".repeat(MAX_TEXT);
        let mut bytes = Vec::new();
        write_message(
            &mut bytes,
            &Message::Text {
                version: VERSION,
                id: "test".into(),
                text: text.clone(),
            },
        )
        .await
        .unwrap();
        let message = read_message(&mut bytes.as_slice()).await.unwrap();
        assert!(matches!(message, Message::Text { text: received, .. } if received == text));
    }
}
