use super::Engine;
use crate::model::{NetworkMode, Route};
use anyhow::{Context, Result};
use file_trans_identity::{connect, TlsStream};
use file_trans_relay::protocol::Target;
use futures_util::{stream, StreamExt};
use std::{net::SocketAddr, time::Duration};
use tokio::{net::TcpStream, time::timeout};

pub(crate) struct Connection {
    pub stream: TlsStream,
    pub route: Route,
}

impl Engine {
    pub(crate) async fn open_address(
        &self,
        address: SocketAddr,
        expected: Option<&str>,
    ) -> Result<TlsStream> {
        timeout(Duration::from_secs(5), async {
            let tcp = TcpStream::connect(address).await?;
            tcp.set_nodelay(true)?;
            connect(tcp, &self.identity, expected).await
        })
        .await
        .context("连接超时，请检查 IP、端口和防火墙")?
    }

    pub(crate) async fn open_peer(&self, id: &str) -> Result<Connection> {
        let peer = self.store.peer(id)?;
        let mode = self.network_settings().mode;
        let mut local_error = None;
        if mode != NetworkMode::Internet {
            match self.open_lan(id, peer.address).await {
                Ok(stream) => {
                    return Ok(Connection {
                        stream,
                        route: Route::Lan,
                    })
                }
                Err(error) if mode == NetworkMode::Lan => {
                    return Err(error.context("仅局域网模式连接失败，不会回退互联网"))
                }
                Err(error) => local_error = Some(error),
            }
        }
        let internet = async {
            let remote = self.relay_client()?.open(Target::Peer(id.into())).await?;
            anyhow::ensure!(remote.peer_id == id, "中继返回了错误的目标设备");
            let stream = timeout(
                Duration::from_secs(10),
                connect(remote.stream, &self.identity, Some(id)),
            )
            .await
            .context("设备间中继 TLS 握手超时")??;
            Ok(Connection {
                stream,
                route: Route::Relay,
            })
        }
        .await;
        internet.with_context(|| match local_error {
            Some(error) => format!("局域网不可达（{error:#}）；互联网中继连接失败"),
            None => "互联网中继连接失败".into(),
        })
    }

    pub(crate) async fn open_lan(&self, id: &str, remembered_address: String) -> Result<TlsStream> {
        let mut addresses: Vec<String> = self
            .discovered()
            .into_iter()
            .filter(|device| device.id == id)
            .flat_map(|device| device.addresses)
            .collect();
        addresses.push(remembered_address);
        addresses.sort();
        addresses.dedup();
        timeout(Duration::from_secs(3), async {
            let attempts = addresses
                .into_iter()
                .filter_map(|address| address.parse::<SocketAddr>().ok())
                .take(8)
                .map(|address| self.open_address(address, Some(id)));
            let mut attempts = stream::iter(attempts).buffer_unordered(4);
            let mut last_error = anyhow::anyhow!("没有可用的局域网地址");
            while let Some(result) = attempts.next().await {
                match result {
                    Ok(stream) => return Ok(stream),
                    Err(error) => last_error = error,
                }
            }
            Err(last_error)
        })
        .await
        .context("局域网探测超时")?
    }

    pub(crate) async fn outbound_permit(&self) -> Result<tokio::sync::OwnedSemaphorePermit> {
        self.outbound
            .clone()
            .try_acquire_owned()
            .context("同时发送的任务过多，请等待已有任务完成")
    }
}
