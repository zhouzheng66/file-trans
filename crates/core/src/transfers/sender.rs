use super::{
    model::{Record, TransferKey},
    source, wire,
};
use crate::{
    model::{NetworkMode, Route},
    Engine,
};
use anyhow::{ensure, Context, Result};
use file_trans_identity::TlsStream;
use file_trans_protocol::{
    write_chunk, write_message, FileIdentity, Message, ResumeState, CHECKPOINT_SIZE, CHUNK_SIZE,
    FILE_VERSION,
};
use sha2::{Digest, Sha256};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::{fs::File, io::AsyncReadExt, time::timeout};

enum Outcome {
    Completed,
    Rejected,
    Paused,
    Reconnect,
    Fatal(String),
}

impl Engine {
    pub(crate) async fn run_transfer(self: &Arc<Self>, key: TransferKey) {
        let control = self.transfers.control(&key);
        let mut revision = control.revision.subscribe();
        let mut network = self.route_revision.subscribe();
        let mut delay = 1;
        let mut probe = Instant::now();
        loop {
            revision.borrow_and_update();
            network.borrow_and_update();
            let Ok(Some(record)) = self.store.transfer(&key) else {
                break;
            };
            if record.terminal() {
                break;
            }
            if self.cancel.is_cancelled() {
                break;
            }
            if record.user_paused {
                tokio::select! { _ = self.cancel.cancelled() => break, _ = revision.changed() => {} }
                continue;
            }
            if let Err(error) = self.store.peer(&key.peer_id) {
                self.task_failed(&key, &error);
                break;
            }
            if record.is_folder() && record.source_path.is_none() {
                match crate::folders::prepare(self, &record).await {
                    Ok(_) => continue,
                    Err(error) => {
                        self.task_failed(&key, &error);
                        break;
                    }
                }
            }
            let work = async {
                let _permit = self.outbound.clone().acquire_owned().await?;
                let mut file = match source::open(&record).await {
                    Ok(file) => file,
                    Err(error) => return Ok(Outcome::Fatal(format!("{error:#}"))),
                };
                if record.sha256.is_none() {
                    self.store.update_transfer(&key, |record| {
                        record.state("preparing");
                        Ok(())
                    })?;
                    let hash = match source::prefix(&mut file, record.activity.size).await {
                        Ok(hash) => hash,
                        Err(error) => return Ok(Outcome::Fatal(format!("{error:#}"))),
                    };
                    if let Err(error) = source::check(&file.metadata().await?, &record) {
                        return Ok(Outcome::Fatal(error.to_string()));
                    }
                    self.store.update_transfer(&key, |record| {
                        record.sha256 = Some(source::digest(&hash));
                        record.state("connecting");
                        Ok(())
                    })?;
                }
                let record = self.store.transfer(&key)?.context("文件任务不存在")?;
                self.send_attempt(&record, file, &mut probe).await
            };
            let result = tokio::select! {
                _ = self.cancel.cancelled() => break,
                _ = revision.changed() => continue,
                _ = network.changed() => continue,
                result = work => result,
            };
            match result {
                Ok(Outcome::Completed) => {
                    let _ = self.store.update_transfer(&key, |record| {
                        record.activity.transferred = record.activity.size;
                        record.activity.status = "completed".into();
                        record.activity.error = None;
                        Ok(())
                    });
                    break;
                }
                Ok(Outcome::Rejected) => {
                    let _ = self.store.update_transfer(&key, |record| {
                        record.activity.status = "rejected".into();
                        Ok(())
                    });
                    break;
                }
                Ok(Outcome::Fatal(message)) => {
                    self.task_failed(&key, &anyhow::anyhow!(message));
                    break;
                }
                Ok(Outcome::Reconnect) => {
                    delay = 1;
                    continue;
                }
                Ok(Outcome::Paused) => {
                    let _ = self.store.update_transfer(&key, |record| {
                        record.peer_paused = true;
                        record.state("paused_by_peer");
                        Ok(())
                    });
                    delay = 3;
                }
                Err(error) => {
                    let _ = self.store.update_transfer(&key, |record| {
                        record.activity.reconnect_count =
                            record.activity.reconnect_count.saturating_add(1);
                        record.activity.error = Some(format!("{error:#}"));
                        record.state("waiting_retry");
                        Ok(())
                    });
                }
            }
            tokio::select! { _ = self.cancel.cancelled() => break, _ = revision.changed() => {}, _ = network.changed() => {}, _ = tokio::time::sleep(Duration::from_secs(delay)) => {} }
            delay = (delay * 2).min(10);
        }
        if let Ok(Some(record)) = self.store.transfer(&key) {
            let _ = crate::folders::cleanup(&record).await;
            if !record.terminal() {
                let _ = self.store.update_transfer(&key, |record| {
                    record.state("waiting_retry");
                    Ok(())
                });
            }
        }
    }

