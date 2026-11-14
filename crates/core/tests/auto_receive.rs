#[path = "support/resume.rs"]
mod support;

use anyhow::Result;
use file_trans_core::{
    model::{NetworkMode, NetworkSettings, Route},
    transfers::Direction,
    Engine, EngineConfig,
};
use file_trans_identity::{connect, Identity};
use file_trans_protocol::{read_message, write_message, Message, VERSION};
use std::{path::Path, sync::atomic::Ordering};
use support::*;
use tempfile::TempDir;

fn config(root: &Path, name: &str, relay: &str) -> EngineConfig {
    let mut config = EngineConfig::new(
        root.join(name),
        root.join(name).join("received"),
        name.into(),
    );
    config.listen_address = "127.0.0.1:0".parse().unwrap();
    config.enable_discovery = false;
    config.auto_accept_files = true;
    config.network = Some(NetworkSettings {
        mode: NetworkMode::Auto,
        relay_url: relay.into(),
    });
    config
}

async fn completed(a: &Engine, b: &Engine, id: &str) {
    eventually(|| {
        a.snapshot().unwrap().activities.iter().any(|task| {
            task.id == id && ["completed", "failed", "rejected"].contains(&task.status.as_str())
        })
    })
    .await;
    for (engine, direction) in [(a, "outgoing"), (b, "incoming")] {
        let task = activity(engine, id, direction);
        assert_eq!(task.status, "completed", "{task:?}");
    }
}

#[tokio::test]
async fn automatically_receives_files_and_folders_on_both_routes_in_a_persisted_directory(
) -> Result<()> {
    let root = TempDir::new()?;
    let service = Service::start().await;
    let bi = Identity::generate()?;
    let a = Engine::start(
        config(root.path(), "A", &service.url),
        Identity::generate()?,
    )
    .await?;
    let b = Engine::start(config(root.path(), "B", &service.url), bi.clone()).await?;
    pair(&a, &b).await?;
    eventually(|| a.relay_status().state == "online" && b.relay_status().state == "online").await;
    let directory = root.path().join("自选接收目录");
    b.set_download_dir(directory.clone()).await?;
    assert_eq!(Path::new(&b.snapshot()?.download_dir), directory);
    tokio::fs::write(directory.join("资料.bin"), b"original").await?;
    let source = root.path().join("资料.bin");
    tokio::fs::write(&source, [0, 1, 128, 255]).await?;
    let folder = root.path().join("项目");
    tokio::fs::create_dir_all(folder.join("子目录/空目录")).await?;
    tokio::fs::write(folder.join("子目录/文件.txt"), "你好").await?;
    for (mode, route) in [
        (NetworkMode::Lan, Route::Lan),
        (NetworkMode::Internet, Route::Relay),
    ] {
        for engine in [&a, &b] {
            let mut settings = engine.network_settings();
            settings.mode = mode;
            engine.set_network_settings(settings).await?;
        }
        for path in [&source, &folder] {
            let id = a.send_path(&b.local().id, path.to_path_buf()).await?;
            completed(&a, &b, &id).await;
            let received = activity(&b, &id, "incoming");
            assert_eq!(received.route, Some(route));
            let saved = std::path::PathBuf::from(received.path.unwrap());
            assert_eq!(saved.parent(), Some(directory.as_path()));
            if path == &source {
                assert_eq!(tokio::fs::read(saved).await?, [0, 1, 128, 255]);
            } else {
                assert!(saved.join("子目录/空目录").is_dir());
                assert_eq!(
                    tokio::fs::read_to_string(saved.join("子目录/文件.txt")).await?,
                    "你好"
                );
            }
        }
    }
    assert_eq!(
        tokio::fs::read(directory.join("资料.bin")).await?,
        b"original"
    );
    assert!(b.set_download_dir(source).await.is_err());
    assert_eq!(Path::new(&b.snapshot()?.download_dir), directory);
    a.shutdown().await;
    b.shutdown().await;
    let b = Engine::start(config(root.path(), "B", &service.url), bi).await?;
    assert_eq!(Path::new(&b.snapshot()?.download_dir), directory);
    b.shutdown().await;
    service.stop().await;
    Ok(())
}

