use super::*;
use axum::http::Uri;
use file_trans_core::EngineConfig;
use file_trans_identity::Identity;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

async fn request(url: &Uri, path: &str, method: &str, range: &str) -> (String, String) {
    let host = url.authority().unwrap().as_str();
    let mut socket = tokio::net::TcpStream::connect(host).await.unwrap();
    let message =
        format!("{method} {path} HTTP/1.1\r\nHost: {host}\r\n{range}Connection: close\r\n\r\n");
    socket.write_all(message.as_bytes()).await.unwrap();
    let mut reply = String::new();
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        socket.read_to_string(&mut reply),
    )
    .await
    .unwrap()
    .unwrap();
    let (headers, body) = reply.split_once("\r\n\r\n").unwrap();
    (headers.to_lowercase(), body.into())
}

#[tokio::test]
async fn signed_video_urls_serve_only_the_requested_local_attachment() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let mut config = EngineConfig::new(root.into(), root.join("received"), "预览测试".into());
    config.enable_discovery = false;
    config.listen_address = (Ipv4Addr::LOCALHOST, 0).into();
    let engine = Engine::start(config, Identity::generate().unwrap())
        .await
        .unwrap();
    let peer_id = "a".repeat(64);
    rusqlite::Connection::open(root.join("file-trans.db"))
        .unwrap()
        .execute(
            "INSERT INTO peers (id, name, address) VALUES (?1, '测试设备', '127.0.0.1:0')",
            [&peer_id],
        )
        .unwrap();
    let path = root.join("视频 样例.MP4");
    let contents = "0000ftypisom0123456789";
    std::fs::write(&path, contents).unwrap();
    let id = engine.send_file(&peer_id, path.clone()).await.unwrap();
    let key = TransferKey {
        id,
        peer_id,
        direction: Direction::Outgoing,
    };
    let url: Uri = video_url(engine.clone(), &key)
        .await
        .unwrap()
        .parse()
        .unwrap();
    assert_eq!(url.host(), Some("127.0.0.1"));
    for (method, range, status, expected) in [
        ("GET", "", 200, contents),
        ("HEAD", "", 200, ""),
        ("GET", "Range: bytes=4-7\r\n", 206, "ftyp"),
        ("GET", "Range: bytes=100-\r\n", 416, ""),
    ] {
        let (headers, body) = request(&url, url.path(), method, range).await;
        assert!(
            headers.starts_with(&format!("http/1.1 {status} ")),
            "{headers}"
        );
        if status == 206 {
            assert!(headers.contains("content-range: bytes 4-7/22"));
        }
        assert_eq!(body, expected);
    }
    let (_, key_path) = url.path().trim_start_matches('/').split_once('/').unwrap();
    let wrong_signature = format!("/{}/{key_path}", "00".repeat(32));
    let (headers, _) = request(&url, &wrong_signature, "GET", "").await;
    assert!(headers.starts_with("http/1.1 404 "));
    std::fs::remove_file(path).unwrap();
    let (headers, _) = request(&url, url.path(), "GET", "").await;
    assert!(headers.starts_with("http/1.1 404 "));
    engine.shutdown().await;
}
