use crate::{
    adapter::byte_stream,
    protocol::{Attach, CONNECT_TIMEOUT, MAX_PACKET},
};
use anyhow::{ensure, Context, Result};
use bytes::Bytes;
use futures_util::{future, SinkExt, StreamExt, TryStreamExt};
use std::io;
use tokio::{io::DuplexStream, net::TcpStream, time::timeout};
use tokio_tungstenite::{
    connect_async_with_config,
    tungstenite::{protocol::WebSocketConfig, Message},
    MaybeTlsStream, WebSocketStream,
};
use url::{Host, Url};

type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

pub fn validate_url(address: &str) -> Result<String> {
    ensure!(address.len() <= 2048, "中继地址过长");
    let url = Url::parse(address.trim())
        .context("请输入有效中继地址，例如 wss://transfer.example.com")?;
    let loopback = match url.host() {
        Some(Host::Ipv4(ip)) => ip.is_loopback(),
        Some(Host::Ipv6(ip)) => ip.is_loopback(),
        _ => false,
    };
    ensure!(
        url.scheme() == "wss" || (url.scheme() == "ws" && loopback),
        "公网中继必须使用 wss://；ws:// 仅允许本机回环 IP"
    );
    ensure!(
        url.host().is_some()
            && url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.fragment().is_none()
            && url.path() == "/",
        "中继地址只能包含协议、主机和可选端口"
    );
    Ok(url.as_str().trim_end_matches('/').to_owned())
}

async fn socket(base: &str, path: &str) -> Result<Socket> {
    let config = WebSocketConfig::default()
        .max_message_size(Some(MAX_PACKET))
        .max_frame_size(Some(MAX_PACKET));
    let (socket, _) = timeout(
        CONNECT_TIMEOUT,
        connect_async_with_config(format!("{base}/{path}"), Some(config), true),
    )
    .await
    .context("连接中继服务超时")??;
    Ok(socket)
}

pub async fn control_stream(base: &str) -> Result<DuplexStream> {
    Ok(adapt(socket(base, "control").await?))
}

pub async fn tunnel_stream(base: &str, session_id: String, ticket: String) -> Result<DuplexStream> {
    timeout(CONNECT_TIMEOUT, async {
        let mut socket = socket(base, "tunnel").await?;
        socket
            .send(Message::Text(
                serde_json::to_string(&Attach { session_id, ticket })?.into(),
            ))
            .await?;
        let response = socket.next().await.context("中继隧道已关闭")??;
        ensure!(
            matches!(response, Message::Text(text) if text == "ready"),
            "中继未能建立双端隧道"
        );
        Ok(adapt(socket))
    })
    .await
    .context("等待对端接入中继超时，设备可能离线或拒绝了连接")?
}

fn adapt(socket: Socket) -> DuplexStream {
    let (sink, source) = socket.split();
    let source = source.map_err(io::Error::other).try_filter_map(|message| {
        future::ready(match message {
            Message::Binary(bytes) => Ok(Some(bytes)),
            Message::Ping(_) | Message::Pong(_) | Message::Close(_) => Ok(None),
            _ => Err(io::Error::other("隧道收到非二进制数据")),
        })
    });
    let sink = sink
        .with(|bytes: Bytes| {
            future::ready(Ok::<_, tokio_tungstenite::tungstenite::Error>(
                Message::Binary(bytes),
            ))
        })
        .sink_map_err(io::Error::other);
    byte_stream(source, sink)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn requires_secure_public_urls_and_rejects_embedded_credentials() {
        assert!(validate_url("wss://transfer.example.com").is_ok());
        assert!(validate_url("ws://127.0.0.1:8787").is_ok());
        for value in [
            "ws://example.com",
            "ws://192.168.1.2",
            "wss://user:secret@example.com",
            "wss://example.com/x",
            "wss://example.com?secret=x",
        ] {
            assert!(validate_url(value).is_err(), "{value}");
        }
    }
}
