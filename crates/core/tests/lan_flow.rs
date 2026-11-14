use anyhow::Result;
use file_trans_core::{Engine, EngineConfig};
use file_trans_identity::{connect, Identity};
use file_trans_protocol::{read_message, write_chunk, write_message, Message, VERSION};
use std::{path::Path, sync::Arc, time::Duration};
use tempfile::TempDir;
use tokio::{
    net::TcpStream,
    time::{sleep, timeout},
};
use uuid::Uuid;

async fn node(root: &Path, name: &str, identity: Identity) -> Arc<Engine> {
    let mut config = EngineConfig::new(
        root.join(name),
        root.join(name).join("received"),
        name.to_owned(),
    );
    config.listen_address = "127.0.0.1:0".parse().unwrap();
    config.enable_discovery = false;
    config.decision_timeout = Duration::from_secs(3);
    Engine::start(config, identity).await.unwrap()
}

async fn eventually(mut condition: impl FnMut() -> bool) {
    timeout(Duration::from_secs(8), async {
        while !condition() {
            sleep(Duration::from_millis(15)).await;
        }
    })
    .await
    .expect("condition timed out");
}

async fn pair(a: &Arc<Engine>, b: &Arc<Engine>) {
    let id = a.request_pair(b.local_address()).await.unwrap();
    let left = a
        .snapshot()
        .unwrap()
        .pairings
        .into_iter()
        .find(|entry| entry.id == id)
        .unwrap();
    let right = b
        .snapshot()
        .unwrap()
        .pairings
        .into_iter()
        .find(|entry| entry.id == id)
        .unwrap();
    assert_eq!(
        left.code, right.code,
        "TLS exporter must agree on both ends"
    );
    a.decide_pairing(&id, true).unwrap();
    b.decide_pairing(&id, true).unwrap();
    eventually(|| a.snapshot().unwrap().peers.len() == 1 && b.snapshot().unwrap().peers.len() == 1)
        .await;
}

