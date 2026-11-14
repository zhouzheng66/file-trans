#[path = "support/resume.rs"]
mod support;
use anyhow::Result;
use file_trans_identity::{connect, Identity, TlsStream};
use file_trans_protocol::{
    read_message, write_chunk, write_message, Direction, FileIdentity, Message, ResumeState,
    FILE_VERSION,
};
use sha2::{Digest, Sha256};
use support::*;
use tempfile::TempDir;
use tokio::net::TcpStream;
use uuid::Uuid;

async fn read(stream: &mut TlsStream) -> Result<Message> {
    loop {
        let message = read_message(stream).await?;
        if !matches!(message, Message::TransferKeepAlive) {
            return Ok(message);
        }
    }
}

#[tokio::test]
async fn lost_completion_confirmation_and_commit_recovery_do_not_duplicate_files() -> Result<()> {
    let root = TempDir::new()?;
    let identity = Identity::generate()?;
    let b_identity = Identity::generate()?;
    let a = node(root.path(), "A", identity.clone(), "", None).await;
    let b = node(root.path(), "B", b_identity.clone(), "", None).await;
    pair(&a, &b).await?;
    let id = Uuid::new_v4().to_string();
    let data = b"raw binary\0\xff";
    let hash = hex::encode(Sha256::digest(data));
    let identity_file = FileIdentity {
        name: "complete.bin".into(),
        size: data.len() as u64,
        sha256: hash.clone(),
    };
    let mut stream = connect(
        TcpStream::connect(b.local_address()).await?,
        &identity,
        Some(&b.local().id),
    )
    .await?;
    write_message(
        &mut stream,
        &Message::TransferOffer {
            version: FILE_VERSION,
            id: id.clone(),
            file: identity_file.clone(),
        },
    )
    .await?;
    eventually(|| {
        b.snapshot()
            .unwrap()
            .activities
            .iter()
            .any(|task| task.id == id)
    })
    .await;
    b.decide_file(&id, true)?;
    assert!(matches!(
        read(&mut stream).await?,
        Message::TransferReply {
            result: ResumeState::Ready { offset: 0, .. },
            ..
        }
    ));
    write_message(
        &mut stream,
        &Message::TransferChunk {
            offset: 0,
            length: data.len() as u32,
            sha256: hash.clone(),
        },
    )
    .await?;
    write_chunk(&mut stream, data).await?;
    write_message(
        &mut stream,
        &Message::TransferCheckpoint {
            offset: data.len() as u64,
            prefix_sha256: hash,
        },
    )
    .await?;
    assert!(matches!(
        read(&mut stream).await?,
        Message::TransferAck { .. }
    ));
    write_message(&mut stream, &Message::TransferFinish { id: id.clone() }).await?;
    drop(stream);
    eventually(|| activity(&b, &id, "incoming").status == "completed").await;
    let b_address = b.local_address();
    b.shutdown().await;
    // 模拟文件已提交、SQLite 最后一步完成记录尚未提交的崩溃窗口。
    let db = rusqlite::Connection::open(root.path().join("B/file-trans.db"))?;
    let json: String = db.query_row(
        "SELECT data FROM file_transfers WHERE id=?1",
        [&id],
        |row| row.get(0),
    )?;
    let mut value: serde_json::Value = serde_json::from_str(&json)?;
    value["committing"] = true.into();
    value["activity"]["status"] = "verifying".into();
    value["activity"]["path"] = serde_json::Value::Null;
    db.execute(
        "UPDATE file_transfers SET data=?1 WHERE id=?2",
        rusqlite::params![value.to_string(), id],
    )?;
    drop(db);
    let b = node(root.path(), "B", b_identity, "", Some(b_address)).await;
    for _ in 0..2 {
        let mut stream = connect(
            TcpStream::connect(b.local_address()).await?,
            &identity,
            Some(&b.local().id),
        )
        .await?;
        write_message(
            &mut stream,
            &Message::TransferOffer {
                version: FILE_VERSION,
                id: id.clone(),
                file: identity_file.clone(),
            },
        )
        .await?;
        assert!(matches!(
            read(&mut stream).await?,
            Message::TransferReply {
                result: ResumeState::Completed,
                ..
            }
        ));
    }
    assert_eq!(
        tokio::fs::read(root.path().join("B/received/complete.bin")).await?,
        data
    );
    assert!(!root.path().join("B/received/complete (1).bin").exists());
    a.shutdown().await;
    b.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn wrong_chunk_offsets_are_rejected_and_cancel_cleans_partial() -> Result<()> {
    let root = TempDir::new()?;
    let identity = Identity::generate()?;
    let a = node(root.path(), "A", identity.clone(), "", None).await;
    let b = node(root.path(), "B", Identity::generate()?, "", None).await;
    pair(&a, &b).await?;
    let id = Uuid::new_v4().to_string();
    let mut stream = connect(
        TcpStream::connect(b.local_address()).await?,
        &identity,
        Some(&b.local().id),
    )
    .await?;
    write_message(
        &mut stream,
        &Message::TransferOffer {
            version: FILE_VERSION,
            id: id.clone(),
            file: FileIdentity {
                name: "invalid.bin".into(),
                size: 3,
                sha256: hex::encode(Sha256::digest(b"abc")),
            },
        },
    )
    .await?;
    eventually(|| {
        b.snapshot()
            .unwrap()
            .activities
            .iter()
            .any(|task| task.id == id)
    })
    .await;
    b.decide_file(&id, true)?;
    assert!(matches!(
        read(&mut stream).await?,
        Message::TransferReply {
            result: ResumeState::Ready { .. },
            ..
        }
    ));
    write_message(
        &mut stream,
        &Message::TransferChunk {
            offset: 1,
            length: 3,
            sha256: hex::encode(Sha256::digest(b"abc")),
        },
    )
    .await?;
    assert!(matches!(
        read(&mut stream).await?,
        Message::TransferReply {
            result: ResumeState::Failed { .. },
            ..
        }
    ));
    b.cancel_transfer(key(&b, &a, &id, Direction::Incoming))
        .await?;
    assert!(!root.path().join("B/received/invalid.bin").exists());
    a.shutdown().await;
    b.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn stale_pause_notifications_do_not_override_newer_resume() -> Result<()> {
    let root = TempDir::new()?;
    let identity = Identity::generate()?;
    let a = node(root.path(), "A", identity.clone(), "", None).await;
    let b = node(root.path(), "B", Identity::generate()?, "", None).await;
    pair(&a, &b).await?;
    let id = Uuid::new_v4().to_string();
    let mut stream = connect(
        TcpStream::connect(b.local_address()).await?,
        &identity,
        Some(&b.local().id),
    )
    .await?;
    write_message(
        &mut stream,
        &Message::TransferOffer {
            version: FILE_VERSION,
            id: id.clone(),
            file: FileIdentity {
                name: "control.bin".into(),
                size: 3,
                sha256: hex::encode(Sha256::digest(b"abc")),
            },
        },
    )
    .await?;
    eventually(|| {
        b.snapshot()
            .unwrap()
            .activities
            .iter()
            .any(|task| task.id == id)
    })
    .await;
    b.decide_file(&id, true)?;
    assert!(matches!(
        read(&mut stream).await?,
        Message::TransferReply {
            result: ResumeState::Ready { .. },
            ..
        }
    ));
    drop(stream);
    eventually(|| activity(&b, &id, "incoming").status == "waiting_connection").await;
    for (sequence, paused) in [(2, false), (1, true), (3, true), (4, false)] {
        let mut stream = connect(
            TcpStream::connect(b.local_address()).await?,
            &identity,
            Some(&b.local().id),
        )
        .await?;
        write_message(
            &mut stream,
            &Message::TransferSignal {
                version: FILE_VERSION,
                id: id.clone(),
                direction: Direction::Incoming,
                paused,
                cancelled: false,
                sequence,
            },
        )
        .await?;
        assert!(matches!(read(&mut stream).await?, Message::Ack { .. }));
        assert_eq!(
            activity(&b, &id, "incoming").status == "paused_by_peer",
            sequence == 3
        );
    }
    b.cancel_transfer(key(&b, &a, &id, Direction::Incoming))
        .await?;
    a.shutdown().await;
    b.shutdown().await;
    Ok(())
}
