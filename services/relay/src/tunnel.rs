use crate::state::{ConnectionGuard, Leg, RelayState};
use anyhow::{ensure, Context, Result};
use axum::extract::ws::{Message, WebSocket};
use file_trans_relay::protocol::{Attach, CONNECT_TIMEOUT, MAX_CONTROL};
use futures_util::{SinkExt, StreamExt};
use std::sync::Arc;
use tokio::time::timeout;

pub(crate) async fn attach(state: Arc<RelayState>, mut socket: WebSocket, guard: ConnectionGuard) {
    let request = timeout(CONNECT_TIMEOUT, async {
        let Message::Text(text) = socket.next().await.context("接入已关闭")?? else {
            anyhow::bail!("无效接入消息")
        };
        ensure!(text.len() <= MAX_CONTROL, "接入消息过大");
        Ok::<Attach, anyhow::Error>(serde_json::from_str(&text)?)
    })
    .await;
    let Ok(Ok(request)) = request else { return };
    if let Ok(Some((session, mut first, mut second))) = state.attach(
        &request.session_id,
        &request.ticket,
        Leg {
            socket,
            _guard: guard,
        },
    ) {
        let forward = async {
            first.socket.send(Message::Text("ready".into())).await?;
            second.socket.send(Message::Text("ready".into())).await?;
            let (mut a_tx, mut a_rx) = first.socket.split();
            let (mut b_tx, mut b_rx) = second.socket.split();
            // 两个方向共享总字节预算，不缓存整个消息队列。
            let budget = std::sync::atomic::AtomicU64::new(state.limits.max_session_bytes);
            let a_to_b = copy(&mut a_rx, &mut b_tx, &budget);
            let b_to_a = copy(&mut b_rx, &mut a_tx, &budget);
            tokio::select! { result = a_to_b => result, result = b_to_a => result }
        };
        tokio::select! { _ = state.cancel.cancelled() => {}, _ = timeout(state.limits.max_session_duration, forward) => {} }
        state.finish(&session);
    }
}

async fn copy<R, W>(
    source: &mut R,
    sink: &mut W,
    budget: &std::sync::atomic::AtomicU64,
) -> Result<()>
where
    R: futures_util::Stream<Item = Result<Message, axum::Error>> + Unpin,
    W: futures_util::Sink<Message, Error = axum::Error> + Unpin,
{
    while let Some(message) = source.next().await {
        match message? {
            Message::Binary(bytes) => {
                consume(budget, bytes.len() as u64)?;
                timeout(CONNECT_TIMEOUT, sink.send(Message::Binary(bytes))).await??;
            }
            Message::Ping(_) | Message::Pong(_) => {}
            Message::Close(_) => break,
            _ => anyhow::bail!("数据隧道只允许二进制消息"),
        }
    }
    Ok(())
}

fn consume(budget: &std::sync::atomic::AtomicU64, bytes: u64) -> Result<()> {
    use std::sync::atomic::Ordering::Relaxed;
    let mut remaining = budget.load(Relaxed);
    loop {
        let next = remaining
            .checked_sub(bytes)
            .context("中继会话超过字节配额")?;
        match budget.compare_exchange_weak(remaining, next, Relaxed, Relaxed) {
            Ok(_) => return Ok(()),
            Err(current) => remaining = current,
        }
    }
}
