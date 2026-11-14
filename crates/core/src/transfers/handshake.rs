use super::{
    model::{PayloadIdentity, Record, TransferKey},
    partial,
    receiver::reply,
    wire,
};
use crate::{model::Route, Engine};
use anyhow::{ensure, Context, Result};
use file_trans_identity::TlsStream;
use file_trans_protocol::ResumeState;
use sha2::Sha256;
use tokio::{fs::File, sync::watch};

pub(super) struct Prepared {
    pub record: Record,
    pub file: File,
    pub hash: Sha256,
}

impl Engine {
    pub(super) async fn prepare_receive(
        &self,
        stream: &mut TlsStream,
        key: &TransferKey,
        payload: PayloadIdentity,
        route: Route,
        revision: &mut watch::Receiver<u64>,
        network: &mut watch::Receiver<u64>,
    ) -> Result<Option<Prepared>> {
        self.store.peer(&key.peer_id)?;
        let identity = payload.file;
        if payload.folder {
            crate::folders::component(&identity.name)?;
        }
        let name = crate::files::safe_file_name(&identity.name)?;
        ensure!(
            identity.size <= 8 * 1024_u64.pow(4)
                && identity.sha256.len() == 64
                && identity.sha256.bytes().all(|byte| byte.is_ascii_hexdigit()),
            "文件身份无效"
        );
        let digest = identity.sha256.to_ascii_lowercase();
        let mut record = if let Some(record) = self.store.transfer(key)? {
            ensure!(
                record.is_folder() == payload.folder
                    && record.activity.content == name
                    && record.activity.size == identity.size
                    && record.sha256.as_deref() == Some(digest.as_str()),
                "同一任务的文件身份发生变化，拒绝续传"
            );
            record
        } else {
            let mut record = Record::new(key.clone(), name, identity.size);
            if payload.folder {
                record.activity.kind = "folder".into();
            }
            record.sha256 = Some(digest);
            record.activity.status = "awaiting_accept".into();
            record.destination = Some(self.download_dir.lock().unwrap().clone());
            record.activity.route = Some(route);
            self.store.insert_transfer(&record)?;
            record
        };
        if record.cancelled || record.activity.status == "rejected" {
            reply(stream, &key.id, ResumeState::Rejected).await?;
            return Ok(None);
        }
        if record.activity.status == "completed" || record.committing {
            let verified = wire::during(stream, partial::verified_final(&record)).await?;
            if verified {
                self.finish_received(&record)?;
                let _ = partial::remove(&record).await;
                reply(stream, &key.id, ResumeState::Completed).await?;
                return Ok(None);
            }
            ensure!(
                record.activity.status != "completed",
                "已完成的接收文件已被移动、删除或修改"
            );
            record = self.store.update_transfer(key, |current| {
                current.committing = false;
                current.final_path = None;
                Ok(())
            })?;
        }
        if record.activity.status == "failed" {
            reply(
                stream,
                &key.id,
                ResumeState::Failed {
                    message: record
                        .activity
                        .error
                        .unwrap_or_else(|| "接收任务出错，请在接收端恢复或取消".into()),
                },
            )
            .await?;
            return Ok(None);
        }
        record = self.store.update_transfer(key, |record| {
            record.peer_paused = false;
            record.activity.route = Some(route);
            record.state(if record.accepted {
                "waiting_connection"
            } else {
                "awaiting_accept"
            });
            Ok(())
        })?;
        if record.user_paused {
            reply(stream, &key.id, ResumeState::Paused).await?;
            return Ok(None);
        }
        if !record.accepted {
            let decision = self.receipts.register(&key.token())?;
            let choice = wire::during(stream, async {
                tokio::select! {
                    _ = self.cancel.cancelled() => Ok(None),
                    _ = revision.changed() => Ok(None),
                    _ = network.changed() => Ok(None),
                    decision = tokio::time::timeout(self.decision_timeout, decision) => Ok(Some(decision.context("接收确认已超时")?.context("接收确认已取消")?)),
                }
            }).await;
            self.receipts.remove(&key.token());
            let Some(accepted) = choice? else {
                return Ok(None);
            };
            record = self.store.update_transfer(key, |record| {
                record.accepted = accepted;
                if !accepted {
                    record.activity.status = "rejected".into();
                }
                Ok(())
            })?;
            if !accepted {
                reply(stream, &key.id, ResumeState::Rejected).await?;
                return Ok(None);
            }
        }
        if self.stopping(&record, network) {
            if record.user_paused || record.peer_paused {
                reply(stream, &key.id, ResumeState::Paused).await?;
            }
            return Ok(None);
        }
        let (file, hash) = wire::during(stream, partial::open(self, &mut record)).await?;
        record = self.store.transfer(key)?.context("接收任务不存在")?;
        if self.stopping(&record, network) {
            if record.user_paused || record.peer_paused {
                reply(stream, &key.id, ResumeState::Paused).await?;
            }
            return Ok(None);
        }
        reply(
            stream,
            &key.id,
            ResumeState::Ready {
                offset: record.activity.transferred,
                prefix_sha256: record.prefix_sha256.clone(),
            },
        )
        .await?;
        self.store.update_transfer(key, |record| {
            record.activity.error = None;
            record.activity.resumed_from = record
                .activity
                .resumed_from
                .max(record.activity.transferred);
            record.state("transferring");
            Ok(())
        })?;
        Ok(Some(Prepared { record, file, hash }))
    }
}