#[tokio::test]
async fn pairs_sends_unicode_text_and_binary_files_without_overwriting() -> Result<()> {
    let root = TempDir::new()?;
    let a_identity = Identity::generate()?;
    let b_identity = Identity::generate()?;
    let a = node(root.path(), "A", a_identity.clone()).await;
    let b = node(root.path(), "B", b_identity.clone()).await;
    assert!(a.send_text(&b.local().id, "未配对".into()).await.is_err());
    pair(&a, &b).await;
    a.send_text(
        &b.local().id,
        "你好\n🖥️ https://example.com <script>plain text</script>".into(),
    )
    .await?;
    assert_eq!(
        b.snapshot()?.activities[0].content,
        "你好\n🖥️ https://example.com <script>plain text</script>"
    );

    let bytes: Vec<u8> = (0..1_100_000).map(|i| (i % 251) as u8).collect();
    for contents in [&bytes[..], &[][..]] {
        let source = root.path().join("任意格式.weird");
        tokio::fs::write(&source, contents).await?;
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
            a.snapshot().unwrap().activities.iter().any(|entry| {
                entry.id == id && ["completed", "failed"].contains(&entry.status.as_str())
            })
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
        assert_eq!(tokio::fs::read(received.path.unwrap()).await?, contents);
    }
    assert_eq!(
        tokio::fs::read(root.path().join("B/received/任意格式.weird")).await?,
        bytes
    );
    assert_eq!(
        tokio::fs::read(root.path().join("B/received/任意格式 (1).weird")).await?,
        b""
    );
    a.shutdown().await;
    b.shutdown().await;

    // 重启后保留可信设备、文字和历史；更新地址通过下次发现/配对完成。
    let restarted = node(root.path(), "A", a_identity).await;
    assert_eq!(restarted.snapshot()?.peers[0].id, b_identity.id);
    assert_eq!(restarted.snapshot()?.activities.len(), 3);
    restarted.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn rejects_unknown_peers_wrong_pins_and_revoked_trust() -> Result<()> {
    let root = TempDir::new()?;
    let a_identity = Identity::generate()?;
    let a = node(root.path(), "A", a_identity.clone()).await;
    let b = node(root.path(), "B", Identity::generate()?).await;
    let stranger = Identity::generate()?;
    let mut stream = connect(
        TcpStream::connect(b.local_address()).await?,
        &stranger,
        Some(&b.local().id),
    )
    .await?;
    write_message(
        &mut stream,
        &Message::Text {
            version: VERSION,
            id: Uuid::new_v4().to_string(),
            text: "unauthorized".into(),
        },
    )
    .await?;
    assert!(read_message(&mut stream).await.is_err());
    assert!(b.snapshot()?.activities.is_empty());
    assert!(connect(
        TcpStream::connect(b.local_address()).await?,
        &stranger,
        Some(&a.local().id)
    )
    .await
    .is_err());
    pair(&a, &b).await;
    b.forget_peer(&a.local().id)?;
    assert!(a.send_text(&b.local().id, "revoked".into()).await.is_err());
    assert!(b.snapshot()?.activities.is_empty());
    a.shutdown().await;
    b.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn pairing_rejection_and_expiration_never_create_trust() -> Result<()> {
    let root = TempDir::new()?;
    let a = node(root.path(), "A", Identity::generate()?).await;
    let b = node(root.path(), "B", Identity::generate()?).await;
    let id = a.request_pair(b.local_address()).await?;
    a.decide_pairing(&id, false)?;
    b.decide_pairing(&id, true)?;
    eventually(|| {
        a.snapshot()
            .unwrap()
            .pairings
            .iter()
            .any(|entry| entry.id == id && entry.status == "rejected")
    })
    .await;
    assert!(a.snapshot()?.peers.is_empty());
    assert!(b.snapshot()?.peers.is_empty());
    let remote_rejected = a.request_pair(b.local_address()).await?;
    b.decide_pairing(&remote_rejected, false)?;
    eventually(|| {
        a.snapshot()
            .unwrap()
            .pairings
            .iter()
            .any(|entry| entry.id == remote_rejected && entry.status == "rejected")
    })
    .await;
    assert!(a.snapshot()?.peers.is_empty());
    let expired = a.request_pair(b.local_address()).await?;
    eventually(|| {
        a.snapshot()
            .unwrap()
            .pairings
            .iter()
            .any(|entry| entry.id == expired && entry.status == "failed")
    })
    .await;
    assert!(a.decide_pairing(&expired, true).is_err());
    assert!(a.snapshot()?.peers.is_empty());
    a.shutdown().await;
    b.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn deduplicates_messages_and_rejects_corrupt_or_interrupted_files() -> Result<()> {
    let root = TempDir::new()?;
    let identity = Identity::generate()?;
    let a = node(root.path(), "A", identity.clone()).await;
    let b = node(root.path(), "B", Identity::generate()?).await;
    pair(&a, &b).await;
    let message_id = Uuid::new_v4().to_string();
    for _ in 0..2 {
        let mut stream = connect(
            TcpStream::connect(b.local_address()).await?,
            &identity,
            Some(&b.local().id),
        )
        .await?;
        write_message(
            &mut stream,
            &Message::Text {
                version: VERSION,
                id: message_id.clone(),
                text: "once".into(),
            },
        )
        .await?;
        assert!(matches!(
            read_message(&mut stream).await?,
            Message::Ack { .. }
        ));
    }
    assert_eq!(b.snapshot()?.activities.len(), 1);
    let mut conflicting = connect(
        TcpStream::connect(b.local_address()).await?,
        &identity,
        Some(&b.local().id),
    )
    .await?;
    write_message(
        &mut conflicting,
        &Message::Text {
            version: VERSION,
            id: message_id,
            text: "different content".into(),
        },
    )
    .await?;
    assert!(read_message(&mut conflicting).await.is_err());
    assert_eq!(b.snapshot()?.activities[0].content, "once");
    for corrupt in [true, false] {
        let mut stream = connect(
            TcpStream::connect(b.local_address()).await?,
            &identity,
            Some(&b.local().id),
        )
        .await?;
        let id = Uuid::new_v4().to_string();
        write_message(
            &mut stream,
            &Message::FileOffer {
                version: VERSION,
                id: id.clone(),
                name: "broken.bin".into(),
                size: 3,
            },
        )
        .await?;
        eventually(|| {
            b.snapshot()
                .unwrap()
                .activities
                .iter()
                .any(|entry| entry.id == id)
        })
        .await;
        b.decide_file(&id, true)?;
        assert!(matches!(
            read_message(&mut stream).await?,
            Message::FileDecision { accepted: true }
        ));
        write_chunk(&mut stream, b"abc").await?;
        if corrupt {
            write_message(
                &mut stream,
                &Message::FileEnd {
                    sha256: "incorrect".into(),
                },
            )
            .await?;
            assert!(read_message(&mut stream).await.is_err());
        }
        drop(stream);
        eventually(|| {
            b.snapshot()
                .unwrap()
                .activities
                .iter()
                .any(|entry| entry.id == id && entry.status == "failed")
        })
        .await;
        assert_eq!(
            std::fs::read_dir(root.path().join("B/received"))?.count(),
            0
        );
    }
    let source = root.path().join("declined.txt");
    tokio::fs::write(&source, "declined").await?;
    let rejected = a.send_file(&b.local().id, source).await?;
    eventually(|| {
        b.snapshot()
            .unwrap()
            .activities
            .iter()
            .any(|entry| entry.id == rejected)
    })
    .await;
    b.decide_file(&rejected, false)?;
    eventually(|| {
        a.snapshot()
            .unwrap()
            .activities
            .iter()
            .any(|entry| entry.id == rejected && entry.status == "rejected")
    })
    .await;
    a.shutdown().await;
    b.shutdown().await;
    Ok(())
}
