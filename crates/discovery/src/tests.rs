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

#[tokio::test]
async fn publishes_port_changes_without_duplicate_entries_or_redundant_notifications() {
    let (sender, mut receiver) = watch::channel(Vec::new());
    let mut device = DiscoveredDevice {
        id: "a".repeat(64),
        name: "设备".into(),
        addresses: vec!["192.168.1.2:1000".into()],
    };
    publish(&sender, device.clone());
    receiver.changed().await.unwrap();
    publish(&sender, device.clone());
    assert!(!receiver.has_changed().unwrap());
    device.addresses = vec!["192.168.1.3:2000".into()];
    publish(&sender, device.clone());
    receiver.changed().await.unwrap();
    assert_eq!(*receiver.borrow(), vec![device]);
}

#[tokio::test]
#[ignore = "需要本地网络组播和 macOS Bonjour 权限，单独运行"]
async fn actively_resolves_the_same_identity_after_a_port_change() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let a_id = format!("c{nonce:063x}");
    let b_id = format!("d{nonce:063x}");
    let a = Discovery::start(&a_id, "解析测试 A", 43212).unwrap();
    let b = Discovery::start(&b_id, "解析测试 B", 43213).unwrap();
    wait_for_port(&a, &b_id, 43213).await;
    drop(b);
    tokio::time::sleep(Duration::from_millis(200)).await;
    let b = Discovery::start(&b_id, "解析测试 B", 43214).unwrap();
    wait_for_port(&a, &b_id, 43214).await;
    drop(b);
}

async fn wait_for_port(discovery: &Discovery, id: &str, port: u16) {
    tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            let _ = discovery.resolve_peer(id).await;
            #[cfg(target_os = "macos")]
            if !super::macos::resolve_peer(id).await.is_ok_and(|device| {
                device
                    .addresses
                    .iter()
                    .any(|address| address.ends_with(&format!(":{port}")))
            }) {
                tokio::time::sleep(Duration::from_millis(100)).await;
                continue;
            }
            if discovery.devices().iter().any(|device| {
                device.id == id
                    && device
                        .addresses
                        .iter()
                        .any(|address| address.ends_with(&format!(":{port}")))
            }) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .expect("没有自动更新设备端口");
}
