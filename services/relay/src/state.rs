use crate::RelayLimits;
use anyhow::{ensure, Result};
use axum::extract::ws::WebSocket;
use file_trans_identity::{server_config, Identity, TlsAcceptor};
use file_trans_relay::protocol::{now_ms, ClientMessage, ServerMessage, Target};
use std::{
    collections::HashMap,
    net::IpAddr,
    sync::{Arc, Mutex, Weak},
    time::{Duration, Instant},
};
use tokio::sync::{mpsc, OwnedSemaphorePermit, Semaphore};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

pub(crate) struct RelayState {
    pub limits: RelayLimits,
    pub acceptor: TlsAcceptor,
    pub cancel: CancellationToken,
    tables: Mutex<Tables>,
    connections: Mutex<HashMap<IpAddr, IpUsage>>,
    slots: Arc<Semaphore>,
}

#[derive(Default)]
struct Tables {
    controls: HashMap<String, (String, mpsc::Sender<ServerMessage>)>,
    invites: HashMap<String, (String, u64)>,
    sessions: HashMap<String, Session>,
    usage: HashMap<String, usize>,
}

struct IpUsage {
    count: usize,
    attempts: u32,
    window: Instant,
}

pub(crate) struct ConnectionGuard {
    state: Weak<RelayState>,
    ip: IpAddr,
}
impl Drop for ConnectionGuard {
    fn drop(&mut self) {
        if let Some(state) = self.state.upgrade() {
            if let Some(usage) = state.connections.lock().unwrap().get_mut(&self.ip) {
                usage.count = usage.count.saturating_sub(1);
            }
        }
    }
}

pub(crate) struct Leg {
    pub socket: WebSocket,
    pub _guard: ConnectionGuard,
}
pub(crate) struct Session {
    pub source: String,
    pub target: String,
    a_ticket: String,
    b_ticket: String,
    deadline: Instant,
    first: Option<(bool, Leg)>,
    pub _permit: OwnedSemaphorePermit,
}

impl RelayState {
    pub fn new(limits: RelayLimits, identity: Identity, cancel: CancellationToken) -> Result<Self> {
        ensure!(
            limits.max_devices > 0
                && limits.max_devices <= 4096
                && limits.max_sessions > 0
                && limits.max_sessions <= 4096
                && limits.max_connections_per_ip > 0
                && limits.max_connections_per_ip <= 4096
                && limits.max_session_bytes > 0
                && !limits.max_session_duration.is_zero(),
            "中继资源上限无效"
        );
        Ok(Self {
            slots: Arc::new(Semaphore::new(limits.max_sessions)),
            limits,
            acceptor: TlsAcceptor::from(server_config(&identity)?),
            cancel,
            tables: Mutex::new(Tables::default()),
            connections: Mutex::new(HashMap::new()),
        })
    }

    pub fn connection(self: &Arc<Self>, ip: IpAddr) -> Option<ConnectionGuard> {
        let mut records = self.connections.lock().unwrap();
        records
            .retain(|_, usage| usage.count > 0 || usage.window.elapsed() < Duration::from_secs(60));
        if records.len() >= 4096 && !records.contains_key(&ip) {
            return None;
        }
        if records.values().map(|usage| usage.count).sum::<usize>()
            >= self.limits.max_devices * 4 + self.limits.max_sessions * 2
        {
            return None;
        }
        let usage = records.entry(ip).or_insert(IpUsage {
            count: 0,
            attempts: 0,
            window: Instant::now(),
        });
        if usage.window.elapsed() > Duration::from_secs(60) {
            usage.window = Instant::now();
            usage.attempts = 0;
        }
        if usage.count >= self.limits.max_connections_per_ip || usage.attempts >= 240 {
            return None;
        }
        usage.count += 1;
        usage.attempts += 1;
        Some(ConnectionGuard {
            state: Arc::downgrade(self),
            ip,
        })
    }

    pub fn register(
        &self,
        id: &str,
        generation: String,
        sender: mpsc::Sender<ServerMessage>,
    ) -> Result<()> {
        let mut tables = self.tables.lock().unwrap();
        ensure!(
            tables.controls.len() < self.limits.max_devices && !tables.controls.contains_key(id),
            "设备已在线或服务已达到设备上限"
        );
        tables.controls.insert(id.to_owned(), (generation, sender));
        Ok(())
    }

    pub fn disconnect(&self, id: &str, generation: &str) {
        let mut tables = self.tables.lock().unwrap();
        if !tables
            .controls
            .get(id)
            .is_some_and(|(current, _)| current == generation)
        {
            return;
        }
        tables.controls.remove(id);
        tables.invites.retain(|_, (owner, _)| owner != id);
        let sessions: Vec<_> = tables
            .sessions
            .iter()
            .filter(|(_, session)| session.source == id || session.target == id)
            .map(|(id, _)| id.clone())
            .collect();
        for id in sessions {
            remove_session(&mut tables, &id);
        }
    }

