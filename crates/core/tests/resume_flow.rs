#[path = "support/resume.rs"]
mod support;
use anyhow::Result;
use file_trans_core::{
    model::{NetworkMode, Route},
    transfers::Direction,
};
use file_trans_identity::Identity;
use std::sync::atomic::Ordering;
use support::*;
use tempfile::TempDir;

#[tokio::test]
async fn sender_pause_resume_sends_only_the_missing_suffix() -> Result<()> {
    let root = TempDir::new()?;
    let a = node(root.path(), "A", Identity::generate()?, "", None).await;
    let b = node(root.path(), "B", Identity::generate()?, "", None).await;
    pair(&a, &b).await?;
    let proxy = Proxy::start(b.local_address()).await;
    remember(root.path(), "A", &b, proxy.address);
    let path = root.path().join("source.bin");
    write_file(&path, 24 * 1024 * 1024).await?;
    let id = offer(&a, &b, &path).await?;
    eventually(|| activity(&a, &id, "outgoing").transferred >= 4 * 1024 * 1024).await;
    a.pause_transfer(key(&a, &b, &id, Direction::Outgoing))
        .await?;
    eventually(|| proxy.active.load(Ordering::SeqCst) == 0).await;
    assert_eq!(activity(&a, &id, "outgoing").status, "paused");
    let offset = activity(&b, &id, "incoming").transferred;
    assert!((4 * 1024 * 1024..24 * 1024 * 1024).contains(&offset));
    let bytes = proxy.bytes.load(Ordering::SeqCst);
    a.resume_transfer(key(&a, &b, &id, Direction::Outgoing))
        .await?;
    finished(&a, &b, &id, &path).await?;
    assert!(activity(&a, &id, "outgoing").resumed_from >= offset);
    let resumed_traffic = proxy.bytes.load(Ordering::SeqCst) - bytes;
    assert!(
        resumed_traffic < 24 * 1024 * 1024 - offset + 1024 * 1024,
        "already confirmed prefix was sent again"
    );
    a.shutdown().await;
    b.shutdown().await;
    proxy.stop().await;
    Ok(())
}

#[tokio::test]
async fn network_break_and_return_automatically_resume_without_new_acceptance() -> Result<()> {
    let root = TempDir::new()?;
    let a = node(root.path(), "A", Identity::generate()?, "", None).await;
    let b = node(root.path(), "B", Identity::generate()?, "", None).await;
    pair(&a, &b).await?;
    let proxy = Proxy::start(b.local_address()).await;
    let address = proxy.address;
    remember(root.path(), "A", &b, address);
    let path = root.path().join("source.bin");
    write_file(&path, 24 * 1024 * 1024).await?;
    let id = offer(&a, &b, &path).await?;
    eventually(|| activity(&a, &id, "outgoing").transferred >= 4 * 1024 * 1024).await;
    proxy.stop().await;
    eventually(|| {
        activity(&a, &id, "outgoing").status == "waiting_retry"
            && activity(&b, &id, "incoming").status == "waiting_connection"
    })
    .await;
    let offset = activity(&b, &id, "incoming").transferred;
    assert!(offset > 0);
    let proxy = Proxy::on(address, b.local_address()).await;
    finished(&a, &b, &id, &path).await?;
    assert!(activity(&a, &id, "outgoing").resumed_from >= offset);
    a.shutdown().await;
    b.shutdown().await;
    proxy.stop().await;
    Ok(())
}

#[tokio::test]
async fn active_file_migrates_lan_to_relay_and_back_without_restarting() -> Result<()> {
    let root = TempDir::new()?;
    let service = Service::start().await;
    let a = node(root.path(), "A", Identity::generate()?, &service.url, None).await;
    let b = node(root.path(), "B", Identity::generate()?, &service.url, None).await;
    pair(&a, &b).await?;
    eventually(|| a.relay_status().state == "online" && b.relay_status().state == "online").await;
    let proxy = Proxy::start(b.local_address()).await;
    remember(root.path(), "A", &b, proxy.address);
    let path = root.path().join("source.bin");
    write_file(&path, 64 * 1024 * 1024).await?;
    let id = offer(&a, &b, &path).await?;
    eventually(|| activity(&a, &id, "outgoing").transferred >= 4 * 1024 * 1024).await;
    let mut settings = a.network_settings();
    settings.mode = NetworkMode::Internet;
    a.set_network_settings(settings).await?;
    eventually(|| {
        let task = activity(&a, &id, "outgoing");
        task.route == Some(Route::Relay) && task.resumed_from > 0
    })
    .await;
    let offset = activity(&a, &id, "outgoing").resumed_from;
    let mut settings = a.network_settings();
    settings.mode = NetworkMode::Lan;
    a.set_network_settings(settings).await?;
    finished(&a, &b, &id, &path).await?;
    let task = activity(&a, &id, "outgoing");
    assert_eq!(task.route, Some(Route::Lan));
    assert!(task.resumed_from >= offset);
    assert_eq!(
        b.snapshot()?
            .activities
            .iter()
            .filter(|task| task.id == id)
            .count(),
        1
    );
    a.shutdown().await;
    b.shutdown().await;
    proxy.stop().await;
    service.stop().await;
    Ok(())
}

