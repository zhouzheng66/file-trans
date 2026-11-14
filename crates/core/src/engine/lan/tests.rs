use super::*;
use crate::{
    model::{NetworkMode, NetworkSettings, Peer},
    EngineConfig,
};
use file_trans_identity::Identity;
use file_trans_protocol::{read_message, write_message, Message, VERSION};
use std::path::Path;
use tempfile::TempDir;

async fn node(root: &Path, name: &str, identity: Identity) -> Arc<Engine> {
    let mut config = EngineConfig::new(
        root.join(name),
        root.join(name).join("received"),
        name.into(),
    );
    config.enable_discovery = false;
    config.listen_address = "0.0.0.0:0".parse().unwrap();
    config.network = Some(NetworkSettings {
        mode: NetworkMode::Lan,
        relay_url: String::new(),
    });
    Engine::start(config, identity).await.unwrap()
}

fn trust(a: &Engine, b: &Engine, address: SocketAddr) {
    a.store
        .save_peer(&Peer {
            id: b.local.id.clone(),
            name: b.local.name.clone(),
            address: address.to_string(),
        })
        .unwrap();
}

fn advertisement(engine: &Engine, address: SocketAddr) -> DiscoveredDevice {
    DiscoveredDevice {
        id: engine.local.id.clone(),
        name: engine.local.name.clone(),
        addresses: vec![address.to_string()],
    }
}

#[tokio::test]
async fn discovers_a_changed_port_during_connect_and_remembers_it_after_restart() {
    let root = TempDir::new().unwrap();
    let ai = Identity::generate().unwrap();
    let bi = Identity::generate().unwrap();
    let a = node(root.path(), "A", ai.clone()).await;
    let b = node(root.path(), "B", bi.clone()).await;
    let previous = b.local_address();
    trust(&a, &b, previous);
    trust(&b, &a, a.local_address());
    b.shutdown().await;
    // 占住旧端口，确保新实例使用不同端口，并模拟旧地址仍响应 TCP 却不完成 TLS。
    let old_listener = tokio::net::TcpListener::bind(previous).await.unwrap();
    let b = node(root.path(), "B", bi).await;
    assert_ne!(previous.port(), b.local_address().port());
    let current = SocketAddr::new("127.0.0.1".parse().unwrap(), b.local_address().port());
    let (updates, mut devices) = watch::channel(Vec::new());
    let found = advertisement(&b, current);
    let refresh = async {
        sleep(Duration::from_millis(80)).await;
        updates.send(vec![found]).unwrap();
        Ok(())
    };
    let mut stream = a
        .connect_lan(
            &b.local.id,
            previous.to_string(),
            &mut devices,
            refresh,
            true,
        )
        .await
        .unwrap();
    write_message(&mut stream, &Message::Ping { version: VERSION })
        .await
        .unwrap();
    assert!(
        matches!(read_message(&mut stream).await.unwrap(), Message::Pong { device } if device.id == b.local.id)
    );
    drop(stream);
    assert_eq!(
        a.store.peer(&b.local.id).unwrap().address,
        current.to_string()
    );
    a.shutdown().await;
    let a = node(root.path(), "A", ai).await;
    // 发送方重启、没有任何发现缓存时，仍可利用上次验证成功的新地址收发。
    a.send_text(&b.local.id, "端口变化后无需重新配对".into())
        .await
        .unwrap();
    assert_eq!(
        b.snapshot().unwrap().activities[0].content,
        "端口变化后无需重新配对"
    );
    drop(old_listener);
    a.shutdown().await;
    b.shutdown().await;
}

#[tokio::test]
async fn spoofed_discovery_never_updates_the_saved_address_or_restores_revoked_trust() {
    let root = TempDir::new().unwrap();
    let a = node(root.path(), "A", Identity::generate().unwrap()).await;
    let b = node(root.path(), "B", Identity::generate().unwrap()).await;
    let stranger = node(root.path(), "stranger", Identity::generate().unwrap()).await;
    let remembered = "127.0.0.1:1".parse().unwrap();
    trust(&a, &b, remembered);
    let fake = advertisement(&b, stranger.local_address());
    let (_updates, mut devices) = watch::channel(vec![fake]);
    let result = a
        .connect_lan(
            &b.local.id,
            remembered.to_string(),
            &mut devices,
            async { Ok(()) },
            false,
        )
        .await;
    assert!(result.is_err());
    assert_eq!(
        a.store.peer(&b.local.id).unwrap().address,
        remembered.to_string()
    );
    a.forget_peer(&b.local.id).unwrap();
    a.store
        .update_peer_address(&b.local.id, &b.local_address().to_string())
        .unwrap();
    assert!(a.store.peer(&b.local.id).is_err());
    a.shutdown().await;
    b.shutdown().await;
    stranger.shutdown().await;
}
