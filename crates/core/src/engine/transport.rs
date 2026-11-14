use super::Engine;
use anyhow::{Context, Result};
use file_trans_identity::{connect, TlsStream};
use std::{net::SocketAddr, time::Duration};
use tokio::{net::TcpStream, time::timeout};

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

    pub(crate) async fn open_peer(&self, id: &str) -> Result<TlsStream> {
        let peer = self.store.peer(id)?;
        let mut addresses: Vec<String> = self
            .discovered()
            .into_iter()
            .filter(|device| device.id == id)
            .flat_map(|device| device.addresses)
            .collect();
        addresses.push(peer.address);
        addresses.sort();
        addresses.dedup();
        let mut last_error = anyhow::anyhow!("没有可连接的设备地址");
        for address in addresses.into_iter().take(8) {
            if let Ok(address) = address.parse() {
                match self.open_address(address, Some(id)).await {
                    Ok(stream) => return Ok(stream),
                    Err(error) => last_error = error,
                }
            }
        }
        Err(last_error.context("设备连接失败"))
    }

    pub(crate) async fn outbound_permit(&self) -> Result<tokio::sync::OwnedSemaphorePermit> {
        self.outbound
            .clone()
            .try_acquire_owned()
            .context("同时发送的任务过多，请等待已有任务完成")
    }
}
