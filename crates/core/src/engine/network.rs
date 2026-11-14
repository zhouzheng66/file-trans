use super::{server, Engine, IO_TIMEOUT};
use crate::model::{NetworkMode, NetworkSettings, Route};
use anyhow::{ensure, Result};
use file_trans_identity::{peer_id, BoxedStream, TlsAcceptor, TlsStream};
use file_trans_relay::{validate_url, IncomingTunnel, RelayClient, RelayStatus};
use std::sync::Arc;
use tokio::{sync::mpsc, task::JoinSet, time::timeout};

pub(super) fn validate_settings(mut settings: NetworkSettings) -> Result<NetworkSettings> {
    settings.relay_url = if settings.relay_url.trim().is_empty() {
        String::new()
    } else {
        validate_url(&settings.relay_url)?
    };
    Ok(settings)
}

impl Engine {
    pub fn network_settings(&self) -> NetworkSettings {
        self.network.lock().unwrap().clone()
    }

    pub async fn set_network_settings(&self, settings: NetworkSettings) -> Result<()> {
        let _update = self.network_updates.lock().await;
        let settings = validate_settings(settings)?;
        if self.network_settings() == settings {
            return Ok(());
        }
        self.store
            .set_setting("network", &serde_json::to_string(&settings)?)?;
        *self.network.lock().unwrap() = settings;
        self.configure_relay()?;
        self.route_revision
            .send_modify(|value| *value = value.wrapping_add(1));
        Ok(())
    }

    pub(super) fn configure_relay(&self) -> Result<()> {
        let settings = self.network_settings();
        let mut current = self.relay.lock().unwrap();
        if settings.mode == NetworkMode::Lan || settings.relay_url.is_empty() {
            if let Some(client) = current.take() {
                client.stop();
            }
        } else if current
            .as_ref()
            .is_none_or(|client| client.url() != settings.relay_url)
        {
            if let Some(client) = current.take() {
                client.stop();
            }
            *current = Some(RelayClient::start(
                &settings.relay_url,
                self.identity.clone(),
                self.relay_events.clone(),
            )?);
        }
        Ok(())
    }

    pub(super) fn relay_client(&self) -> Result<RelayClient> {
        ensure!(
            self.network_settings().mode != NetworkMode::Lan,
            "当前为仅局域网模式"
        );
        self.relay
            .lock()
            .unwrap()
            .clone()
            .ok_or_else(|| anyhow::anyhow!("尚未配置中继地址，请在两端配置同一个 wss:// 中继"))
    }

    pub fn relay_status(&self) -> RelayStatus {
        if self.network_settings().mode == NetworkMode::Lan {
            RelayStatus::inactive("disabled")
        } else {
            self.relay
                .lock()
                .unwrap()
                .as_ref()
                .map(RelayClient::snapshot)
                .unwrap_or_else(|| RelayStatus::inactive("unconfigured"))
        }
    }

    pub async fn create_invite(&self) -> Result<()> {
        self.relay_client()?.create_invite().await
    }
}

pub(super) async fn receive(engine: Arc<Engine>, mut receiver: mpsc::Receiver<IncomingTunnel>) {
    let acceptor = TlsAcceptor::from(
        file_trans_identity::server_config(&engine.identity).expect("已验证设备证书"),
    );
    let mut tasks = JoinSet::new();
    loop {
        tokio::select! {
            _ = engine.cancel.cancelled() => break,
            Some(_) = tasks.join_next(), if !tasks.is_empty() => {},
            incoming = receiver.recv() => {
                let Some(incoming) = incoming else { break };
                let Ok(client) = engine.relay_client() else { continue };
                if client.url() != incoming.relay_url { continue; }
                let authorized = match &incoming.invite {
                    Some(code) => client.valid_invite(code),
                    None => engine.store.peer(&incoming.peer_id).is_ok(),
                };
                if !authorized || tasks.len() >= 8 { client.reject(incoming.session_id); continue; }
                let engine = Arc::clone(&engine);
                let acceptor = acceptor.clone();
                tasks.spawn(async move {
                    let result = tokio::select! { _ = engine.cancel.cancelled() => return, result = timeout(IO_TIMEOUT, async {
                        let base = client.attach(&incoming).await?;
                        let stream = TlsStream::Server(acceptor.accept(Box::new(base) as BoxedStream).await?);
                        ensure!(peer_id(&stream)? == incoming.peer_id, "中继接入设备身份不匹配");
                        Ok::<_, anyhow::Error>(stream)
                    }) => result };
                    if let Ok(Ok(stream)) = result { server::serve_stream(&engine, stream, None, Route::Relay).await; }
                });
            }
        }
    }
    while tasks.join_next().await.is_some() {}
}