    async fn send_attempt(
        &self,
        record: &Record,
        mut file: File,
        last_probe: &mut Instant,
    ) -> Result<Outcome> {
        self.store.update_transfer(&record.key, |record| {
            record.state("connecting");
            Ok(())
        })?;
        let connection = self.open_peer(&record.key.peer_id).await?;
        let route = connection.route;
        let mut stream = connection.stream;
        self.store.update_transfer(&record.key, |record| {
            record.activity.route = Some(route);
            record.state("awaiting_accept");
            Ok(())
        })?;
        let hash = record.sha256.clone().context("源文件指纹丢失")?;
        let identity = FileIdentity {
            name: record.activity.content.clone(),
            size: record.activity.size,
            sha256: hash,
        };
        let offer = if record.is_folder() {
            Message::FolderOffer {
                version: file_trans_protocol::FOLDER_VERSION,
                id: record.key.id.clone(),
                folder: identity,
            }
        } else {
            Message::TransferOffer {
                version: FILE_VERSION,
                id: record.key.id.clone(),
                file: identity,
            }
        };
        write_message(&mut stream, &offer).await?;
        let response = timeout(
            self.decision_timeout + crate::engine::IO_TIMEOUT,
            read_reply(&mut stream, &record.key.id),
        )
        .await??;
        let (mut position, prefix) = match response {
            ResumeState::Completed => return Ok(Outcome::Completed),
            ResumeState::Rejected => return Ok(Outcome::Rejected),
            ResumeState::Paused => return Ok(Outcome::Paused),
            ResumeState::Busy => anyhow::bail!("对端正在保存上一条连接的断点，稍后重试"),
            ResumeState::Failed { message } => return Ok(Outcome::Fatal(message)),
            ResumeState::Ready {
                offset,
                prefix_sha256,
            } => (offset, prefix_sha256),
        };
        if position > record.activity.size {
            return Ok(Outcome::Fatal("对端断点超过文件大小".into()));
        }
        let mut hasher = match wire::during(&mut stream, source::prefix(&mut file, position)).await
        {
            Ok(hash) => hash,
            Err(error) => return Ok(Outcome::Fatal(format!("{error:#}"))),
        };
        if source::digest(&hasher) != prefix {
            return Ok(Outcome::Fatal(
                "双方文件前缀不一致，拒绝拼接不同内容".into(),
            ));
        }
        self.store.update_transfer(&record.key, |record| {
            record.peer_paused = false;
            record.accepted = true;
            record.activity.transferred = position;
            record.activity.resumed_from = record.activity.resumed_from.max(position);
            record.prefix_sha256 = prefix;
            record.activity.error = None;
            record.state("transferring");
            Ok(())
        })?;
        let mut buffer = vec![0; CHUNK_SIZE];
        while position < record.activity.size {
            let end = (position + CHECKPOINT_SIZE).min(record.activity.size);
            while position < end {
                let length = (end - position).min(CHUNK_SIZE as u64) as usize;
                if let Err(error) = file.read_exact(&mut buffer[..length]).await {
                    return Ok(Outcome::Fatal(format!("源文件读取失败：{error}")));
                }
                timeout(crate::engine::IO_TIMEOUT, async {
                    write_message(
                        &mut stream,
                        &Message::TransferChunk {
                            offset: position,
                            length: length as u32,
                            sha256: hex::encode(Sha256::digest(&buffer[..length])),
                        },
                    )
                    .await?;
                    write_chunk(&mut stream, &buffer[..length]).await
                })
                .await??;
                hasher.update(&buffer[..length]);
                position += length as u64;
            }
            let prefix = source::digest(&hasher);
            write_message(
                &mut stream,
                &Message::TransferCheckpoint {
                    offset: position,
                    prefix_sha256: prefix.clone(),
                },
            )
            .await?;
            ensure!(
                matches!(wire::read(&mut stream).await?, Message::TransferAck { offset } if offset == position),
                "断点确认偏移不匹配"
            );
            self.store.update_transfer(&record.key, |record| {
                record.activity.transferred = position;
                record.prefix_sha256 = prefix;
                record.state("transferring");
                Ok(())
            })?;
            if let Err(error) = source::check(&file.metadata().await?, record) {
                return Ok(Outcome::Fatal(error.to_string()));
            }
            if route == Route::Relay
                && self.network_settings().mode == NetworkMode::Auto
                && last_probe.elapsed() >= Duration::from_secs(10)
            {
                *last_probe = Instant::now();
                let peer = self.store.peer(&record.key.peer_id)?;
                if self
                    .open_lan(&record.key.peer_id, peer.address)
                    .await
                    .is_ok()
                {
                    return Ok(Outcome::Reconnect);
                }
            }
        }
        self.store.update_transfer(&record.key, |record| {
            record.state("verifying");
            Ok(())
        })?;
        write_message(
            &mut stream,
            &Message::TransferFinish {
                id: record.key.id.clone(),
            },
        )
        .await?;
        match read_reply(&mut stream, &record.key.id).await? {
            ResumeState::Completed => Ok(Outcome::Completed),
            ResumeState::Failed { message } => Ok(Outcome::Fatal(message)),
            _ => anyhow::bail!("无效文件完成确认"),
        }
    }

    pub(crate) fn task_failed(&self, key: &TransferKey, error: &anyhow::Error) {
        let _ = self.store.update_transfer(key, |record| {
            if !record.cancelled && record.activity.status != "completed" {
                record.activity.status = "failed".into();
                record.activity.error = Some(format!("{error:#}"));
            }
            Ok(())
        });
    }
}

async fn read_reply(stream: &mut TlsStream, id: &str) -> Result<ResumeState> {
    ensure!(id.len() <= 36, "任务 ID 无效");
    match wire::read(stream).await? {
        Message::TransferReply {
            id: received,
            result,
        } if received == id => Ok(result),
        _ => anyhow::bail!("对端未支持可续传文件协议或任务 ID 不匹配，请更新两端客户端"),
    }
}
