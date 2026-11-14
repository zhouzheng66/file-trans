#[path = "support/resume.rs"]
mod support;
use anyhow::Result;
use file_trans_core::{
    model::{NetworkMode, Route},
    transfers::Direction,
    Engine,
};
use file_trans_identity::{connect, Identity};
use file_trans_protocol::{
    read_message, write_message, FileIdentity, Message, ResumeState, FOLDER_VERSION,
};
use std::{collections::BTreeMap, path::Path, sync::Arc};
use support::*;
use tempfile::TempDir;
use tokio::io::AsyncReadExt;

async fn tree(root: &Path) -> Result<BTreeMap<String, String>> {
    use sha2::{Digest, Sha256};
    let mut entries = BTreeMap::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(path) = pending.pop() {
        let mut reader = tokio::fs::read_dir(path).await?;
        while let Some(item) = reader.next_entry().await? {
            let relative = item
                .path()
                .strip_prefix(root)?
                .to_string_lossy()
                .replace('\\', "/");
            if item.file_type().await?.is_dir() {
                entries.insert(relative, "directory".into());
                pending.push(item.path());
            } else {
                let mut file = tokio::fs::File::open(item.path()).await?;
                let mut hash = Sha256::new();
                let mut bytes = vec![0; 256 * 1024];
                loop {
                    let length = file.read(&mut bytes).await?;
                    if length == 0 {
                        break;
                    }
                    hash.update(&bytes[..length]);
                }
                entries.insert(relative, hex::encode(hash.finalize()));
            }
        }
    }
    Ok(entries)
}

