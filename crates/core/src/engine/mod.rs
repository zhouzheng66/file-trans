mod pairing;
mod server;
mod transfer;
mod transport;

use crate::{
    decisions::{Pairings, Receipts},
    model::{Activity, Snapshot},
    storage::Store,
};
use anyhow::{ensure, Context, Result};
use file_trans_discovery::Discovery;
use file_trans_identity::Identity;
use file_trans_protocol::Device;
use std::{
    collections::HashMap,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{net::TcpListener, sync::Semaphore, task::JoinHandle};
use tokio_util::sync::CancellationToken;

pub(crate) const IO_TIMEOUT: Duration = Duration::from_secs(30);

pub struct EngineConfig {
    pub data_dir: PathBuf,
    pub download_dir: PathBuf,
    pub name: String,
    pub listen_address: SocketAddr,
    pub enable_discovery: bool,
    pub decision_timeout: Duration,
}

impl EngineConfig {
    pub fn new(data_dir: PathBuf, download_dir: PathBuf, name: String) -> Self {
        Self {
            data_dir,
            download_dir,
            name,
            listen_address: SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 0),
            enable_discovery: true,
            decision_timeout: Duration::from_secs(120),
        }
    }
}

pub struct Engine {
    pub(crate) identity: Identity,
    pub(crate) local: Device,
    pub(crate) address: SocketAddr,
    pub(crate) store: Store,
    pub(crate) download_dir: Mutex<PathBuf>,
    pub(crate) pairings: Pairings,
    pub(crate) receipts: Receipts,
    pub(crate) live: Mutex<HashMap<String, Activity>>,
    pub(crate) cancel: CancellationToken,
    pub(crate) decision_timeout: Duration,
    outbound: Arc<Semaphore>,
    discovery: Mutex<Option<Discovery>>,
    discovery_error: Option<String>,
    server_task: Mutex<Option<JoinHandle<()>>>,
}

impl Engine {
    pub async fn start(config: EngineConfig, identity: Identity) -> Result<Arc<Self>> {
        validate_name(&config.name)?;
        tokio::fs::create_dir_all(&config.data_dir).await?;
        let store = Store::open(&config.data_dir.join("file-trans.db"))?;
        let download_dir = store
            .setting("download_dir")?
            .map(PathBuf::from)
            .unwrap_or(config.download_dir);
        tokio::fs::create_dir_all(&download_dir)
            .await
            .context("创建接收目录失败")?;
        let listener = TcpListener::bind(config.listen_address)
            .await
            .context("启动局域网监听失败")?;
        let address = listener.local_addr()?;
        let local = Device {
            id: identity.id.clone(),
            name: config.name,
            port: address.port(),
        };
        let (discovery, discovery_error) = if config.enable_discovery {
            match Discovery::start(&local.id, &local.name, local.port) {
                Ok(discovery) => (Some(discovery), None),
                Err(error) => (
                    None,
                    Some(format!("自动发现不可用，可使用手动地址连接：{error:#}")),
                ),
            }
        } else {
            (None, None)
        };
        let tls = file_trans_identity::server_config(&identity)?;
        let engine = Arc::new(Self {
            identity,
            local,
            address,
            store,
            download_dir: Mutex::new(download_dir),
            pairings: Pairings::default(),
            receipts: Receipts::default(),
            live: Mutex::new(HashMap::new()),
            cancel: CancellationToken::new(),
            decision_timeout: config.decision_timeout,
            outbound: Arc::new(Semaphore::new(4)),
            discovery: Mutex::new(discovery),
            discovery_error,
            server_task: Mutex::new(None),
        });
        let task = tokio::spawn(server::listen(
            Arc::clone(&engine),
            listener,
            file_trans_identity::TlsAcceptor::from(tls),
        ));
        *engine.server_task.lock().unwrap() = Some(task);
        Ok(engine)
    }

    pub fn local(&self) -> &Device {
        &self.local
    }

