use super::{Engine, IO_TIMEOUT};
use anyhow::{ensure, Context, Result};
use file_trans_identity::{peer_id, TlsAcceptor, TlsStream};
use file_trans_protocol::{read_message, write_message, Message, VERSION};
use std::{net::SocketAddr, sync::Arc};
use tokio::{net::TcpListener, sync::Semaphore, task::JoinSet, time::timeout};

pub(super) async fn listen(engine: Arc<Engine>, listener: TcpListener, acceptor: TlsAcceptor) {
    let slots = Arc::new(Semaphore::new(16));
    let mut tasks = JoinSet::new();
    loop {
        tokio::select! {
            _ = engine.cancel.cancelled() => break,
            Some(_) = tasks.join_next(), if !tasks.is_empty() => {},
            accepted = listener.accept() => {
                let Ok((tcp, address)) = accepted else { break };
                let Ok(permit) = slots.clone().try_acquire_owned() else { continue };
                let acceptor = acceptor.clone();
                let engine = Arc::clone(&engine);
                tasks.spawn(async move {
                    let _permit = permit;
                    let _ = tcp.set_nodelay(true);
                    let Ok(Ok(stream)) = timeout(IO_TIMEOUT, acceptor.accept(tcp)).await else { return };
                    let mut stream = TlsStream::Server(stream);
                    let result = tokio::select! {
                        _ = engine.cancel.cancelled() => return,
                        result = handle(&engine, &mut stream, address) => result,
                    };
                    if let Err(error) = result {
                        let _ = timeout(IO_TIMEOUT, write_message(&mut stream, &Message::Error { message: format!("{error:#}") })).await;
                    }
                });
            }
        }
    }
    tasks.abort_all();
    while tasks.join_next().await.is_some() {}
}

async fn handle(engine: &Engine, stream: &mut TlsStream, address: SocketAddr) -> Result<()> {
    let message = timeout(IO_TIMEOUT, read_message(stream))
        .await
        .context("等待请求超时")??;
    if let Message::PairRequest { version, device } = message {
        ensure!(version == VERSION, "协议版本不兼容");
        return engine.receive_pair(stream, address, device).await;
    }
    let peer = engine.store.peer(&peer_id(stream)?)?;
    match message {
        Message::Ping { version } => {
            ensure!(version == VERSION, "协议版本不兼容");
            timeout(
                IO_TIMEOUT,
                write_message(
                    stream,
                    &Message::Pong {
                        device: engine.local.clone(),
                    },
                ),
            )
            .await??;
        }
        Message::Text { version, id, text } => {
            ensure!(version == VERSION, "协议版本不兼容");
            engine.receive_text(stream, &peer.id, id, text).await?;
        }
        Message::FileOffer {
            version,
            id,
            name,
            size,
        } => {
            ensure!(version == VERSION, "协议版本不兼容");
            engine
                .receive_file(stream, &peer.id, id, name, size)
                .await?;
        }
        _ => anyhow::bail!("无效业务请求"),
    }
    Ok(())
}
