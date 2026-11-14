mod control;
mod state;
mod tunnel;

use anyhow::Result;
use axum::{
    extract::{ConnectInfo, State, WebSocketUpgrade},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::get,
    Router,
};
use file_trans_identity::Identity;
use std::{net::SocketAddr, sync::Arc, time::Duration};
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;

#[derive(Clone)]
pub struct RelayLimits {
    pub max_devices: usize,
    pub max_sessions: usize,
    pub max_connections_per_ip: usize,
    pub max_session_bytes: u64,
    pub max_session_duration: Duration,
}

impl Default for RelayLimits {
    fn default() -> Self {
        Self {
            max_devices: 256,
            max_sessions: 64,
            max_connections_per_ip: 32,
            max_session_bytes: 16 * 1024_u64.pow(3),
            max_session_duration: Duration::from_secs(7200),
        }
    }
}

pub async fn serve(
    listener: TcpListener,
    limits: RelayLimits,
    cancel: CancellationToken,
) -> Result<()> {
    let state = Arc::new(state::RelayState::new(
        limits,
        Identity::generate()?,
        cancel.clone(),
    )?);
    let router = Router::new()
        .route("/health", get(|| async { "ok" }))
        .route("/control", get(control_upgrade))
        .route("/tunnel", get(tunnel_upgrade))
        .with_state(state.clone());
    let cleanup = tokio::spawn(async move {
        loop {
            tokio::select! { _ = state.cancel.cancelled() => break, _ = tokio::time::sleep(Duration::from_secs(1)) => state.prune() }
        }
        state.clear();
    });
    let result = axum::serve(
        listener,
        router.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(cancel.clone().cancelled_owned())
    .await;
    cancel.cancel();
    let _ = cleanup.await;
    Ok(result?)
}

async fn control_upgrade(
    ws: WebSocketUpgrade,
    State(state): State<Arc<state::RelayState>>,
    ConnectInfo(address): ConnectInfo<SocketAddr>,
) -> Response {
    let Some(guard) = state.connection(address.ip()) else {
        return StatusCode::TOO_MANY_REQUESTS.into_response();
    };
    ws.max_message_size(file_trans_relay::protocol::MAX_PACKET)
        .max_frame_size(file_trans_relay::protocol::MAX_PACKET)
        .on_upgrade(move |socket| async move {
            let _guard = guard;
            control::handle(state, socket).await;
        })
        .into_response()
}

async fn tunnel_upgrade(
    ws: WebSocketUpgrade,
    State(state): State<Arc<state::RelayState>>,
    ConnectInfo(address): ConnectInfo<SocketAddr>,
) -> Response {
    let Some(guard) = state.connection(address.ip()) else {
        return StatusCode::TOO_MANY_REQUESTS.into_response();
    };
    ws.max_message_size(file_trans_relay::protocol::MAX_PACKET)
        .max_frame_size(file_trans_relay::protocol::MAX_PACKET)
        .on_upgrade(move |socket| tunnel::attach(state, socket, guard))
        .into_response()
}