    pub fn local_address(&self) -> SocketAddr {
        if self.address.ip().is_unspecified() {
            SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), self.address.port())
        } else {
            self.address
        }
    }

    pub fn snapshot(&self) -> Result<Snapshot> {
        let mut activities = self.store.activities()?;
        for live in self.live.lock().unwrap().values() {
            if let Some(entry) = activities
                .iter_mut()
                .find(|entry| activity_key(entry) == activity_key(live))
            {
                *entry = live.clone();
            } else {
                activities.push(live.clone());
            }
        }
        activities.sort_by_key(|activity| std::cmp::Reverse(activity.timestamp));
        let mut completed = 0;
        activities.retain(|activity| {
            if ["completed", "failed", "rejected"].contains(&activity.status.as_str()) {
                completed += 1;
                completed <= 200
            } else {
                true
            }
        });
        let local_addresses = if_addrs::get_if_addrs()
            .unwrap_or_default()
            .into_iter()
            .filter(|interface| !interface.is_loopback() && interface.ip().is_ipv4())
            .map(|interface| SocketAddr::new(interface.ip(), self.address.port()).to_string())
            .collect();
        Ok(Snapshot {
            local: self.local.clone(),
            listen_address: self.address.to_string(),
            local_addresses,
            download_dir: self
                .download_dir
                .lock()
                .unwrap()
                .to_string_lossy()
                .into_owned(),
            discovered: self.discovered(),
            discovery_error: self.discovery_error.clone(),
            peers: self.store.peers()?,
            pairings: self.pairings.snapshot(),
            activities,
        })
    }

    fn discovered(&self) -> Vec<file_trans_discovery::DiscoveredDevice> {
        self.discovery
            .lock()
            .unwrap()
            .as_ref()
            .map(Discovery::devices)
            .unwrap_or_default()
    }

    pub fn decide_pairing(&self, id: &str, accepted: bool) -> Result<()> {
        self.pairings.decide(id, accepted)
    }
    pub fn decide_file(&self, id: &str, accepted: bool) -> Result<()> {
        self.receipts.decide(id, accepted)
    }
    pub fn forget_peer(&self, id: &str) -> Result<()> {
        self.store.forget_peer(id)
    }

    pub async fn set_download_dir(&self, path: PathBuf) -> Result<()> {
        ensure!(path.is_absolute(), "接收目录必须为绝对路径");
        tokio::fs::create_dir_all(&path).await?;
        self.store
            .set_setting("download_dir", &path.to_string_lossy())?;
        *self.download_dir.lock().unwrap() = path;
        Ok(())
    }

    pub async fn shutdown(&self) {
        self.cancel.cancel();
        self.discovery.lock().unwrap().take();
        let task = self.server_task.lock().unwrap().take();
        if let Some(task) = task {
            let _ = task.await;
        }
    }

    pub(crate) fn record(&self, activity: &Activity) -> Result<()> {
        self.store.save_activity(activity)?;
        self.progress(activity);
        Ok(())
    }

    pub(crate) fn progress(&self, activity: &Activity) {
        let mut live = self.live.lock().unwrap();
        if live.len() >= 256 {
            let oldest = live
                .iter()
                .filter(|(_, activity)| {
                    ["completed", "failed", "rejected"].contains(&activity.status.as_str())
                })
                .min_by_key(|(_, activity)| activity.timestamp)
                .map(|(key, _)| key.clone());
            if let Some(oldest) = oldest {
                live.remove(&oldest);
            }
        }
        live.insert(activity_key(activity), activity.clone());
    }

    pub(crate) fn failed(&self, activity: &mut Activity, error: &anyhow::Error) {
        activity.status = "failed".into();
        activity.error = Some(format!("{error:#}"));
        if let Err(error) = self.record(activity) {
            tracing::error!("任务状态保存失败：{error:#}");
        }
    }
}

pub(crate) fn validate_name(name: &str) -> Result<()> {
    ensure!(
        !name.trim().is_empty() && name.len() <= 180 && !name.chars().any(char::is_control),
        "设备名称无效"
    );
    Ok(())
}

pub(crate) fn validate_id(id: &str) -> Result<()> {
    uuid::Uuid::parse_str(id).context("消息 ID 无效")?;
    Ok(())
}

fn activity_key(activity: &Activity) -> String {
    format!(
        "{}:{}:{}",
        activity.direction, activity.peer_id, activity.id
    )
}
