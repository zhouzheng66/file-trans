//! 局域网候选地址的动态连接与已配对设备的主动发现。
use super::Engine;
use anyhow::{Context, Result};
use file_trans_discovery::{DiscoveredDevice, Discovery};
use file_trans_identity::TlsStream;
use futures_util::{stream, stream::FuturesUnordered, StreamExt};
use std::{collections::HashSet, future::Future, net::SocketAddr, sync::Arc, time::Duration};
use tokio::{
    sync::watch,
    time::{sleep, timeout},
};

#[cfg(test)]
mod tests;

const CONNECT_BUDGET: Duration = Duration::from_secs(5);

impl Engine {
    pub(crate) async fn open_lan(&self, id: &str, remembered: String) -> Result<TlsStream> {
        let discovery = self.discovery.lock().unwrap().clone();
        let mut devices = discovery
            .as_ref()
            .map(|discovery| discovery.subscribe())
            .unwrap_or_else(|| watch::channel(Vec::new()).1);
        let refresh = async {
            match &discovery {
                Some(discovery) => discovery.resolve_peer(id).await,
                None => Ok(()),
            }
        };
        self.connect_lan(id, remembered, &mut devices, refresh, discovery.is_some())
            .await
    }

    /// 订阅先于读取快照，确保握手期间到达的地址更新也能参加竞争。
    async fn connect_lan(
        &self,
        id: &str,
        remembered: String,
        devices: &mut watch::Receiver<Vec<DiscoveredDevice>>,
        refresh: impl Future<Output = Result<()>>,
        mut watching: bool,
    ) -> Result<TlsStream> {
        let mut attempted = HashSet::new();
        let mut attempts = FuturesUnordered::new();
        let mut errors = Vec::new();
        let mut resolving = watching;
        tokio::pin!(refresh);
        let work = async {
            loop {
                let candidates = devices.borrow_and_update().clone();
                let addresses = candidates
                    .into_iter()
                    .filter(|device| device.id == id)
                    .flat_map(|device| device.addresses)
                    .chain(std::iter::once(remembered.clone()));
                // 优先发现的新地址；一个失效地址不会阻塞后续的地址更新。
                for address in addresses.filter_map(|address| address.parse::<SocketAddr>().ok()) {
                    if attempts.len() >= 4 || attempted.len() >= 32 {
                        break;
                    }
                    if attempted.insert(address) {
                        attempts.push(async move {
                            let result = timeout(
                                Duration::from_secs(2),
                                self.open_address(address, Some(id)),
                            )
                            .await
                            .context("地址连接超时")
                            .and_then(|result| result);
                            (address, result)
                        });
                    }
                }
                if attempts.is_empty() && !watching && !resolving {
                    return Ok(None);
                }
                tokio::select! {
                    _ = self.cancel.cancelled() => anyhow::bail!("应用已退出"),
                    Some((address, result)) = attempts.next(), if !attempts.is_empty() => {
                        match result {
                            Ok(stream) => {
                                // 广播不是信任凭据，只有固定指纹 TLS 验证成功才更新持久地址。
                                self.store.update_peer_address(id, &address.to_string())?;
                                self.store.peer(id).context("设备信任已移除")?;
                                return Ok(Some(stream));
                            }
                            Err(error) => errors.push(format!("{address}：{error:#}")),
                        }
                    },
                    result = &mut refresh, if resolving => {
                        resolving = false;
                        if let Err(error) = result { errors.push(format!("刷新设备地址：{error:#}")); }
                    },
                    result = devices.changed(), if watching => {
                        if result.is_err() { watching = false; }
                    },
                }
            }
        };
        match timeout(CONNECT_BUDGET, work).await {
            Ok(Ok(Some(stream))) => return Ok(stream),
            Ok(Err(error)) => return Err(error),
            _ => {}
        }
        if errors.is_empty() {
            anyhow::bail!(
                "未能连接设备，等待局域网地址或连接响应超时；请确认对方在线并允许本地网络访问"
            );
        }
        anyhow::bail!("局域网连接失败，已尝试自动刷新地址；{}", errors.join("；"))
    }
}

/// 即使没有正在发送的任务，也持续刷新已配对设备，恢复休眠后的发现状态。
/// 仅刷新地址，不发送业务内容，不将广播信息写入信任库。
pub(super) async fn refresh_paired(engine: Arc<Engine>, discovery: Arc<Discovery>) {
    loop {
        if engine.network_settings().mode != crate::model::NetworkMode::Internet {
            if let Ok(peers) = engine.store.peers() {
                let work = stream::iter(peers)
                    .map(|peer| {
                        let discovery = Arc::clone(&discovery);
                        async move {
                            let _ = discovery.resolve_peer(&peer.id).await;
                        }
                    })
                    .buffer_unordered(4)
                    .collect::<Vec<_>>();
                tokio::select! { _ = engine.cancel.cancelled() => break, _ = work => {} }
            }
        }
        tokio::select! { _ = engine.cancel.cancelled() => break, _ = sleep(Duration::from_secs(10)) => {} }
    }
}
