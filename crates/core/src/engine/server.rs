use super::{Engine, IO_TIMEOUT};
use crate::model::Route;
use anyhow::{ensure, Context, Result};
use file_trans_identity::{peer_id, BoxedStream, TlsAcceptor, TlsStream};
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
                if !engine.network_settings().mode.allows(Route::Lan) { continue; }
                let Ok(permit) = slots.clone().try_acquire_owned() else { continue };
                let acceptor = acceptor.clone();
                let engine = Arc::clone(&engine);
                tasks.spawn(async move {
                    let _permit = permit;
                    let _ = tcp.set_nodelay(true);
                    let result = tokio::select! { _ = engine.cancel.cancelled() => return, result = timeout(IO_TIMEOUT, acceptor.accept(Box::new(tcp) as BoxedStream)) => result };
                    let Ok(Ok(stream)) = result else { return };
                    serve_stream(&engine, TlsStream::Server(stream), Some(address), Route::Lan).await;
                });
            }
        }
    }
    while tasks.join_next().await.is_some() {}
}

pub(super) async fn serve_stream(
    engine: &Arc<Engine>,
    mut stream: TlsStream,
    address: Option<SocketAddr>,
    route: Route,
) {
    let result = handle(engine, &mut stream, address, route).await;
    if let Err(error) = result {
        let _ = timeout(
            IO_TIMEOUT,
            write_message(
                &mut stream,
                &Message::Error {
                    message: format!("{error:#}"),
                },
            ),
        )
        .await;
    }
}

async fn handle(
    engine: &Arc<Engine>,
    stream: &mut TlsStream,
    address: Option<SocketAddr>,
    route: Route,
) -> Result<()> {
    ensure!(
        engine.network_settings().mode.allows(route),
        "当前网络模式不接受此通道的新连接"
    );
    let message = tokio::select! {
        _ = engine.cancel.cancelled() => return Ok(()),
        result = timeout(IO_TIMEOUT, read_message(stream)) => result.context("等待请求超时")??,
    };
    if let Message::PairRequest { version, device } = message {
        ensure!(version == VERSION, "协议版本不兼容");
        return tokio::select! { _ = engine.cancel.cancelled() => Ok(()), result = engine.receive_pair(stream, address, device) => result };
    }
    let remote_id = peer_id(stream)?;
    if let Message::TransferOffer { version, id, file } = message {
        ensure!(
            version == file_trans_protocol::FILE_VERSION,
            "文件续传协议版本不兼容"
        );
        return engine
            .receive_resumable(
                stream,
                &remote_id,
                id,
                crate::transfers::model::PayloadIdentity {
                    file,
                    folder: false,
                },
                route,
            )
            .await;
    }
    if let Message::FolderOffer {
        version,
        id,
        folder,
    } = message
    {
        ensure!(
            version == file_trans_protocol::FOLDER_VERSION,
            "文件夹协议版本不兼容，请更新两端客户端"
        );
        return engine
            .receive_resumable(
                stream,
                &remote_id,
                id,
                crate::transfers::model::PayloadIdentity {
                    file: folder,
                    folder: true,
                },
                route,
            )
            .await;
    }
    let peer = engine.store.peer(&remote_id)?;
    let operation = async {
        if let Message::TransferSignal {
            version,
            id,
            direction,
            paused,
            cancelled,
            sequence,
        } = message
        {
            ensure!(
                version == file_trans_protocol::FILE_VERSION,
                "文件控制协议版本不兼容"
            );
            return engine
                .receive_transfer_signal(
                    stream,
                    crate::transfers::TransferKey {
                        id,
                        peer_id: remote_id,
                        direction,
                    },
                    paused,
                    cancelled,
                    sequence,
                )
                .await;
        }
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
                engine
                    .receive_text(stream, &peer.id, id, text, route)
                    .await?;
            }
            Message::FileOffer {
                version,
                id,
                name,
                size,
            } => {
                ensure!(version == VERSION, "协议版本不兼容");
                engine
                    .receive_file(stream, &peer.id, id, name, size, route)
                    .await?;
            }
            _ => anyhow::bail!("无效业务请求"),
        }
        Ok(())
    };
    tokio::select! { _ = engine.cancel.cancelled() => Ok(()), result = operation => result }
}