#[tokio::test]
async fn automatic_receipt_keeps_its_directory_and_offset_after_pause_restart_and_migration(
) -> Result<()> {
    let root = TempDir::new()?;
    let service = Service::start().await;
    let bi = Identity::generate()?;
    let a = Engine::start(
        config(root.path(), "A", &service.url),
        Identity::generate()?,
    )
    .await?;
    let b = Engine::start(config(root.path(), "B", &service.url), bi.clone()).await?;
    pair(&a, &b).await?;
    eventually(|| a.relay_status().state == "online" && b.relay_status().state == "online").await;
    let address = b.local_address();
    let proxy = Proxy::start(address).await;
    remember(root.path(), "A", &b, proxy.address);
    let source = root.path().join("large.bin");
    write_file(&source, 24 * 1024 * 1024).await?;
    let id = a.send_file(&b.local().id, source.clone()).await?;
    eventually(|| activity(&a, &id, "outgoing").transferred >= 4 * 1024 * 1024).await;
    b.pause_transfer(key(&b, &a, &id, Direction::Incoming))
        .await?;
    eventually(|| proxy.active.load(Ordering::SeqCst) == 0).await;
    let offset = activity(&b, &id, "incoming").transferred;
    assert!(offset > 0);
    let new_directory = root.path().join("新的接收目录");
    b.set_download_dir(new_directory.clone()).await?;
    b.shutdown().await;
    let mut configuration = config(root.path(), "B", &service.url);
    configuration.listen_address = address;
    let b = Engine::start(configuration, bi).await?;
    assert_eq!(activity(&b, &id, "incoming").status, "paused");
    b.resume_transfer(key(&b, &a, &id, Direction::Incoming))
        .await?;
    let mut settings = a.network_settings();
    settings.mode = NetworkMode::Internet;
    a.set_network_settings(settings).await?;
    finished(&a, &b, &id, &source).await?;
    let received = activity(&b, &id, "incoming");
    assert!(received.resumed_from >= offset);
    assert_eq!(received.route, Some(Route::Relay));
    assert_eq!(
        Path::new(received.path.as_ref().unwrap()).parent(),
        Some(root.path().join("B/received").as_path())
    );
    let next = a.send_file(&b.local().id, source.clone()).await?;
    finished(&a, &b, &next, &source).await?;
    assert_eq!(
        Path::new(activity(&b, &next, "incoming").path.as_ref().unwrap()).parent(),
        Some(new_directory.as_path())
    );
    a.shutdown().await;
    b.shutdown().await;
    proxy.stop().await;
    service.stop().await;
    Ok(())
}

#[tokio::test]
async fn legacy_sender_is_auto_accepted_but_revoked_and_unknown_devices_are_not() -> Result<()> {
    use sha2::{Digest, Sha256};
    let root = TempDir::new()?;
    let ai = Identity::generate()?;
    let a = Engine::start(config(root.path(), "A", ""), ai.clone()).await?;
    let b = Engine::start(config(root.path(), "B", ""), Identity::generate()?).await?;
    pair(&a, &b).await?;
    let id = uuid::Uuid::new_v4().to_string();
    let mut stream = connect(
        tokio::net::TcpStream::connect(b.local_address()).await?,
        &ai,
        Some(&b.local().id),
    )
    .await?;
    write_message(
        &mut stream,
        &Message::FileOffer {
            version: VERSION,
            id: id.clone(),
            name: "legacy.bin".into(),
            size: 0,
        },
    )
    .await?;
    assert!(matches!(
        read_message(&mut stream).await?,
        Message::FileDecision { accepted: true }
    ));
    write_message(
        &mut stream,
        &Message::FileEnd {
            sha256: hex::encode(Sha256::digest([])),
        },
    )
    .await?;
    assert!(matches!(
        read_message(&mut stream).await?,
        Message::Ack { .. }
    ));
    b.forget_peer(&a.local().id)?;
    for identity in [&ai, &Identity::generate()?] {
        let mut stream = connect(
            tokio::net::TcpStream::connect(b.local_address()).await?,
            identity,
            Some(&b.local().id),
        )
        .await?;
        write_message(
            &mut stream,
            &Message::FileOffer {
                version: VERSION,
                id: uuid::Uuid::new_v4().to_string(),
                name: "forbidden.bin".into(),
                size: 0,
            },
        )
        .await?;
        assert!(read_message(&mut stream).await.is_err());
    }
    assert!(!root.path().join("B/received/forbidden.bin").exists());
    assert_eq!(b.snapshot()?.activities.len(), 1);
    a.shutdown().await;
    b.shutdown().await;
    Ok(())
}
