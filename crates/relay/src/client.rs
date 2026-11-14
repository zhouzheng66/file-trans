use crate::{
    protocol::{read_json, write_json, ClientMessage, ServerMessage, Target, CONNECT_TIMEOUT},
    socket::{control_stream, tunnel_stream, validate_url},
};
use anyhow::{ensure, Context, Result};
use file_trans_identity::{connect, Identity};
use serde::Serialize;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::{
    io::DuplexStream,
    sync::{mpsc, oneshot},
    time::{interval, sleep, timeout},
};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RelayStatus {
    pub state: String,
    pub error: Option<String>,
    pub invite_code: Option<String>,
    pub invite_expires_at: Option<u64>,
}

impl RelayStatus {
    pub fn inactive(state: &str) -> Self {
        Self {
            state: state.into(),
            error: None,
            invite_code: None,
            invite_expires_at: None,
        }
    }
}

#[derive(Debug)]
pub struct IncomingTunnel {
    pub relay_url: String,
    pub session_id: String,
    pub ticket: String,
    pub peer_id: String,
    pub invite: Option<String>,
}

pub struct OpenedTunnel {
    pub stream: DuplexStream,
    pub peer_id: String,
}

struct Command {
    message: ClientMessage,
    reply: Option<oneshot::Sender<ServerMessage>>,
}

#[derive(Clone)]
pub struct RelayClient {
    url: String,
    status: Arc<Mutex<RelayStatus>>,
    commands: mpsc::Sender<Command>,
    cancel: CancellationToken,
}

impl RelayClient {
    pub fn start(
        url: &str,
        identity: Identity,
        incoming: mpsc::Sender<IncomingTunnel>,
    ) -> Result<Self> {
        let (commands, receiver) = mpsc::channel(32);
        let client = Self {
            url: validate_url(url)?,
            status: Arc::new(Mutex::new(RelayStatus::inactive("connecting"))),
            commands,
            cancel: CancellationToken::new(),
        };
        let actor = client.clone();
        tokio::spawn(async move {
            actor.run(identity, incoming, receiver).await;
        });
        Ok(client)
    }

    pub fn url(&self) -> &str {
        &self.url
    }
    pub fn snapshot(&self) -> RelayStatus {
        self.status.lock().unwrap().clone()
    }
    pub fn stop(&self) {
        self.cancel.cancel();
    }

    pub fn valid_invite(&self, code: &str) -> bool {
        let status = self.status.lock().unwrap();
        status.invite_code.as_deref() == Some(code)
            && status
                .invite_expires_at
                .is_some_and(|expiry| expiry > crate::protocol::now_ms())
    }

    async fn request(&self, message: ClientMessage) -> Result<ServerMessage> {
        ensure!(
            self.snapshot().state == "online",
            "中继尚未连接，请检查地址和网络后重试"
        );
        let (tx, rx) = oneshot::channel();
        timeout(CONNECT_TIMEOUT, async {
            self.commands
                .send(Command {
                    message,
                    reply: Some(tx),
                })
                .await
                .context("中继连接已关闭")?;
            let response = rx.await.context("中继控制连接已断开")?;
            if let ServerMessage::Error { message, .. } = &response {
                anyhow::bail!("{message}");
            }
            Ok(response)
        })
        .await
        .context("中继请求超时")?
    }

    pub async fn create_invite(&self) -> Result<()> {
        let response = self
            .request(ClientMessage::Invite {
                request_id: Uuid::new_v4().to_string(),
            })
            .await?;
        ensure!(
            matches!(response, ServerMessage::Invite { .. }),
            "无效的中继邀请响应"
        );
        Ok(())
    }

    pub async fn open(&self, target: Target) -> Result<OpenedTunnel> {
        let response = self
            .request(ClientMessage::Open {
                request_id: Uuid::new_v4().to_string(),
                target,
            })
            .await?;
        let ServerMessage::Opened {
            session_id,
            ticket,
            peer_id,
            ..
        } = response
        else {
            anyhow::bail!("无效的中继连接响应")
        };
        Ok(OpenedTunnel {
            stream: tunnel_stream(&self.url, session_id, ticket).await?,
            peer_id,
        })
    }

    pub async fn attach(&self, incoming: &IncomingTunnel) -> Result<DuplexStream> {
        tunnel_stream(
            &self.url,
            incoming.session_id.clone(),
            incoming.ticket.clone(),
        )
        .await
    }

