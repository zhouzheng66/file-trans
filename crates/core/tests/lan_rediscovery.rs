//! 真实 mDNS/系统 Bonjour 验收；不使用注入的地址，也不修改数据库模拟重连。
#[path = "support/resume.rs"]
mod support;

use anyhow::Result;
use file_trans_core::{
    model::{NetworkMode, NetworkSettings},
    Engine, EngineConfig,
};
use file_trans_identity::Identity;
use std::{path::Path, sync::Arc, time::Duration};
use support::{activity, eventually, finished, pair, write_file};
use tempfile::TempDir;
use tokio::{net::TcpListener, time::sleep};

async fn node(root: &Path, name: &str, identity: Identity) -> Arc<Engine> {
    let mut config = EngineConfig::new(
        root.join(name),
        root.join(name).join("received"),
        name.into(),
    );
    config.auto_accept_files = true;
    config.network = Some(NetworkSettings {
        mode: NetworkMode::Lan,
        relay_url: String::new(),
    });
    Engine::start(config, identity).await.unwrap()
}

#[tokio::test]
#[ignore = "需要真实局域网组播和系统 Bonjour 权限，单独运行"]
async fn paired_peers_rediscover_a_restarted_receiver_and_finish_a_waiting_file() -> Result<()> {
    let root = TempDir::new()?;
    let ai = Identity::generate()?;
    let bi = Identity::generate()?;
    let a = node(root.path(), "A", ai.clone()).await;
    let b = node(root.path(), "B", bi.clone()).await;
    pair(&a, &b).await?;
    a.send_text(&b.local().id, "首次发送".into()).await?;
    let previous = b.local_address();
    b.shutdown().await;
    let old_listener = TcpListener::bind(previous).await?;
    let source = root.path().join("自动重连.bin");
    write_file(&source, 1024 * 1024).await?;
    let id = a.send_file(&bi.id, source.clone()).await?;
    sleep(Duration::from_millis(300)).await;
    let b = node(root.path(), "B", bi).await;
    assert_ne!(previous.port(), b.local_address().port());
    finished(&a, &b, &id, &source).await?;
    assert!(a.snapshot()?.peers[0]
        .address
        .ends_with(&format!(":{}", b.local_address().port())));
    assert_eq!(
        activity(&a, &id, "outgoing").route,
        Some(file_trans_core::model::Route::Lan)
    );
    // 再重启发送端，保留信任和已验证地址，不调用任何配对/手动地址接口。
    a.shutdown().await;
    let a = node(root.path(), "A", ai).await;
    a.send_text(&b.local().id, "双方重启后仍可发送".into())
        .await?;
    eventually(|| {
        b.snapshot()
            .unwrap()
            .activities
            .iter()
            .any(|entry| entry.content == "双方重启后仍可发送")
    })
    .await;
    a.shutdown().await;
    b.shutdown().await;
    drop(old_listener);
    Ok(())
}
