use super::Engine;
use crate::model::Route;
use anyhow::{ensure, Result};
use file_trans_protocol::{read_message, write_message, Message, VERSION};
use futures_util::{stream, StreamExt};
use std::{collections::HashMap, time::Duration};
use tokio::time::timeout;

impl Engine {
    pub async fn refresh_peer_connections(&self) -> Result<HashMap<String, Option<Route>>> {
        let peers = self.store.peers()?;
        let checks = stream::iter(peers).map(|peer| async move {
            let route = self.probe_peer(&peer.id).await.ok();
            (peer.id, route)
        });
        Ok(checks.buffer_unordered(4).collect().await)
    }

    async fn probe_peer(&self, id: &str) -> Result<Route> {
        let mut connection = self.open_peer(id).await?;
        timeout(Duration::from_secs(3), async {
            write_message(&mut connection.stream, &Message::Ping { version: VERSION }).await?;
            let response = read_message(&mut connection.stream).await?;
            ensure!(
                matches!(response, Message::Pong { device } if device.id == id),
                "设备响应无效"
            );
            Ok(connection.route)
        })
        .await?
    }
}
