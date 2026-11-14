use super::{validate_id, Engine, IO_TIMEOUT};
use crate::{
    files::{persist_without_overwrite, safe_file_name},
    model::Activity,
};
use anyhow::{ensure, Context, Result};
use file_trans_identity::TlsStream;
use file_trans_protocol::{
    read_frame, read_message, write_chunk, write_message, Frame, Message, CHUNK_SIZE, MAX_TEXT,
    VERSION,
};
use sha2::{Digest, Sha256};
use std::{path::PathBuf, sync::Arc};
use tokio::{
    fs::File,
    io::{AsyncReadExt, AsyncWriteExt},
    time::timeout,
};
use uuid::Uuid;

impl Engine {
    pub async fn send_text(&self, peer_id: &str, text: String) -> Result<()> {
        ensure!(
            !text.trim().is_empty() && text.len() <= MAX_TEXT,
            "文字为空或超过 64 KiB"
        );
        self.store.peer(peer_id)?;
        let _permit = self.outbound_permit().await?;
        let id = Uuid::new_v4().to_string();
        let mut activity = Activity::new(
            id.clone(),
            peer_id,
            "outgoing",
            "text",
            text.clone(),
            text.len() as u64,
        );
        self.record(&activity)?;
        let result = tokio::select! {
            _ = self.cancel.cancelled() => Err(anyhow::anyhow!("应用已退出")),
            result = timeout(IO_TIMEOUT, async {
                let mut stream = self.open_peer(peer_id).await?;
                write_message(&mut stream, &Message::Text { version: VERSION, id: id.clone(), text }).await?;
                ensure_ack(&mut stream, &id).await
            }) => result.context("文字发送超时").and_then(|result| result),
        };
        if let Err(error) = result {
            self.failed(&mut activity, &error);
            return Err(error);
        }
        activity.status = "completed".into();
        activity.transferred = activity.size;
        self.record(&activity)
    }

    pub(crate) async fn receive_text(
        &self,
        stream: &mut TlsStream,
        peer_id: &str,
        id: String,
        text: String,
    ) -> Result<()> {
        validate_id(&id)?;
        ensure!(
            !text.trim().is_empty() && text.len() <= MAX_TEXT,
            "文字为空或过大"
        );
        let mut activity = Activity::new(
            id.clone(),
            peer_id,
            "incoming",
            "text",
            text.clone(),
            text.len() as u64,
        );
        activity.status = "completed".into();
        activity.transferred = activity.size;
        self.store.receive_text(&activity)?;
        timeout(IO_TIMEOUT, write_message(stream, &Message::Ack { id })).await??;
        Ok(())
    }

    pub async fn send_file(self: &Arc<Self>, peer_id: &str, path: PathBuf) -> Result<String> {
        self.store.peer(peer_id)?;
        let permit = self.outbound_permit().await?;
        let file = File::open(&path).await.context("无法打开源文件")?;
        let metadata = file.metadata().await?;
        ensure!(metadata.is_file(), "请选择普通文件，当前不支持文件夹");
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .context("文件名称不是有效 Unicode")?
            .to_owned();
        safe_file_name(&name)?;
        let id = Uuid::new_v4().to_string();
        let mut activity = Activity::new(
            id.clone(),
            peer_id,
            "outgoing",
            "file",
            name,
            metadata.len(),
        );
        activity.path = Some(path.to_string_lossy().into_owned());
        self.record(&activity)?;
        let engine = Arc::clone(self);
        tokio::spawn(async move {
            let _permit = permit;
            let result = tokio::select! {
                _ = engine.cancel.cancelled() => Err(anyhow::anyhow!("应用已退出")),
                result = engine.upload(&mut activity, file) => result,
            };
            if let Err(error) = result {
                engine.failed(&mut activity, &error);
            }
        });
        Ok(id)
    }

    async fn upload(&self, activity: &mut Activity, mut file: File) -> Result<()> {
        let mut stream = self.open_peer(&activity.peer_id).await?;
        activity.status = "awaiting_accept".into();
        self.record(activity)?;
        timeout(
            IO_TIMEOUT,
            write_message(
                &mut stream,
                &Message::FileOffer {
                    version: VERSION,
                    id: activity.id.clone(),
                    name: activity.content.clone(),
                    size: activity.size,
                },
            ),
        )
        .await??;
        let response = timeout(
            self.decision_timeout + IO_TIMEOUT,
            read_message(&mut stream),
        )
        .await
        .context("等待接收确认超时")??;
        let Message::FileDecision { accepted } = response else {
            anyhow::bail!("无效文件接收响应")
        };
        if !accepted {
            activity.status = "rejected".into();
            self.record(activity)?;
            return Ok(());
        }
        activity.status = "transferring".into();
        self.record(activity)?;
        let mut buffer = vec![0; CHUNK_SIZE];
        let mut hash = Sha256::new();
        loop {
            let length = timeout(IO_TIMEOUT, file.read(&mut buffer)).await??;
            if length == 0 {
                break;
            }
            ensure!(
                activity.transferred + length as u64 <= activity.size,
                "发送过程中源文件大小发生变化"
            );
            timeout(IO_TIMEOUT, write_chunk(&mut stream, &buffer[..length]))
                .await
                .context("发送文件块超时")??;
            hash.update(&buffer[..length]);
            activity.transferred += length as u64;
            self.progress(activity);
        }
        ensure!(
            activity.transferred == activity.size,
            "发送过程中源文件被截断"
        );
        activity.status = "verifying".into();
        self.record(activity)?;
        timeout(IO_TIMEOUT, async {
            write_message(
                &mut stream,
                &Message::FileEnd {
                    sha256: hex::encode(hash.finalize()),
                },
            )
            .await?;
            ensure_ack(&mut stream, &activity.id).await
        })
        .await
        .context("等待文件保存确认超时")??;
        activity.status = "completed".into();
        self.record(activity)
    }

