use anyhow::Result;
use file_trans_core::{
    model::{NetworkMode, NetworkSettings, Route},
    Engine, EngineConfig,
};
use file_trans_identity::Identity;
use file_trans_relay_server::{serve, RelayLimits};
use std::{path::Path, sync::Arc, time::Duration};
use tempfile::TempDir;
use tokio::{
    net::TcpListener,
    task::JoinHandle,
    time::{sleep, timeout},
};
use tokio_util::sync::CancellationToken;

struct Service {
    url: String,
    address: std::net::SocketAddr,
    cancel: CancellationToken,
    task: JoinHandle<Result<()>>,
}
impl Service {
    async fn start() -> Self {
        Self::on("127.0.0.1:0".parse().unwrap(), RelayLimits::default()).await
    }
    async fn on(address: std::net::SocketAddr, limits: RelayLimits) -> Self {
        let listener = TcpListener::bind(address).await.unwrap();
        let address = listener.local_addr().unwrap();
        let cancel = CancellationToken::new();
        let task = tokio::spawn(serve(listener, limits, cancel.clone()));
        Self {
            url: format!("ws://{address}"),
            address,
            cancel,
            task,
        }
    }
    async fn stop(self) {
        self.cancel.cancel();
        timeout(Duration::from_secs(5), self.task)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
    }
}

async fn node(root: &Path, name: &str, identity: Identity, url: &str) -> Arc<Engine> {
    let mut config = EngineConfig::new(
        root.join(name),
        root.join(name).join("received"),
        name.into(),
    );
    config.listen_address = "127.0.0.1:0".parse().unwrap();
    config.enable_discovery = false;
    config.decision_timeout = Duration::from_secs(4);
    config.network = Some(NetworkSettings {
        mode: NetworkMode::Auto,
        relay_url: url.into(),
    });
    Engine::start(config, identity).await.unwrap()
}

