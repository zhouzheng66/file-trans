#[path = "support/resume.rs"]
mod support;

use anyhow::Result;
use file_trans_core::{transfers::Direction, Engine, EngineConfig};
use file_trans_identity::Identity;
use std::{net::SocketAddr, path::Path};
use support::*;
use tempfile::TempDir;

fn upgraded(root: &Path, address: SocketAddr) -> EngineConfig {
    let mut config = EngineConfig::new(root.join("B"), root.join("B/received"), "B".into());
    config.listen_address = address;
    config.enable_discovery = false;
    config.auto_accept_files = true;
    config
}

async fn waiting(receiver: &Engine, id: &str) {
    eventually(|| {
        receiver
            .snapshot()
            .unwrap()
            .activities
            .iter()
            .any(|activity| activity.id == id && activity.status == "awaiting_accept")
    })
    .await;
}

#[tokio::test]
async fn upgrade_accepts_old_pending_receipts_but_preserves_pause_and_rejection() -> Result<()> {
    let root = TempDir::new()?;
    let identity = Identity::generate()?;
    let a = node(root.path(), "A", Identity::generate()?, "", None).await;
    let b = node(root.path(), "B", identity.clone(), "", None).await;
    pair(&a, &b).await?;
    let source = root.path().join("upgrade.bin");
    tokio::fs::write(&source, b"upgrade without confirmation").await?;
    let paused = a.send_file(&b.local().id, source.clone()).await?;
    waiting(&b, &paused).await;
    b.pause_transfer(key(&b, &a, &paused, Direction::Incoming))
        .await?;
    let rejected = a.send_file(&b.local().id, source.clone()).await?;
    waiting(&b, &rejected).await;
    b.decide_file(&rejected, false)?;
    eventually(|| activity(&b, &rejected, "incoming").status == "rejected").await;
    let pending = a.send_file(&b.local().id, source.clone()).await?;
    waiting(&b, &pending).await;
    let address = b.local_address();
    b.shutdown().await;
    let b = Engine::start(upgraded(root.path(), address), identity).await?;
    assert_eq!(activity(&b, &paused, "incoming").status, "paused");
    assert_eq!(activity(&b, &rejected, "incoming").status, "rejected");
    finished(&a, &b, &pending, &source).await?;
    b.resume_transfer(key(&b, &a, &paused, Direction::Incoming))
        .await?;
    finished(&a, &b, &paused, &source).await?;
    a.shutdown().await;
    b.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn upgrade_with_an_unconfirmed_revoked_peer_starts_without_accepting_its_file() -> Result<()>
{
    let root = TempDir::new()?;
    let identity = Identity::generate()?;
    let a = node(root.path(), "A", Identity::generate()?, "", None).await;
    let b = node(root.path(), "B", identity.clone(), "", None).await;
    pair(&a, &b).await?;
    let source = root.path().join("revoked.bin");
    tokio::fs::write(&source, b"must not receive").await?;
    let id = a.send_file(&b.local().id, source).await?;
    waiting(&b, &id).await;
    b.forget_peer(&a.local().id)?;
    a.shutdown().await;
    b.shutdown().await;
    let b = Engine::start(upgraded(root.path(), b.local_address()), identity).await?;
    assert!(b.snapshot()?.peers.is_empty());
    assert!(!root.path().join("B/received/revoked.bin").exists());
    let database = rusqlite::Connection::open(root.path().join("B/file-trans.db"))?;
    let accepted: bool = database.query_row(
        "SELECT json_extract(data, '$.accepted') FROM file_transfers WHERE id=?1 AND direction='incoming'",
        [&id],
        |row| row.get(0),
    )?;
    assert!(!accepted);
    b.shutdown().await;
    Ok(())
}