    pub fn reject(&self, session_id: String) {
        let _ = self.commands.try_send(Command {
            message: ClientMessage::Reject { session_id },
            reply: None,
        });
    }

    async fn run(
        &self,
        identity: Identity,
        incoming: mpsc::Sender<IncomingTunnel>,
        mut commands: mpsc::Receiver<Command>,
    ) {
        let mut delay = 1;
        loop {
            *self.status.lock().unwrap() = RelayStatus::inactive("connecting");
            let result = tokio::select! {
                _ = self.cancel.cancelled() => break,
                result = self.session(&identity, &incoming, &mut commands) => result,
            };
            let was_online = self.snapshot().state == "online";
            let mut status = RelayStatus::inactive("error");
            status.error = Some(
                result
                    .err()
                    .map(|error| format!("{error:#}"))
                    .unwrap_or_else(|| "中继控制连接已关闭".into()),
            );
            *self.status.lock().unwrap() = status;
            while commands.try_recv().is_ok() {}
            tokio::select! { _ = self.cancel.cancelled() => break, _ = sleep(Duration::from_secs(delay)) => {} }
            delay = if was_online { 1 } else { (delay * 2).min(10) };
        }
        *self.status.lock().unwrap() = RelayStatus::inactive("disabled");
    }

    async fn session(
        &self,
        identity: &Identity,
        incoming: &mpsc::Sender<IncomingTunnel>,
        commands: &mut mpsc::Receiver<Command>,
    ) -> Result<()> {
        let mut stream = timeout(CONNECT_TIMEOUT, async {
            connect(control_stream(&self.url).await?, identity, None).await
        })
        .await
        .context("中继设备认证超时")??;
        ensure!(
            matches!(timeout(CONNECT_TIMEOUT, read_json::<ServerMessage>(&mut stream)).await??, ServerMessage::Registered { device_id, version } if device_id == identity.id && version == crate::protocol::VERSION),
            "中继注册身份或协议版本不匹配"
        );
        *self.status.lock().unwrap() = RelayStatus::inactive("online");
        let (mut reader, mut writer) = tokio::io::split(stream);
        let (events_tx, mut events) = mpsc::channel(32);
        let reader_task = tokio_util::task::AbortOnDropHandle::new(tokio::spawn(async move {
            loop {
                let message = read_json::<ServerMessage>(&mut reader).await;
                let failed = message.is_err();
                if events_tx.send(message).await.is_err() || failed {
                    break;
                }
            }
        }));
        let mut pending = HashMap::new();
        let mut heartbeat = interval(Duration::from_secs(15));
        let mut last_seen = Instant::now();
        let result = async {
            loop {
                tokio::select! {
                    command = commands.recv() => {
                        let command = command.context("中继客户端已关闭")?;
                        let id = match &command.message { ClientMessage::Invite { request_id } | ClientMessage::Open { request_id, .. } => Some(request_id.clone()), _ => None };
                        if let (Some(id), Some(reply)) = (id, command.reply) {
                            pending.retain(|_, reply: &mut oneshot::Sender<ServerMessage>| !reply.is_closed());
                            ensure!(pending.len() < 32, "中继待处理请求过多");
                            pending.insert(id, reply);
                        }
                        timeout(CONNECT_TIMEOUT, write_json(&mut writer, &command.message)).await??;
                    }
                    event = events.recv() => {
                        let event = event.context("中继连接已关闭")??;
                        last_seen = Instant::now();
                        if let ServerMessage::Incoming { session_id, ticket, peer_id, invite } = event {
                            let notification = IncomingTunnel { relay_url: self.url.clone(), session_id: session_id.clone(), ticket, peer_id, invite };
                            if incoming.try_send(notification).is_err() { timeout(CONNECT_TIMEOUT, write_json(&mut writer, &ClientMessage::Reject { session_id })).await??; }
                            continue;
                        }
                        if let ServerMessage::Invite { code, expires_at, .. } = &event {
                            let mut status = self.status.lock().unwrap();
                            status.invite_code = Some(code.clone());
                            status.invite_expires_at = Some(*expires_at);
                        }
                        if let Some(reply) = event.request_id().and_then(|id| pending.remove(id)) { let _ = reply.send(event); }
                    }
                    _ = heartbeat.tick() => {
                        ensure!(last_seen.elapsed() < Duration::from_secs(45), "中继心跳超时");
                        timeout(CONNECT_TIMEOUT, write_json(&mut writer, &ClientMessage::Heartbeat)).await??;
                    }
                }
            }
        }.await;
        reader_task.abort();
        result
    }
}
