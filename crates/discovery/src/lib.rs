use anyhow::Result;
use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};
use serde::Serialize;
use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::{Arc, Mutex},
};
use tokio_util::sync::CancellationToken;

const SERVICE: &str = "_filetrans._tcp.local.";

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveredDevice {
    pub id: String,
    pub name: String,
    pub addresses: Vec<String>,
}

pub struct Discovery {
    daemon: ServiceDaemon,
    fullname: String,
    devices: Arc<Mutex<HashMap<String, (String, DiscoveredDevice)>>>,
    cancel: CancellationToken,
}

impl Discovery {
    pub fn start(id: &str, name: &str, port: u16) -> Result<Self> {
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
        let receiver = daemon.browse(SERVICE)?;
        let devices = Arc::new(Mutex::new(HashMap::new()));
        let shared = Arc::clone(&devices);
        let local_id = id.to_owned();
        let cancel = CancellationToken::new();
        let token = cancel.clone();
        tokio::spawn(async move {
            loop {
                let event = tokio::select! {
                    _ = token.cancelled() => break,
                    event = receiver.recv_async() => match event { Ok(value) => value, Err(_) => break },
                };
                match event {
                    ServiceEvent::ServiceResolved(info) => {
                        let Some(id) = info.get_property_val_str("id") else {
                            continue;
                        };
                        if id == local_id
                            || id.len() != 64
                            || !id.bytes().all(|b| b.is_ascii_hexdigit())
                            || info.get_property_val_str("version") != Some("1")
                        {
                            continue;
                        }
                        let name: String = info
                            .get_property_val_str("name")
                            .unwrap_or("未命名设备")
                            .chars()
                            .filter(|c| !c.is_control())
                            .take(60)
                            .collect();
                        let addresses = info
                            .get_addresses()
                            .iter()
                            .filter(|ip| ip.is_ipv4())
                            .map(|ip| SocketAddr::new(ip.to_ip_addr(), info.get_port()).to_string())
                            .collect::<Vec<_>>();
                        if addresses.is_empty() {
                            continue;
                        }
                        let mut entries = shared.lock().unwrap();
                        if entries.len() < 256 || entries.contains_key(id) {
                            entries.insert(
                                id.to_owned(),
                                (
                                    info.get_fullname().to_owned(),
                                    DiscoveredDevice {
                                        id: id.to_owned(),
                                        name,
                                        addresses,
                                    },
                                ),
                            );
                        }
                    }
                    ServiceEvent::ServiceRemoved(_, fullname) => {
                        shared
                            .lock()
                            .unwrap()
                            .retain(|_, (service, _)| *service != fullname);
                    }
                    _ => {}
                }
            }
        });
        Ok(Self {
            daemon,
            fullname,
            devices,
            cancel,
        })
    }

    pub fn devices(&self) -> Vec<DiscoveredDevice> {
        let mut devices: Vec<_> = self
            .devices
            .lock()
            .unwrap()
            .values()
            .map(|(_, device)| device.clone())
            .collect();
        devices.sort_by(|a, b| a.name.cmp(&b.name));
        devices
    }
}

impl Drop for Discovery {
    fn drop(&mut self) {
        self.cancel.cancel();
        let _ = self.daemon.unregister(&self.fullname);
        let _ = self.daemon.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    #[tokio::test]
    #[ignore = "需要本地网络组播权限，单独运行"]
    async fn publishes_discovers_and_removes_a_real_mdns_service() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let a_id = format!("a{nonce:063x}");
        let b_id = format!("b{nonce:063x}");
        let a = Discovery::start(&a_id, "发现测试 A", 43210).unwrap();
        let b = Discovery::start(&b_id, "发现测试 B", 43211).unwrap();
        tokio::time::timeout(Duration::from_secs(20), async {
            loop {
                if a.devices().iter().any(|device| device.id == b_id)
                    && b.devices().iter().any(|device| device.id == a_id)
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        })
        .await
        .expect("两节点没有通过组播发现对方");
        let peer = a
            .devices()
            .into_iter()
            .find(|device| device.id == b_id)
            .unwrap();
        assert_eq!(peer.name, "发现测试 B");
        assert!(peer
            .addresses
            .iter()
            .all(|address| address.ends_with(":43211")));
        drop(b);
        tokio::time::timeout(Duration::from_secs(20), async {
            while a.devices().iter().any(|device| device.id == b_id) {
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        })
        .await
        .expect("服务退出后没有从发现列表移除");
    }
}