    pub fn dispatch(&self, device: &str, message: ClientMessage) -> Result<Option<ServerMessage>> {
        let mut tables = self.tables.lock().unwrap();
        match message {
            ClientMessage::Heartbeat => Ok(Some(ServerMessage::Heartbeat)),
            ClientMessage::Invite { request_id } => {
                Uuid::parse_str(&request_id)?;
                tables
                    .invites
                    .retain(|_, (owner, expiry)| owner != device && *expiry > now_ms());
                let code = Uuid::new_v4().simple().to_string()[..12].to_ascii_uppercase();
                let expires_at = now_ms() + 300_000;
                tables
                    .invites
                    .insert(code.clone(), (device.to_owned(), expires_at));
                Ok(Some(ServerMessage::Invite {
                    request_id,
                    code,
                    expires_at,
                }))
            }
            ClientMessage::Open { request_id, target } => {
                Uuid::parse_str(&request_id)?;
                let (target, invite) = match target {
                    Target::Peer(id) => (id, None),
                    Target::Invite(code) => {
                        let (id, expiry) = tables
                            .invites
                            .get(&code)
                            .ok_or_else(|| anyhow::anyhow!("配对码无效或已过期"))?;
                        ensure!(*expiry > now_ms(), "配对码已过期");
                        (id.clone(), Some(code))
                    }
                };
                ensure!(target != device, "不能连接本机");
                let target_sender = tables
                    .controls
                    .get(&target)
                    .map(|(_, sender)| sender.clone())
                    .ok_or_else(|| anyhow::anyhow!("对方未连接同一中继，可能已离线"))?;
                ensure!(
                    *tables.usage.get(device).unwrap_or(&0) < 4
                        && *tables.usage.get(&target).unwrap_or(&0) < 4,
                    "设备的中继任务数量已达上限"
                );
                let permit = self
                    .slots
                    .clone()
                    .try_acquire_owned()
                    .map_err(|_| anyhow::anyhow!("中继服务繁忙，请稍后重试"))?;
                let session_id = Uuid::new_v4().to_string();
                let a_ticket = token();
                let b_ticket = token();
                target_sender
                    .try_send(ServerMessage::Incoming {
                        session_id: session_id.clone(),
                        ticket: b_ticket.clone(),
                        peer_id: device.into(),
                        invite,
                    })
                    .map_err(|_| anyhow::anyhow!("对端控制连接繁忙或已断开"))?;
                *tables.usage.entry(device.into()).or_default() += 1;
                *tables.usage.entry(target.clone()).or_default() += 1;
                tables.sessions.insert(
                    session_id.clone(),
                    Session {
                        source: device.into(),
                        target: target.clone(),
                        a_ticket: a_ticket.clone(),
                        b_ticket,
                        deadline: Instant::now() + Duration::from_secs(20),
                        first: None,
                        _permit: permit,
                    },
                );
                Ok(Some(ServerMessage::Opened {
                    request_id,
                    session_id,
                    ticket: a_ticket,
                    peer_id: target,
                }))
            }
            ClientMessage::Reject { session_id } => {
                if tables
                    .sessions
                    .get(&session_id)
                    .is_some_and(|session| session.source == device || session.target == device)
                {
                    remove_session(&mut tables, &session_id);
                }
                Ok(None)
            }
        }
    }

    pub fn attach(&self, id: &str, ticket: &str, leg: Leg) -> Result<Option<(Session, Leg, Leg)>> {
        let mut tables = self.tables.lock().unwrap();
        let session = tables
            .sessions
            .get_mut(id)
            .ok_or_else(|| anyhow::anyhow!("中继会话不存在或已失效"))?;
        ensure!(session.deadline > Instant::now(), "隧道接入已超时");
        let side = if ticket == session.a_ticket {
            true
        } else {
            ensure!(ticket == session.b_ticket, "隧道接入票据无效");
            false
        };
        if let Some((first_side, _)) = &session.first {
            ensure!(*first_side != side, "隧道票据已使用");
        }
        if session.first.is_none() {
            session.first = Some((side, leg));
            return Ok(None);
        }
        let mut session = tables.sessions.remove(id).unwrap();
        let (_, first) = session.first.take().unwrap();
        Ok(Some((session, first, leg)))
    }

    pub fn finish(&self, session: &Session) {
        let mut tables = self.tables.lock().unwrap();
        release(&mut tables, &session.source);
        release(&mut tables, &session.target);
    }
    pub fn prune(&self) {
        let mut tables = self.tables.lock().unwrap();
        tables.invites.retain(|_, (_, expiry)| *expiry > now_ms());
        let expired: Vec<_> = tables
            .sessions
            .iter()
            .filter(|(_, session)| session.deadline <= Instant::now())
            .map(|(id, _)| id.clone())
            .collect();
        for id in expired {
            remove_session(&mut tables, &id);
        }
    }
    pub fn clear(&self) {
        let mut tables = self.tables.lock().unwrap();
        tables.sessions.clear();
        tables.controls.clear();
        tables.invites.clear();
    }
}

fn token() -> String {
    format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple())
}
fn release(tables: &mut Tables, id: &str) {
    if let Some(count) = tables.usage.get_mut(id) {
        *count = count.saturating_sub(1);
        if *count == 0 {
            tables.usage.remove(id);
        }
    }
}
fn remove_session(tables: &mut Tables, id: &str) {
    if let Some(session) = tables.sessions.remove(id) {
        release(tables, &session.source);
        release(tables, &session.target);
    }
}

#[cfg(test)]
#[path = "state_tests.rs"]
mod tests;