async fn offer_folder(a: &Arc<Engine>, b: &Engine, root: &Path) -> Result<String> {
    let id = a.send_path(&b.local().id, root.into()).await?;
    eventually(|| {
        let sent = activity(a, &id, "outgoing");
        assert_ne!(sent.status, "failed", "folder preparation: {sent:?}");
        b.snapshot()
            .unwrap()
            .activities
            .iter()
            .any(|task| task.id == id && task.status == "awaiting_accept")
    })
    .await;
    assert_eq!(activity(b, &id, "incoming").kind, "folder");
    b.decide_file_from(&a.local().id, &id, true)?;
    Ok(id)
}
async fn wait_task(
    a: &Engine,
    b: &Engine,
    id: &str,
    predicate: impl Fn(&file_trans_core::model::Activity) -> bool,
) {
    let result = tokio::time::timeout(std::time::Duration::from_secs(25), async {
        loop {
            let sent = activity(a, id, "outgoing");
            assert_ne!(
                sent.status,
                "failed",
                "source={sent:?}; receiver={:?}",
                activity(b, id, "incoming")
            );
            if predicate(&sent) {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await;
    assert!(
        result.is_ok(),
        "source={:?}; receiver={:?}",
        activity(a, id, "outgoing"),
        activity(b, id, "incoming")
    );
}
async fn done(a: &Engine, b: &Engine, id: &str) {
    wait_task(a, b, id, |task| {
        ["completed", "failed", "rejected"].contains(&task.status.as_str())
    })
    .await;
    assert_eq!(
        activity(a, id, "outgoing").status,
        "completed",
        "{:?}",
        activity(a, id, "outgoing").error
    );
    assert_eq!(
        activity(b, id, "incoming").status,
        "completed",
        "{:?}",
        activity(b, id, "incoming").error
    );
}

#[tokio::test]
async fn preserves_tree_empty_folders_and_no_clobber_and_recovers_completion() -> Result<()> {
    let root = TempDir::new()?;
    let a_identity = Identity::generate()?;
    let b_identity = Identity::generate()?;
    let a = node(root.path(), "A", a_identity.clone(), "", None).await;
    let b = node(root.path(), "B", b_identity.clone(), "", None).await;
    pair(&a, &b).await?;
    let folder = root.path().join("资料.v1");
    tokio::fs::create_dir_all(folder.join("子目录/空目录")).await?;
    tokio::fs::write(folder.join("子目录/你好.bin"), [0, 1, 128, 255]).await?;
    tokio::fs::write(folder.join("空文件"), []).await?;
    tokio::fs::write(folder.join(".hidden"), b"hidden").await?;
    let expected = tree(&folder).await?;
    let occupied = root.path().join("B/received/资料.v1");
    tokio::fs::create_dir(&occupied).await?;
    tokio::fs::write(occupied.join("original"), b"untouched").await?;
    let id = offer_folder(&a, &b, &folder).await?;
    done(&a, &b, &id).await;
    let received = activity(&b, &id, "incoming");
    let saved = Path::new(received.path.as_ref().unwrap());
    assert_eq!(saved.file_name().unwrap(), "资料.v1 (1)");
    assert_eq!(tree(saved).await?, expected);
    assert_eq!(
        tokio::fs::read(occupied.join("original")).await?,
        b"untouched"
    );
    eventually(|| {
        std::fs::read_dir(root.path().join("A/folder-snapshots"))
            .unwrap()
            .next()
            .is_none()
    })
    .await;
    // 模拟目录已提交，但完成状态尚未写入数据库的崩溃窗口。
    let db = rusqlite::Connection::open(root.path().join("B/file-trans.db"))?;
    db.execute("UPDATE file_transfers SET data=json_set(data,'$.committing',json('true'),'$.activity.status','waiting_connection') WHERE id=?1", [&id])?;
    let record: String = db.query_row(
        "SELECT data FROM file_transfers WHERE id=?1",
        [&id],
        |row| row.get(0),
    )?;
    let record: serde_json::Value = serde_json::from_str(&record)?;
    b.shutdown().await;
    let b = node(root.path(), "B", b_identity, "", None).await;
    for _ in 0..2 {
        let tcp = tokio::net::TcpStream::connect(b.local_address()).await?;
        let mut stream = connect(tcp, &a_identity, Some(&b.local().id)).await?;
        write_message(
            &mut stream,
            &Message::FolderOffer {
                version: FOLDER_VERSION,
                id: id.clone(),
                folder: FileIdentity {
                    name: "资料.v1".into(),
                    size: received.size,
                    sha256: record["sha256"].as_str().unwrap().into(),
                },
            },
        )
        .await?;
        loop {
            match read_message(&mut stream).await? {
                Message::TransferKeepAlive => {}
                Message::TransferReply {
                    result: ResumeState::Completed,
                    ..
                } => break,
                other => panic!("unexpected {other:?}"),
            }
        }
    }
    assert_eq!(tree(saved).await?, expected);
    assert!(!root.path().join("B/received/资料.v1 (2)").exists());
    let empty = root.path().join("空文件夹");
    tokio::fs::create_dir(&empty).await?;
    // 对方重启后更新测试地址，验证空根目录。
    remember(root.path(), "A", &b, b.local_address());
    let id = offer_folder(&a, &b, &empty).await?;
    done(&a, &b, &id).await;
    assert!(tree(Path::new(
        activity(&b, &id, "incoming").path.as_ref().unwrap()
    ))
    .await?
    .is_empty());
    a.shutdown().await;
    b.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn folder_snapshot_survives_pause_restart_and_live_lan_relay_migration() -> Result<()> {
    let root = TempDir::new()?;
    let service = Service::start().await;
    let ai = Identity::generate()?;
    let bi = Identity::generate()?;
    let a = node(root.path(), "A", ai.clone(), &service.url, None).await;
    let b = node(root.path(), "B", bi.clone(), &service.url, None).await;
    let aa = a.local_address();
    let ba = b.local_address();
    pair(&a, &b).await?;
    let proxy = Proxy::start(ba).await;
    remember(root.path(), "A", &b, proxy.address);
    let folder = root.path().join("project");
    tokio::fs::create_dir_all(folder.join("empty/nested")).await?;
    write_file(&folder.join("data.bin"), 64 * 1024 * 1024).await?;
    let expected = tree(&folder).await?;
    let id = offer_folder(&a, &b, &folder).await?;
    eventually(|| activity(&a, &id, "outgoing").transferred >= 4 * 1024 * 1024).await;
    a.pause_transfer(key(&a, &b, &id, Direction::Outgoing))
        .await?;
    eventually(|| proxy.active.load(std::sync::atomic::Ordering::SeqCst) == 0).await;
    let offset = activity(&b, &id, "incoming").transferred;
    assert!(offset > 0);
    // 快照准备好后源目录变化不会拼接不同版本，重启也继续使用同一份快照。
    tokio::fs::remove_dir_all(&folder).await?;
    a.shutdown().await;
    b.shutdown().await;
    let a = node(root.path(), "A", ai, &service.url, Some(aa)).await;
    let b = node(root.path(), "B", bi, &service.url, Some(ba)).await;
    assert_eq!(activity(&a, &id, "outgoing").status, "paused");
    eventually(|| a.relay_status().state == "online" && b.relay_status().state == "online").await;
    let mut settings = a.network_settings();
    settings.mode = NetworkMode::Internet;
    a.set_network_settings(settings).await?;
    a.resume_transfer(key(&a, &b, &id, Direction::Outgoing))
        .await?;
    wait_task(&a, &b, &id, |t| {
        t.route == Some(Route::Relay) && t.resumed_from >= offset
    })
    .await;
    let mut settings = a.network_settings();
    settings.mode = NetworkMode::Lan;
    a.set_network_settings(settings).await?;
    done(&a, &b, &id).await;
    let task = activity(&a, &id, "outgoing");
    assert_eq!(task.route, Some(Route::Lan));
    assert!(task.resumed_from >= offset);
    assert_eq!(
        tree(Path::new(
            activity(&b, &id, "incoming").path.as_ref().unwrap()
        ))
        .await?,
        expected
    );
    a.shutdown().await;
    b.shutdown().await;
    proxy.stop().await;
    service.stop().await;
    Ok(())
}

#[tokio::test]
async fn folder_network_break_resumes_and_cancel_cleans_only_managed_data() -> Result<()> {
    let root = TempDir::new()?;
    let a = node(root.path(), "A", Identity::generate()?, "", None).await;
    let b = node(root.path(), "B", Identity::generate()?, "", None).await;
    pair(&a, &b).await?;
    let proxy = Proxy::start(b.local_address()).await;
    let address = proxy.address;
    remember(root.path(), "A", &b, address);
    let folder = root.path().join("folder");
    tokio::fs::create_dir(&folder).await?;
    write_file(&folder.join("a.bin"), 24 * 1024 * 1024).await?;
    let expected = tree(&folder).await?;
    let id = offer_folder(&a, &b, &folder).await?;
    eventually(|| activity(&a, &id, "outgoing").transferred >= 4 * 1024 * 1024).await;
    proxy.stop().await;
    eventually(|| activity(&a, &id, "outgoing").status == "waiting_retry").await;
    let offset = activity(&b, &id, "incoming").transferred;
    let proxy = Proxy::on(address, b.local_address()).await;
    done(&a, &b, &id).await;
    assert!(activity(&a, &id, "outgoing").resumed_from >= offset);
    let completed = activity(&b, &id, "incoming").path.unwrap();
    assert_eq!(tree(Path::new(&completed)).await?, expected);
    let id = offer_folder(&a, &b, &folder).await?;
    eventually(|| activity(&a, &id, "outgoing").transferred >= 4 * 1024 * 1024).await;
    b.cancel_transfer(key(&b, &a, &id, Direction::Incoming))
        .await?;
    eventually(|| {
        ["cancelled", "rejected"].contains(&activity(&a, &id, "outgoing").status.as_str())
    })
    .await;
    eventually(|| {
        std::fs::read_dir(root.path().join("A/folder-snapshots"))
            .unwrap()
            .next()
            .is_none()
    })
    .await;
    assert_eq!(tree(Path::new(&completed)).await?, expected);
    let staging = root
        .path()
        .join("B/received/.file-trans-partials")
        .join(&b.local().id);
    assert!(std::fs::read_dir(staging)?.next().is_none());
    a.shutdown().await;
    b.shutdown().await;
    proxy.stop().await;
    Ok(())
}