#[tokio::test]
async fn both_nodes_restart_preserve_independent_pauses_and_checkpoint() -> Result<()> {
    let root = TempDir::new()?;
    let a_identity = Identity::generate()?;
    let b_identity = Identity::generate()?;
    let a = node(root.path(), "A", a_identity.clone(), "", None).await;
    let b = node(root.path(), "B", b_identity.clone(), "", None).await;
    let a_address = a.local_address();
    let b_address = b.local_address();
    pair(&a, &b).await?;
    let proxy = Proxy::start(b_address).await;
    remember(root.path(), "A", &b, proxy.address);
    let path = root.path().join("source.bin");
    write_file(&path, 24 * 1024 * 1024).await?;
    let id = offer(&a, &b, &path).await?;
    eventually(|| activity(&a, &id, "outgoing").transferred >= 4 * 1024 * 1024).await;
    b.pause_transfer(key(&b, &a, &id, Direction::Incoming))
        .await?;
    a.pause_transfer(key(&a, &b, &id, Direction::Outgoing))
        .await?;
    eventually(|| proxy.active.load(Ordering::SeqCst) == 0).await;
    let offset = activity(&b, &id, "incoming").transferred;
    a.shutdown().await;
    b.shutdown().await;
    let a = node(root.path(), "A", a_identity, "", Some(a_address)).await;
    let b = node(root.path(), "B", b_identity, "", Some(b_address)).await;
    assert_eq!(activity(&a, &id, "outgoing").status, "paused");
    assert_eq!(activity(&b, &id, "incoming").status, "paused");
    a.resume_transfer(key(&a, &b, &id, Direction::Outgoing))
        .await?;
    eventually(|| activity(&a, &id, "outgoing").status == "paused_by_peer").await;
    assert_eq!(activity(&b, &id, "incoming").transferred, offset);
    b.resume_transfer(key(&b, &a, &id, Direction::Incoming))
        .await?;
    finished(&a, &b, &id, &path).await?;
    assert!(activity(&a, &id, "outgoing").resumed_from >= offset);
    a.shutdown().await;
    b.shutdown().await;
    proxy.stop().await;
    Ok(())
}

#[tokio::test]
async fn source_changes_and_corrupted_partials_never_silently_resume() -> Result<()> {
    use tokio::io::AsyncWriteExt;
    let root = TempDir::new()?;
    let a = node(root.path(), "A", Identity::generate()?, "", None).await;
    let b = node(root.path(), "B", Identity::generate()?, "", None).await;
    pair(&a, &b).await?;
    let proxy = Proxy::start(b.local_address()).await;
    remember(root.path(), "A", &b, proxy.address);
    let path = root.path().join("source.bin");
    for corrupt_source in [true, false] {
        write_file(&path, 16 * 1024 * 1024).await?;
        let id = offer(&a, &b, &path).await?;
        eventually(|| activity(&a, &id, "outgoing").transferred >= 4 * 1024 * 1024).await;
        a.pause_transfer(key(&a, &b, &id, Direction::Outgoing))
            .await?;
        eventually(|| proxy.active.load(Ordering::SeqCst) == 0).await;
        let partial: String = rusqlite::Connection::open(root.path().join("B/file-trans.db"))?
            .query_row(
                "SELECT json_extract(data,'$.partial_path') FROM file_transfers WHERE id=?1",
                [&id],
                |row| row.get(0),
            )?;
        let target = if corrupt_source {
            path.as_path()
        } else {
            std::path::Path::new(&partial)
        };
        let mut file = tokio::fs::OpenOptions::new()
            .write(true)
            .open(target)
            .await?;
        file.write_all(b"changed").await?;
        file.sync_all().await?;
        drop(file);
        a.resume_transfer(key(&a, &b, &id, Direction::Outgoing))
            .await?;
        eventually(|| activity(&a, &id, "outgoing").status == "failed").await;
        let error = activity(&a, &id, "outgoing").error.unwrap();
        assert!(
            error.contains(if corrupt_source {
                "源文件"
            } else {
                "断点内容"
            }),
            "{error}"
        );
        assert!(!root.path().join("B/received/source.bin").exists());
        a.cancel_transfer(key(&a, &b, &id, Direction::Outgoing))
            .await?;
        b.cancel_transfer(key(&b, &a, &id, Direction::Incoming))
            .await?;
        assert!(!std::path::Path::new(&partial).exists());
    }
    a.shutdown().await;
    b.shutdown().await;
    proxy.stop().await;
    Ok(())
}