    pub(crate) async fn receive_file(
        &self,
        stream: &mut TlsStream,
        peer_id: &str,
        id: String,
        name: String,
        size: u64,
    ) -> Result<()> {
        validate_id(&id)?;
        let name = safe_file_name(&name)?;
        ensure!(
            size <= 8 * 1024_u64.pow(4),
            "文件大小超过当前协议上限 8 TiB"
        );
        ensure!(
            !self.store.received(&id, peer_id)?,
            "该文件任务已经接收完成"
        );
        let decision = self.receipts.register(&id)?;
        let mut activity = Activity::new(id.clone(), peer_id, "incoming", "file", name, size);
        activity.status = "awaiting_accept".into();
        let result = async {
            self.record(&activity)?;
            let accepted = timeout(self.decision_timeout, decision)
                .await
                .context("文件接收确认超时")?
                .context("文件接收已取消")?;
            self.store.peer(peer_id)?;
            timeout(
                IO_TIMEOUT,
                write_message(stream, &Message::FileDecision { accepted }),
            )
            .await??;
            if !accepted {
                activity.status = "rejected".into();
                self.record(&activity)?;
                return Ok(());
            }
            activity.status = "transferring".into();
            self.record(&activity)?;
            self.download(stream, &mut activity).await
        }
        .await;
        self.receipts.remove(&id);
        if let Err(error) = &result {
            self.failed(&mut activity, error);
        }
        result
    }

    async fn download(&self, stream: &mut TlsStream, activity: &mut Activity) -> Result<()> {
        let directory = self.download_dir.lock().unwrap().clone();
        let temporary = tempfile::Builder::new()
            .prefix(".file-trans-")
            .tempfile_in(&directory)?;
        let mut file = File::from_std(temporary.reopen()?);
        let mut hash = Sha256::new();
        let expected_hash = loop {
            let frame = timeout(IO_TIMEOUT, read_frame(stream))
                .await
                .context("接收文件块超时")??;
            match frame {
                Frame::Data(bytes) => {
                    ensure!(
                        activity.transferred + bytes.len() as u64 <= activity.size,
                        "收到的数据超过声明的文件大小"
                    );
                    timeout(IO_TIMEOUT, file.write_all(&bytes))
                        .await
                        .context("写入接收文件超时")??;
                    hash.update(&bytes);
                    activity.transferred += bytes.len() as u64;
                    self.progress(activity);
                }
                Frame::Control(Message::FileEnd { sha256 }) => break sha256,
                _ => anyhow::bail!("文件传输期间收到无效消息"),
            }
        };
        ensure!(activity.transferred == activity.size, "文件长度不匹配");
        ensure!(
            hex::encode(hash.finalize()) == expected_hash,
            "文件 SHA-256 校验失败"
        );
        activity.status = "verifying".into();
        self.progress(activity);
        timeout(IO_TIMEOUT, async {
            file.flush().await?;
            file.sync_all().await
        })
        .await??;
        drop(file);
        let path = persist_without_overwrite(temporary, &directory, &activity.content)?;
        activity.path = Some(path.to_string_lossy().into_owned());
        activity.status = "completed".into();
        self.record(activity)?;
        // 最后的 ACK 丢失时，本地仍然是已保存状态，不应把完成记录改为失败。
        if !matches!(
            timeout(
                IO_TIMEOUT,
                write_message(
                    stream,
                    &Message::Ack {
                        id: activity.id.clone()
                    }
                )
            )
            .await,
            Ok(Ok(()))
        ) {
            tracing::warn!("文件已保存，但发送确认超时");
        }
        Ok(())
    }
}

async fn ensure_ack(stream: &mut TlsStream, id: &str) -> Result<()> {
    ensure!(
        matches!(read_message(stream).await?, Message::Ack { id: received } if received == id),
        "无效接收确认"
    );
    Ok(())
}