async fn eventually(mut condition: impl FnMut() -> bool) {
    timeout(Duration::from_secs(15), async {
        while !condition() {
            sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("condition timed out");
}

async fn confirm(a: &Arc<Engine>, b: &Arc<Engine>, id: &str) {
    let a_pair = a
        .snapshot()
        .unwrap()
        .pairings
        .into_iter()
        .find(|pair| pair.id == id)
        .unwrap();
    let b_pair = b
        .snapshot()
        .unwrap()
        .pairings
        .into_iter()
        .find(|pair| pair.id == id)
        .unwrap();
    assert_eq!(a_pair.code, b_pair.code);
    a.decide_pairing(id, true).unwrap();
    b.decide_pairing(id, true).unwrap();
    eventually(|| {
        !a.snapshot().unwrap().peers.is_empty() && !b.snapshot().unwrap().peers.is_empty()
    })
    .await;
}

async fn remote_pair(a: &Arc<Engine>, b: &Arc<Engine>) {
    eventually(|| a.relay_status().state == "online" && b.relay_status().state == "online").await;
    b.create_invite().await.unwrap();
    let code = b.relay_status().invite_code.unwrap();
    let id = a.request_pair_code(code).await.unwrap();
    confirm(a, b, &id).await;
}

async fn mode(engine: &Engine, mode: NetworkMode) {
    let mut settings = engine.network_settings();
    settings.mode = mode;
    engine.set_network_settings(settings).await.unwrap();
}

#[tokio::test]
async fn internet_invite_pairs_and_transfers_text_and_binary_with_end_to_end_tls() -> Result<()> {
    let root = TempDir::new()?;
    let service = Service::start().await;
    let a = node(root.path(), "A", Identity::generate()?, &service.url).await;
    let b = node(root.path(), "B", Identity::generate()?, &service.url).await;
    remote_pair(&a, &b).await;
    assert!(
        a.snapshot()?.peers[0].address.is_empty(),
        "relay pairing must not pretend to know a LAN address"
    );
    a.send_text(&b.local().id, "跨网中继你好 🖥️".into()).await?;
    assert_eq!(a.snapshot()?.activities[0].route, Some(Route::Relay));
    assert_eq!(b.snapshot()?.activities[0].route, Some(Route::Relay));
    let contents: Vec<u8> = (0..1_100_000).map(|index| (index % 251) as u8).collect();
    let source = root.path().join("中继测试.bin");
    tokio::fs::write(&source, &contents).await?;
    let id = a.send_file(&b.local().id, source).await?;
    eventually(|| {
        b.snapshot()
            .unwrap()
            .activities
            .iter()
            .any(|entry| entry.id == id && entry.status == "awaiting_accept")
    })
    .await;
    b.decide_file(&id, true)?;
    eventually(|| {
        a.snapshot()
            .unwrap()
            .activities
            .iter()
            .any(|entry| entry.id == id && ["completed", "failed"].contains(&entry.status.as_str()))
    })
    .await;
    let sent = a
        .snapshot()?
        .activities
        .into_iter()
        .find(|entry| entry.id == id)
        .unwrap();
    assert_eq!(sent.status, "completed", "{:?}", sent.error);
    let received = b
        .snapshot()?
        .activities
        .into_iter()
        .find(|entry| entry.id == id)
        .unwrap();
    assert_eq!(received.route, Some(Route::Relay));
    assert_eq!(tokio::fs::read(received.path.unwrap()).await?, contents);
    a.shutdown().await;
    b.shutdown().await;
    service.stop().await;
    Ok(())
}

#[tokio::test]
async fn auto_prefers_lan_falls_back_and_respects_forced_modes() -> Result<()> {
    let root = TempDir::new()?;
    let service = Service::start().await;
    let a = node(root.path(), "A", Identity::generate()?, &service.url).await;
    let b = node(root.path(), "B", Identity::generate()?, &service.url).await;
    let id = a.request_pair(b.local_address()).await?;
    confirm(&a, &b, &id).await;
    eventually(|| a.relay_status().state == "online" && b.relay_status().state == "online").await;
    a.send_text(&b.local().id, "自动优先局域网".into()).await?;
    assert_eq!(a.snapshot()?.activities[0].route, Some(Route::Lan));
    mode(&a, NetworkMode::Internet).await;
    a.send_text(&b.local().id, "强制中继".into()).await?;
    assert_eq!(a.snapshot()?.activities[0].route, Some(Route::Relay));
    mode(&a, NetworkMode::Auto).await;
    mode(&b, NetworkMode::Internet).await;
    a.send_text(&b.local().id, "局域网被拒绝，自动回退".into())
        .await?;
    assert_eq!(a.snapshot()?.activities[0].route, Some(Route::Relay));
    let count = b.snapshot()?.activities.len();
    mode(&a, NetworkMode::Lan).await;
    assert_eq!(a.relay_status().state, "disabled");
    assert!(a
        .send_text(&b.local().id, "不得走互联网".into())
        .await
        .is_err());
    assert_eq!(b.snapshot()?.activities.len(), count);
    assert!(a.create_invite().await.is_err());
    mode(&b, NetworkMode::Lan).await;
    service.stop().await;
    a.send_text(&b.local().id, "没有公网仍可直传".into())
        .await?;
    assert_eq!(a.snapshot()?.activities[0].route, Some(Route::Lan));
    a.shutdown().await;
    b.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn rotates_invites_rejects_untrusted_tunnels_and_reconnects_after_restart() -> Result<()> {
    let root = TempDir::new()?;
    let service = Service::start().await;
    let address = service.address;
    let a = node(root.path(), "A", Identity::generate()?, &service.url).await;
    let b = node(root.path(), "B", Identity::generate()?, &service.url).await;
    eventually(|| a.relay_status().state == "online" && b.relay_status().state == "online").await;
    b.create_invite().await?;
    let obsolete = b.relay_status().invite_code.unwrap();
    b.create_invite().await?;
    assert!(a.request_pair_code(obsolete).await.is_err());
    remote_pair(&a, &b).await;
    let stranger = node(root.path(), "C", Identity::generate()?, &service.url).await;
    eventually(|| stranger.relay_status().state == "online").await;
    // 已记住 ID 也不能绕过接收端的可信设备授权。
    let (events, _receiver) = tokio::sync::mpsc::channel(4);
    let raw = file_trans_relay::RelayClient::start(&service.url, Identity::generate()?, events)?;
    eventually(|| raw.snapshot().state == "online").await;
    assert!(raw
        .open(file_trans_relay::protocol::Target::Peer(
            b.local().id.clone()
        ))
        .await
        .is_err());
    assert!(b.snapshot()?.activities.is_empty());
    raw.stop();
    stranger.shutdown().await;
    service.stop().await;
    eventually(|| a.relay_status().state != "online" && b.relay_status().state != "online").await;
    let restarted = Service::on(address, RelayLimits::default()).await;
    eventually(|| a.relay_status().state == "online" && b.relay_status().state == "online").await;
    mode(&a, NetworkMode::Internet).await;
    a.send_text(&b.local().id, "重启中继后继续传输".into())
        .await?;
    b.forget_peer(&a.local().id)?;
    assert!(a
        .send_text(&b.local().id, "信任已移除".into())
        .await
        .is_err());
    a.shutdown().await;
    b.shutdown().await;
    restarted.stop().await;
    Ok(())
}

#[tokio::test]
async fn settings_persist_and_internet_only_never_uses_a_working_lan() -> Result<()> {
    let root = TempDir::new()?;
    let identity = Identity::generate()?;
    let a = node(root.path(), "A", identity.clone(), "").await;
    let b = node(root.path(), "B", Identity::generate()?, "").await;
    let id = a.request_pair(b.local_address()).await?;
    confirm(&a, &b, &id).await;
    mode(&a, NetworkMode::Internet).await;
    assert!(a
        .send_text(&b.local().id, "未配置中继，不允许偷偷直连".into())
        .await
        .is_err());
    assert!(b.snapshot()?.activities.is_empty());
    assert!(a.request_pair(b.local_address()).await.is_err());
    a.shutdown().await;
    let mut config = EngineConfig::new(
        root.path().join("A"),
        root.path().join("A/received"),
        "A".into(),
    );
    config.listen_address = "127.0.0.1:0".parse()?;
    config.enable_discovery = false;
    let restarted = Engine::start(config, identity).await?;
    assert_eq!(restarted.network_settings().mode, NetworkMode::Internet);
    assert_eq!(restarted.snapshot()?.peers[0].id, b.local().id);
    restarted.shutdown().await;
    b.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn relay_quota_interruption_retains_checkpoint_until_cancelled() -> Result<()> {
    let root = TempDir::new()?;
    let limits = RelayLimits {
        max_session_bytes: 128 * 1024,
        ..Default::default()
    };
    let service = Service::on("127.0.0.1:0".parse()?, limits).await;
    let a = node(root.path(), "A", Identity::generate()?, &service.url).await;
    let b = node(root.path(), "B", Identity::generate()?, &service.url).await;
    let id = a.request_pair(b.local_address()).await?;
    confirm(&a, &b, &id).await;
    eventually(|| a.relay_status().state == "online" && b.relay_status().state == "online").await;
    mode(&a, NetworkMode::Internet).await;
    let source = root.path().join("quota.bin");
    tokio::fs::write(&source, vec![42; 512 * 1024]).await?;
    let id = a.send_file(&b.local().id, source).await?;
    eventually(|| {
        b.snapshot()
            .unwrap()
            .activities
            .iter()
            .any(|entry| entry.id == id && entry.status == "awaiting_accept")
    })
    .await;
    b.decide_file(&id, true)?;
    eventually(|| {
        a.snapshot()
            .unwrap()
            .activities
            .iter()
            .any(|entry| entry.id == id && entry.status == "waiting_retry")
            && b.snapshot()
                .unwrap()
                .activities
                .iter()
                .any(|entry| entry.id == id && entry.status == "waiting_connection")
    })
    .await;
    assert!(!root.path().join("B/received/quota.bin").exists());
    let partial: String = rusqlite::Connection::open(root.path().join("B/file-trans.db"))?
        .query_row(
            "SELECT json_extract(data,'$.partial_path') FROM file_transfers WHERE id=?1",
            [&id],
            |row| row.get(0),
        )?;
    assert!(std::path::Path::new(&partial).exists());
    a.cancel_transfer(file_trans_core::transfers::TransferKey {
        id: id.clone(),
        peer_id: b.local().id.clone(),
        direction: file_trans_core::transfers::Direction::Outgoing,
    })
    .await?;
    b.cancel_transfer(file_trans_core::transfers::TransferKey {
        id,
        peer_id: a.local().id.clone(),
        direction: file_trans_core::transfers::Direction::Incoming,
    })
    .await?;
    assert!(!std::path::Path::new(&partial).exists());
    a.shutdown().await;
    b.shutdown().await;
    service.stop().await;
    Ok(())
}

#[tokio::test]
async fn tunnel_tickets_are_single_use_and_wrong_tickets_do_not_consume_a_leg() -> Result<()> {
    use file_trans_relay::{protocol::Target, RelayClient};
    let service = Service::start().await;
    let (a_events, _a_receiver) = tokio::sync::mpsc::channel(4);
    let (b_events, mut b_receiver) = tokio::sync::mpsc::channel(4);
    let a = RelayClient::start(&service.url, Identity::generate()?, a_events)?;
    let b_identity = Identity::generate()?;
    let b = RelayClient::start(&service.url, b_identity.clone(), b_events)?;
    eventually(|| a.snapshot().state == "online" && b.snapshot().state == "online").await;
    let caller = a.clone();
    let open = tokio::spawn(async move { caller.open(Target::Peer(b_identity.id)).await });
    let incoming = timeout(Duration::from_secs(5), b_receiver.recv())
        .await?
        .unwrap();
    assert!(file_trans_relay::tunnel_stream(
        &service.url,
        incoming.session_id.clone(),
        "wrong-ticket".into()
    )
    .await
    .is_err());
    let recipient = b.attach(&incoming).await?;
    let sender = open.await??;
    assert!(
        b.attach(&incoming).await.is_err(),
        "a used ticket cannot reopen a tunnel"
    );
    drop(recipient);
    drop(sender);
    a.stop();
    b.stop();
    service.stop().await;
    Ok(())
}
