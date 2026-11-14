use super::{validate_name, Engine, IO_TIMEOUT};
use crate::model::{Pairing, Peer};
use anyhow::{ensure, Context, Result};
use file_trans_identity::{peer_id, secure_code, TlsStream};
use file_trans_protocol::{read_message, write_message, Device, Message, VERSION};
use std::{net::SocketAddr, sync::Arc};
use tokio::time::timeout;
use uuid::Uuid;

impl Engine {
    pub async fn request_pair(self: &Arc<Self>, address: SocketAddr) -> Result<String> {
        let permit = self.outbound_permit().await?;
        let mut stream = self.open_address(address, None).await?;
        let remote_id = peer_id(&stream)?;
        ensure!(remote_id != self.local.id, "不能与本机配对");
        let code = secure_code(&stream)?;
        let challenge = timeout(IO_TIMEOUT, async {
            write_message(
                &mut stream,
                &Message::PairRequest {
                    version: VERSION,
                    device: self.local.clone(),
                },
            )
            .await?;
            read_message(&mut stream).await
        })
        .await
        .context("获取配对响应超时")??;
        let Message::PairChallenge { request_id, device } = challenge else {
            anyhow::bail!("无效配对响应")
        };
        super::validate_id(&request_id)?;
        validate_name(&device.name)?;
        ensure!(
            device.id == remote_id && device.port != 0,
            "配对设备身份无效"
        );
        let peer = Peer {
            id: remote_id,
            name: device.name,
            address: SocketAddr::new(address.ip(), device.port).to_string(),
        };
        let decision = self.pairings.register(Pairing {
            id: request_id.clone(),
            peer: peer.clone(),
            direction: "outgoing".into(),
            code,
            status: "pending".into(),
            error: None,
        })?;
        let engine = Arc::clone(self);
        let id = request_id.clone();
        tokio::spawn(async move {
            let _permit = permit;
            let result = tokio::select! {
                _ = engine.cancel.cancelled() => Err(anyhow::anyhow!("应用已退出")),
                result = timeout(engine.decision_timeout, async {
                    let (mut reader, mut writer) = tokio::io::split(stream);
                    let response = read_message(&mut reader);
                    tokio::pin!(response);
                    let accepted = tokio::select! {
                        local = decision => {
                            let local = local.context("配对确认已取消")?;
                            write_message(&mut writer, &Message::PairDecision { accepted: local }).await?;
                            let Message::PairResult { accepted: remote } = response.await? else { anyhow::bail!("无效配对结果") };
                            local && remote
                        }
                        remote = &mut response => {
                            ensure!(matches!(remote?, Message::PairResult { accepted: false }), "对端在双端确认前返回无效结果");
                            false
                        }
                    };
                    if accepted { engine.store.save_peer(&peer)?; }
                    Ok(accepted)
                }) => result.context("配对已超时，请重新发起").and_then(|result| result),
            };
            engine.pairings.finish(&id, &result);
            Ok::<(), anyhow::Error>(())
        });
        Ok(request_id)
    }

    pub(crate) async fn receive_pair(
        &self,
        stream: &mut TlsStream,
        address: SocketAddr,
        device: Device,
    ) -> Result<()> {
        validate_name(&device.name)?;
        let remote_id = peer_id(stream)?;
        ensure!(
            device.id == remote_id && remote_id != self.local.id && device.port != 0,
            "配对设备身份无效"
        );
        let id = Uuid::new_v4().to_string();
        let peer = Peer {
            id: remote_id,
            name: device.name,
            address: SocketAddr::new(address.ip(), device.port).to_string(),
        };
        let decision = self.pairings.register(Pairing {
            id: id.clone(),
            peer: peer.clone(),
            direction: "incoming".into(),
            code: secure_code(stream)?,
            status: "pending".into(),
            error: None,
        })?;
        let result = timeout(self.decision_timeout, async {
            write_message(stream, &Message::PairChallenge { request_id: id.clone(), device: self.local.clone() }).await?;
            let accepted = {
                let remote = read_message(stream);
                tokio::pin!(remote);
                let mut decision = decision;
                tokio::select! {
                    local = &mut decision => {
                        if local.context("配对确认已取消")? {
                            let Message::PairDecision { accepted } = remote.await? else { anyhow::bail!("无效配对确认") };
                            accepted
                        } else { false }
                    }
                    remote = &mut remote => {
                        let Message::PairDecision { accepted } = remote? else { anyhow::bail!("无效配对确认") };
                        if accepted { decision.await.context("配对确认已取消")? } else { false }
                    }
                }
            };
            if accepted { self.store.save_peer(&peer)?; }
            write_message(stream, &Message::PairResult { accepted }).await?;
            Ok(accepted)
        }).await.context("配对已超时，请重新发起").and_then(|result| result);
        self.pairings.finish(&id, &result);
        result.map(|_| ())
    }
}
