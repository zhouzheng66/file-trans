use anyhow::{Context, Result};
use file_trans_relay_server::{serve, RelayLimits};
use std::time::Duration;
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;

#[tokio::main]
async fn main() -> Result<()> {
    let address = std::env::var("RELAY_LISTEN").unwrap_or_else(|_| "127.0.0.1:8787".into());
    let limits = RelayLimits {
        max_devices: value("RELAY_MAX_DEVICES", 256)?,
        max_sessions: value("RELAY_MAX_SESSIONS", 64)?,
        max_connections_per_ip: value("RELAY_MAX_CONNECTIONS_PER_IP", 32)?,
        max_session_bytes: value("RELAY_MAX_SESSION_BYTES", 16 * 1024_u64.pow(3))?,
        max_session_duration: Duration::from_secs(value("RELAY_MAX_SESSION_SECONDS", 7200)?),
    };
    let listener = TcpListener::bind(&address).await.context("中继监听失败")?;
    println!(
        "File Trans 中继监听 {}；公网部署请使用 HTTPS/WSS 反向代理",
        listener.local_addr()?
    );
    let cancel = CancellationToken::new();
    let shutdown = cancel.clone();
    tokio::spawn(async move {
        #[cfg(unix)]
        {
            let mut term =
                tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                    .expect("无法监听终止信号");
            tokio::select! { _ = tokio::signal::ctrl_c() => {}, _ = term.recv() => {} }
        }
        #[cfg(not(unix))]
        {
            let _ = tokio::signal::ctrl_c().await;
        }
        shutdown.cancel();
    });
    serve(listener, limits, cancel).await
}

fn value<T: std::str::FromStr>(key: &str, default: T) -> Result<T> {
    match std::env::var(key) {
        Ok(value) => value
            .parse()
            .map_err(|_| anyhow::anyhow!("环境变量 {key} 无效")),
        Err(std::env::VarError::NotPresent) => Ok(default),
        Err(error) => Err(error.into()),
    }
}
