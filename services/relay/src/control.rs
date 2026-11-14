use crate::state::RelayState;
use anyhow::Context;
use axum::extract::ws::{Message, WebSocket};
use bytes::Bytes;
use file_trans_identity::{peer_id, BoxedStream, TlsStream};
use file_trans_relay::{
    adapter::byte_stream,
    protocol::{read_json, write_json, ClientMessage, ServerMessage, CONNECT_TIMEOUT},
};
use futures_util::{future, SinkExt, StreamExt, TryStreamExt};
use std::{
    io,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::{io::DuplexStream, sync::mpsc, time::timeout};
use uuid::Uuid;

pub(crate) fn adapt(socket: WebSocket) -> DuplexStream {
    let (sink, source) = socket.split();
    let source = source.map_err(io::Error::other).try_filter_map(|message| {
        future::ready(match message {
            Message::Binary(bytes) => Ok(Some(bytes)),
            Message::Ping(_) | Message::Pong(_) | Message::Close(_) => Ok(None),
            _ => Err(io::Error::other("控制隧道收到非二进制消息")),
        })
    });
    let sink = sink
        .with(|bytes: Bytes| future::ready(Ok::<_, axum::Error>(Message::Binary(bytes))))
        .sink_map_err(io::Error::other);
    byte_stream(source, sink)
}

pub(crate) async fn handle(state: Arc<RelayState>, socket: WebSocket) {
    let result = timeout(
        CONNECT_TIMEOUT,
        state
            .acceptor
            .accept(Box::new(adapt(socket)) as BoxedStream),
    )
    .await;
    let Ok(Ok(stream)) = result else { return };
    let stream = TlsStream::Server(stream);
    let Ok(id) = peer_id(&stream) else { return };
    let generation = Uuid::new_v4().to_string();
    let (sender, mut messages) = mpsc::channel(32);
    if state
        .register(&id, generation.clone(), sender.clone())
        .is_err()
    {
        return;
    }
    let (mut reader, mut writer) = tokio::io::split(stream);
    let incoming = async {
        let mut window = Instant::now();
        let mut count = 0;
        loop {
            let message = timeout(
                Duration::from_secs(45),
                read_json::<ClientMessage>(&mut reader),
            )
            .await??;
            if window.elapsed() > Duration::from_secs(60) {
                window = Instant::now();
                count = 0;
            }
            count += 1;
            anyhow::ensure!(count <= 120, "控制请求过于频繁");
            let request_id = match &message {
                ClientMessage::Open { request_id, .. } | ClientMessage::Invite { request_id } => {
                    request_id.clone()
                }
                _ => String::new(),
            };
            let response = match state.dispatch(&id, message) {
                Ok(response) => response,
                Err(error) => Some(ServerMessage::Error {
                    request_id,
                    message: error.to_string(),
                }),
            };
            if let Some(response) = response {
                sender.try_send(response).context("控制响应队列已满")?;
            }
        }
        #[allow(unreachable_code)]
        Ok::<(), anyhow::Error>(())
    };
    let outgoing = async {
        timeout(
            CONNECT_TIMEOUT,
            write_json(
                &mut writer,
                &ServerMessage::Registered {
                    device_id: id.clone(),
                    version: file_trans_relay::protocol::VERSION,
                },
            ),
        )
        .await??;
        while let Some(message) = messages.recv().await {
            timeout(CONNECT_TIMEOUT, write_json(&mut writer, &message)).await??;
        }
        Ok::<(), anyhow::Error>(())
    };
    tokio::select! { _ = state.cancel.cancelled() => {}, _ = incoming => {}, _ = outgoing => {} }
    state.disconnect(&id, &generation);
}