#[tokio::test]
async fn relay_restart_during_transfer_reconnects_and_resumes_automatically() -> Result<()> {
    let root = TempDir::new()?;
    let service = Service::start().await;
    let address = service.address;
    let proxy = Proxy::start(address).await;
    let url = format!("ws://{}", proxy.address);
    let a = node(root.path(), "A", Identity::generate()?, &url, None).await;
    let b = node(root.path(), "B", Identity::generate()?, &url, None).await;
    pair(&a, &b).await?;
    let mut settings = a.network_settings();
    settings.mode = NetworkMode::Internet;
    a.set_network_settings(settings).await?;
    eventually(|| a.relay_status().state == "online" && b.relay_status().state == "online").await;
    let path = root.path().join("source.bin");
    write_file(&path, 24 * 1024 * 1024).await?;
    let id = offer(&a, &b, &path).await?;
    eventually(|| activity(&a, &id, "outgoing").transferred >= 4 * 1024 * 1024).await;
    service.stop().await;
    eventually(|| activity(&b, &id, "incoming").status == "waiting_connection").await;
    let offset = activity(&b, &id, "incoming").transferred;
    assert!(offset > 0);
    let service = Service::on(address).await;
    finished(&a, &b, &id, &path).await?;
    assert!(activity(&a, &id, "outgoing").resumed_from >= offset);
    assert_eq!(activity(&a, &id, "outgoing").route, Some(Route::Relay));
    a.shutdown().await;
    b.shutdown().await;
    proxy.stop().await;
    service.stop().await;
    Ok(())
}

#[tokio::test]
async fn active_lan_failure_automatically_falls_back_to_relay() -> Result<()> {
    let root = TempDir::new()?;
    let service = Service::start().await;
    let a = node(root.path(), "A", Identity::generate()?, &service.url, None).await;
    let b = node(root.path(), "B", Identity::generate()?, &service.url, None).await;
    pair(&a, &b).await?;
    eventually(|| a.relay_status().state == "online" && b.relay_status().state == "online").await;
    let proxy = Proxy::start(b.local_address()).await;
    remember(root.path(), "A", &b, proxy.address);
    let path = root.path().join("source.bin");
    write_file(&path, 24 * 1024 * 1024).await?;
    let id = offer(&a, &b, &path).await?;
    eventually(|| activity(&a, &id, "outgoing").transferred >= 4 * 1024 * 1024).await;
    proxy.stop().await;
    finished(&a, &b, &id, &path).await?;
    let task = activity(&a, &id, "outgoing");
    assert_eq!(task.route, Some(Route::Relay));
    assert!(task.resumed_from > 0);
    a.shutdown().await;
    b.shutdown().await;
    service.stop().await;
    Ok(())
}

#[tokio::test]
async fn automatic_mode_moves_an_active_relay_file_back_to_lan_when_it_returns() -> Result<()> {
    let root = TempDir::new()?;
    let service = Service::start().await;
    let slow_relay = Proxy::start(service.address).await;
    let url = format!("ws://{}", slow_relay.address);
    let a = node(root.path(), "A", Identity::generate()?, &url, None).await;
    let b = node(root.path(), "B", Identity::generate()?, &url, None).await;
    pair(&a, &b).await?;
    eventually(|| a.relay_status().state == "online" && b.relay_status().state == "online").await;
    remember(root.path(), "A", &b, "127.0.0.1:1".parse()?);
    let path = root.path().join("source.bin");
    write_file(&path, 64 * 1024 * 1024).await?;
    let id = offer(&a, &b, &path).await?;
    eventually(|| activity(&a, &id, "outgoing").transferred >= 4 * 1024 * 1024).await;
    assert_eq!(activity(&a, &id, "outgoing").route, Some(Route::Relay));
    remember(root.path(), "A", &b, b.local_address());
    finished(&a, &b, &id, &path).await?;
    let task = activity(&a, &id, "outgoing");
    assert_eq!(task.route, Some(Route::Lan));
    assert!(task.resumed_from > 0);
    a.shutdown().await;
    b.shutdown().await;
    slow_relay.stop().await;
    service.stop().await;
    Ok(())
}
