use super::{
    model::{Direction, PayloadIdentity, Record, TransferKey},
    partial, source, wire,
};
use crate::{model::Route, Engine};
use anyhow::{ensure, Context, Result};
use file_trans_identity::TlsStream;
use file_trans_protocol::{
    read_frame, write_message, Frame, Message, ResumeState, CHECKPOINT_SIZE, CHUNK_SIZE,
};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use tokio::sync::watch;

enum Next {
    Frame(Frame),
    Stopped(Option<String>),
}

impl Engine {
    pub(crate) async fn receive_resumable(
        self: &Arc<Self>,
        stream: &mut TlsStream,
        peer_id: &str,
        id: String,
        identity: PayloadIdentity,
        route: Route,
    ) -> Result<()> {
        let key = TransferKey {
            id,
            peer_id: peer_id.into(),
            direction: Direction::Incoming,
        };
        key.validate()?;
        if !self.network_settings().mode.allows(route) {
            return reply(stream, &key.id, ResumeState::Busy).await;
        }
        let control = self.transfers.control(&key);
        let Ok(_lease) = control.receive_lock.clone().try_lock_owned() else {
            return reply(stream, &key.id, ResumeState::Busy).await;
        };
        let result = self
            .receive_attempt(stream, &key, identity, route, control.revision.subscribe())
            .await;
        if let Err(error) = result {
            let interrupted = self.cancel.is_cancelled()
                || self.store.transfer(&key)?.is_some_and(|record| {
                    record.user_paused || record.peer_paused || record.cancelled
                });
            if !interrupted {
                self.task_failed(&key, &error);
            }
            let _ = reply(
                stream,
                &key.id,
                if interrupted {
                    ResumeState::Paused
                } else {
                    ResumeState::Failed {
                        message: format!("{error:#}"),
                    }
                },
            )
            .await;
        }
        if let Some(record) = self.store.transfer(&key)? {
            if record.cancelled {
                partial::remove(&record).await?;
            }
        }
        Ok(())
    }

