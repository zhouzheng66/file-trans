use super::{
    model::{Direction, TransferKey},
    partial,
};
use crate::Engine;
use anyhow::{ensure, Context, Result};
use file_trans_identity::TlsStream;
use file_trans_protocol::{write_message, Message, FILE_VERSION};
use std::sync::Arc;

impl Engine {
    pub(crate) async fn restore_transfers(self: &Arc<Self>) -> Result<()> {
        let records = self.store.transfers()?;
        if let Err(error) = crate::folders::recover(&self.transfer_cache, &records).await {
            tracing::warn!(%error, "回收未引用的目录快照失败");
        }
        for record in records {
            if record.key.direction == Direction::Incoming
                && record.is_folder()
                && ["completed", "cancelled"].contains(&record.activity.status.as_str())
            {
                if let Err(error) = partial::remove(&record).await {
                    tracing::warn!(%error, "清理已结束目录任务的临时数据失败");
                }
            }
            if record.key.direction == Direction::Outgoing {
                if let Err(error) = crate::folders::cleanup(&record).await {
                    tracing::warn!(%error, "清理已结束目录任务的发送快照失败");
                }
            }
            if record.key.direction == Direction::Outgoing && !record.terminal() {
                self.schedule_transfer(&record.key)?;
            }
        }
        Ok(())
    }

    pub(crate) fn schedule_transfer(self: &Arc<Self>, key: &TransferKey) -> Result<()> {
        ensure!(!self.cancel.is_cancelled(), "应用正在关闭");
        let Some(running) = self.transfers.reserve(key)? else {
            return Ok(());
        };
        let engine = Arc::clone(self);
        let key = key.clone();
        self.transfers.tracker.spawn(async move {
            engine.run_transfer(key.clone()).await;
            drop(running);
            if !engine.cancel.is_cancelled() {
                if let Ok(Some(record)) = engine.store.transfer(&key) {
                    if !record.terminal() && !record.user_paused {
                        let _ = engine.schedule_transfer(&key);
                    }
                }
            }
        });
        Ok(())
    }

    pub async fn pause_transfer(self: &Arc<Self>, key: TransferKey) -> Result<()> {
        key.validate()?;
        let control = self.transfers.control(&key);
        let committing =
            key.direction == Direction::Incoming && control.receive_lock.try_lock().is_err();
        let record = self.store.update_transfer(&key, |record| {
            ensure!(!record.terminal(), "任务已经结束");
            ensure!(
                !(record.committing && committing),
                "文件正在提交，请等待完成"
            );
            record.control_sequence = record.control_sequence.saturating_add(1);
            record.user_paused = true;
            record.state("paused");
            Ok(())
        })?;
        self.transfers.wake(&key);
        self.notify_transfer(key, true, false, record.control_sequence);
        Ok(())
    }

    pub async fn resume_transfer(self: &Arc<Self>, key: TransferKey) -> Result<()> {
        key.validate()?;
        self.store.peer(&key.peer_id)?;
        let record = self.store.update_transfer(&key, |record| {
            ensure!(
                !["completed", "rejected", "cancelled"].contains(&record.activity.status.as_str()),
                "任务已经结束，不能恢复"
            );
            record.control_sequence = record.control_sequence.saturating_add(1);
            record.user_paused = false;
            record.activity.error = None;
            record.state(if key.direction == Direction::Outgoing {
                "waiting_retry"
            } else {
                "waiting_connection"
            });
            Ok(())
        })?;
        self.transfers.wake(&key);
        self.notify_transfer(key.clone(), false, false, record.control_sequence);
        if key.direction == Direction::Outgoing {
            self.schedule_transfer(&key)?;
        }
        Ok(())
    }

    pub async fn cancel_transfer(self: &Arc<Self>, key: TransferKey) -> Result<()> {
        key.validate()?;
        let control = self.transfers.control(&key);
        let committing =
            key.direction == Direction::Incoming && control.receive_lock.try_lock().is_err();
        let record = self.store.update_transfer(&key, |record| {
            ensure!(
                record.activity.status != "completed",
                "已完成的文件不会由取消操作删除"
            );
            ensure!(
                !(record.committing && committing),
                "文件正在提交，请等待完成"
            );
            record.control_sequence = record.control_sequence.saturating_add(1);
            record.cancelled = true;
            record.state("cancelled");
            Ok(())
        })?;
        self.transfers.wake(&key);
        self.notify_transfer(key.clone(), false, true, record.control_sequence);
        if key.direction == Direction::Incoming {
            let control = self.transfers.control(&key);
            let _lease = control.receive_lock.lock().await;
            partial::remove(&self.store.transfer(&key)?.context("任务不存在")?).await?;
        } else {
            self.schedule_transfer(&key)?;
        }
        Ok(())
    }

    pub(crate) fn decide_managed_file(
        &self,
        peer_id: Option<&str>,
        id: &str,
        accepted: bool,
    ) -> Result<()> {
        let records: Vec<_> = self
            .store
            .transfers()?
            .into_iter()
            .filter(|record| {
                record.key.id == id
                    && record.key.direction == Direction::Incoming
                    && peer_id.is_none_or(|id| id == record.key.peer_id)
                    && record.activity.status == "awaiting_accept"
            })
            .collect();
        if records.is_empty() {
            return self.receipts.decide(id, accepted);
        }
        ensure!(records.len() == 1, "存在同名任务，请指定对端设备");
        self.receipts.decide(&records[0].key.token(), accepted)
    }

    fn notify_transfer(
        self: &Arc<Self>,
        key: TransferKey,
        paused: bool,
        cancelled: bool,
        sequence: u64,
    ) {
        let engine = Arc::clone(self);
        self.transfers.tracker.spawn(async move {
            let work = async {
                let mut connection = engine.open_peer(&key.peer_id).await?;
                write_message(&mut connection.stream, &Message::TransferSignal { version: FILE_VERSION, id: key.id, direction: key.direction.opposite(), paused, cancelled, sequence }).await?;
                super::wire::read(&mut connection.stream).await?;
                Ok::<_, anyhow::Error>(())
            };
            tokio::select! { _ = engine.cancel.cancelled() => {}, _ = tokio::time::timeout(crate::engine::IO_TIMEOUT, work) => {} }
        });
    }

    pub(crate) async fn receive_transfer_signal(
        self: &Arc<Self>,
        stream: &mut TlsStream,
        key: TransferKey,
        paused: bool,
        cancelled: bool,
        sequence: u64,
    ) -> Result<()> {
        key.validate()?;
        ensure!(sequence > 0, "控制序号无效");
        let mut applied = false;
        let record = self.store.update_transfer(&key, |record| {
            if sequence <= record.peer_control_sequence
                || record.committing
                || ["completed", "rejected", "cancelled"].contains(&record.activity.status.as_str())
            {
                return Ok(());
            }
            applied = true;
            record.peer_control_sequence = sequence;
            record.peer_paused = paused;
            record.cancelled = cancelled;
            if !paused {
                record.activity.error = None;
            }
            record.state(if key.direction == Direction::Outgoing {
                "waiting_retry"
            } else {
                "waiting_connection"
            });
            Ok(())
        })?;
        if !applied {
            return write_message(stream, &Message::Ack { id: key.id }).await;
        }
        self.transfers.wake(&key);
        if key.direction == Direction::Outgoing && !record.terminal() {
            self.schedule_transfer(&key)?;
        }
        if cancelled && key.direction == Direction::Incoming {
            let control = self.transfers.control(&key);
            let _lease = control.receive_lock.lock().await;
            partial::remove(&record).await?;
        }
        write_message(stream, &Message::Ack { id: key.id }).await
    }
}
