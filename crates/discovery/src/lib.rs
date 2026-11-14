use anyhow::{ensure, Result};
use mdns_sd::{ResolvedService, ServiceDaemon, ServiceEvent, ServiceInfo};
use serde::Serialize;
use std::{
    net::SocketAddr,
    time::{Duration, Instant},
};
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(test)]
mod tests;

const SERVICE: &str = "_filetrans._tcp.local.";

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveredDevice {
    pub id: String,
    pub name: String,
    pub addresses: Vec<String>,
}

pub struct Discovery {
    daemon: ServiceDaemon,
    fullname: String,
    devices: watch::Sender<Vec<DiscoveredDevice>>,
    refresh: watch::Sender<u64>,
    cancel: CancellationToken,
}

impl Discovery {
    pub fn start(id: &str, name: &str, port: u16) -> Result<Self> {
        ensure!(valid_id(id), "无效的设备标识");
        let daemon = ServiceDaemon::new()?;
        let service = ServiceInfo::new(
            SERVICE,
            &id[..24],
            &format!("file-trans-{}.local.", &id[..24]),
            "",
            port,
            [("id", id), ("name", name), ("version", "1")].as_slice(),
        )?
        .enable_addr_auto();
        let fullname = service.get_fullname().to_owned();
        daemon.register(service)?;
        let mut receiver = daemon.browse(SERVICE)?;
        let devices = watch::channel(Vec::new()).0;
        let shared = devices.clone();
        let local_id = id.to_owned();
        let cancel = CancellationToken::new();
        let token = cancel.clone();
        let (refresh, mut requested) = watch::channel(0_u64);
        let browser = daemon.clone();
        tokio::spawn(async move {
            // 重新发出 PTR 查询，覆盖休眠、断网和错过设备启动广播的情况。
            // 不 stop_browse：保留已有缓存，避免刷新时设备列表闪烁。
            let mut interval = tokio::time::interval(Duration::from_secs(10));
            interval.tick().await;
            let mut last_browse = Instant::now() - Duration::from_secs(1);
            loop {
                let event = tokio::select! {
                    _ = token.cancelled() => break,
                    _ = interval.tick() => None,
                    result = requested.changed() => {
                        if result.is_err() { break; }
                        None
                    },
                    event = receiver.recv_async() => match event {
                        Ok(value) => Some(value), Err(_) => break,
                    },
                };
                let Some(event) = event else {
                    if last_browse.elapsed() >= Duration::from_secs(1) {
                        if let Ok(next) = browser.browse(SERVICE) {
                            receiver = next;
                            last_browse = Instant::now();
                        }
                    }
                    continue;
                };
                match event {
                    ServiceEvent::ServiceResolved(info) => {
                        let Some(device) = device_from_info(&info, &local_id) else {
                            continue;
                        };
                        publish(&shared, device);
                    }
                    ServiceEvent::ServiceRemoved(_, fullname) => {
                        shared.send_if_modified(|devices| {
                            let before = devices.len();
                            devices.retain(|device| service_name(&device.id) != fullname);
                            devices.len() != before
                        });
                    }
                    _ => {}
                }
            }
        });
        Ok(Self {
            daemon,
            fullname,
            devices,
            refresh,
            cancel,
        })
    }

    pub fn devices(&self) -> Vec<DiscoveredDevice> {
        self.devices.borrow().clone()
    }

    pub fn subscribe(&self) -> watch::Receiver<Vec<DiscoveredDevice>> {
        self.devices.subscribe()
    }

    /// 主动确认已配对设备的 SRV/地址，不依赖启动时的一次广播。
    pub async fn resolve_peer(&self, id: &str) -> Result<()> {
        ensure!(valid_id(id), "无效的设备标识");
        self.refresh
            .send_modify(|revision| *revision = revision.wrapping_add(1));
        self.daemon
            .verify(service_name(id), Duration::from_secs(10))?;
        // macOS 使用系统 Bonjour 再查询一次，覆盖原始组播套接字发现不可用的情况。
        #[cfg(target_os = "macos")]
        publish(&self.devices, macos::resolve_peer(id).await?);
        Ok(())
    }
}

fn valid_id(id: &str) -> bool {
    id.len() == 64 && id.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn service_name(id: &str) -> String {
    format!("{}.{SERVICE}", &id[..24])
}

fn publish(devices: &watch::Sender<Vec<DiscoveredDevice>>, device: DiscoveredDevice) {
    devices.send_if_modified(|devices| {
        if let Some(previous) = devices.iter_mut().find(|entry| entry.id == device.id) {
            if *previous == device {
                return false;
            }
            *previous = device;
        } else if devices.len() < 256 {
            devices.push(device);
        } else {
            return false;
        }
        devices.sort_by(|a, b| a.name.cmp(&b.name));
        true
    });
}

fn device_from_info(info: &ResolvedService, local_id: &str) -> Option<DiscoveredDevice> {
    let id = info.get_property_val_str("id")?;
    if id == local_id || !valid_id(id) || info.get_property_val_str("version") != Some("1") {
        return None;
    }
    let mut addresses: Vec<_> = info
        .get_addresses()
        .iter()
        .filter(|ip| ip.is_ipv4())
        .map(|ip| SocketAddr::new(ip.to_ip_addr(), info.get_port()).to_string())
        .collect();
    addresses.sort();
    if addresses.is_empty() || info.get_port() == 0 {
        return None;
    }
    Some(DiscoveredDevice {
        id: id.into(),
        name: clean_name(info.get_property_val_str("name").unwrap_or("未命名设备")),
        addresses,
    })
}

fn clean_name(name: &str) -> String {
    name.chars()
        .filter(|character| !character.is_control())
        .take(60)
        .collect()
}

impl Drop for Discovery {
    fn drop(&mut self) {
        self.cancel.cancel();
        let _ = self.daemon.unregister(&self.fullname);
        let _ = self.daemon.shutdown();
    }
}