    async fn receive_attempt(
        &self,
        stream: &mut TlsStream,
        key: &TransferKey,
        identity: PayloadIdentity,
        route: Route,
        mut revision: watch::Receiver<u64>,
    ) -> Result<()> {
        let mut network = self.route_revision.subscribe();
        let Some(super::handshake::Prepared {
            mut record,
            mut file,
            mut hash,
        }) = self
            .prepare_receive(stream, key, identity, route, &mut revision, &mut network)
            .await?
        else {
            return Ok(());
        };
        let mut position = record.activity.transferred;
        let mut committed = position;
        loop {
            let next = self
                .next_transfer_frame(stream, &mut revision, &mut network)
                .await?;
            match next {
                Next::Stopped(error) => {
                    wire::during(
                        stream,
                        partial::checkpoint(
                            self,
                            &record,
                            &mut file,
                            position,
                            &hash,
                            "waiting_connection",
                        ),
                    )
                    .await?;
                    if let Some(error) = error {
                        self.store.update_transfer(key, |record| {
                            record.activity.error = Some(error);
                            Ok(())
                        })?;
                    }
                    return Ok(());
                }
                Next::Frame(Frame::Control(Message::TransferKeepAlive)) => {}
                Next::Frame(Frame::Control(Message::TransferChunk {
                    offset,
                    length,
                    sha256,
                })) => {
                    ensure!(
                        offset == position
                            && length > 0
                            && length as usize <= CHUNK_SIZE
                            && position + u64::from(length) <= record.activity.size
                            && position + u64::from(length) - committed <= CHECKPOINT_SIZE,
                        "文件块偏移或长度无效"
                    );
                    let bytes = match self
                        .next_transfer_frame(stream, &mut revision, &mut network)
                        .await?
                    {
                        Next::Frame(Frame::Data(bytes)) => bytes,
                        Next::Stopped(_) => {
                            partial::checkpoint(
                                self,
                                &record,
                                &mut file,
                                position,
                                &hash,
                                "waiting_connection",
                            )
                            .await?;
                            return Ok(());
                        }
                        _ => anyhow::bail!("文件块缺少二进制内容"),
                    };
                    ensure!(
                        bytes.len() == length as usize
                            && hex::encode(Sha256::digest(&bytes)) == sha256,
                        "文件块内容校验失败"
                    );
                    // 不在磁盘写入中间取消 future；独占锁持有到实际写入结束。
                    tokio::io::AsyncWriteExt::write_all(&mut file, &bytes)
                        .await
                        .context("写入断点文件失败")?;
                    hash.update(&bytes);
                    position += bytes.len() as u64;
                }
                Next::Frame(Frame::Control(Message::TransferCheckpoint {
                    offset,
                    prefix_sha256,
                })) => {
                    ensure!(
                        offset == position && source::digest(&hash) == prefix_sha256,
                        "文件检查点不一致"
                    );
                    wire::during(
                        stream,
                        partial::checkpoint(
                            self,
                            &record,
                            &mut file,
                            position,
                            &hash,
                            "transferring",
                        ),
                    )
                    .await?;
                    committed = position;
                    if write_message(stream, &Message::TransferAck { offset: position })
                        .await
                        .is_err()
                    {
                        self.store.update_transfer(key, |record| {
                            record.state("waiting_connection");
                            Ok(())
                        })?;
                        return Ok(());
                    }
                }
                Next::Frame(Frame::Control(Message::TransferFinish { id })) => {
                    ensure!(
                        id == key.id
                            && position == record.activity.size
                            && Some(source::digest(&hash)) == record.sha256,
                        "文件长度或最终 SHA-256 校验失败"
                    );
                    record = wire::during(
                        stream,
                        partial::checkpoint(self, &record, &mut file, position, &hash, "verifying"),
                    )
                    .await?;
                    if record.cancelled || record.user_paused || record.peer_paused {
                        return Ok(());
                    }
                    drop(file);
                    let path = wire::during(stream, async {
                        if record.is_folder() {
                            crate::folders::commit(self, &record).await
                        } else {
                            partial::commit(self, &record).await
                        }
                    })
                    .await?;
                    record.final_path = Some(path);
                    self.finish_received(&record)?;
                    if record.is_folder() {
                        // 正式目录已经提交，临时数据清理失败不能把成功任务变成失败。
                        let _ = partial::remove(&record).await;
                    }
                    let _ = reply(stream, &key.id, ResumeState::Completed).await;
                    return Ok(());
                }
                _ => anyhow::bail!("文件会话收到无效消息"),
            }
        }
    }

    pub(super) fn stopping(&self, record: &Record, network: &watch::Receiver<u64>) -> bool {
        self.cancel.is_cancelled()
            || record.cancelled
            || record.user_paused
            || record.peer_paused
            || network.has_changed().unwrap_or(true)
    }

    pub(super) fn finish_received(&self, record: &Record) -> Result<()> {
        self.store.update_transfer(&record.key, |current| {
            current.final_path = record.final_path.clone();
            current.activity.path = record
                .final_path
                .as_ref()
                .map(|path| path.to_string_lossy().into_owned());
            current.activity.transferred = current.activity.size;
            current.activity.status = "completed".into();
            current.activity.error = None;
            current.committing = false;
            current.cancelled = false;
            Ok(())
        })?;
        Ok(())
    }

    async fn next_transfer_frame(
        &self,
        stream: &mut TlsStream,
        control: &mut watch::Receiver<u64>,
        network: &mut watch::Receiver<u64>,
    ) -> Result<Next> {
        tokio::select! {
            _ = self.cancel.cancelled() => Ok(Next::Stopped(None)),
            _ = control.changed() => Ok(Next::Stopped(None)),
            _ = network.changed() => Ok(Next::Stopped(None)),
            frame = tokio::time::timeout(crate::engine::IO_TIMEOUT, read_frame(stream)) => {
                match frame {
                    Ok(Ok(frame)) => Ok(Next::Frame(frame)),
                    Ok(Err(error)) if error.downcast_ref::<std::io::Error>().is_none() => Err(error),
                    other => Ok(Next::Stopped(Some(format!("连接中断，已保留断点：{other:?}")))),
                }
            }
        }
    }
}

pub(super) async fn reply(stream: &mut TlsStream, id: &str, result: ResumeState) -> Result<()> {
    tokio::time::timeout(
        crate::engine::IO_TIMEOUT,
        write_message(
            stream,
            &Message::TransferReply {
                id: id.into(),
                result,
            },
        ),
    )
    .await?
}
