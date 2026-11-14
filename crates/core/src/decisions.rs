use crate::model::Pairing;
use anyhow::{anyhow, ensure, Result};
use std::{collections::HashMap, sync::Mutex};
use tokio::sync::oneshot;

type PendingPairing = (Pairing, Option<oneshot::Sender<bool>>);

#[derive(Default)]
pub struct Pairings {
    entries: Mutex<HashMap<String, PendingPairing>>,
}

impl Pairings {
    pub fn register(&self, pairing: Pairing) -> Result<oneshot::Receiver<bool>> {
        let mut entries = self.entries.lock().unwrap();
        ensure!(
            entries
                .values()
                .filter(|(entry, _)| ["pending", "waiting_peer"].contains(&entry.status.as_str()))
                .count()
                < 8,
            "待确认配对过多，请稍后再试"
        );
        if entries.len() >= 32 {
            entries.retain(|_, (entry, _)| {
                ["pending", "waiting_peer"].contains(&entry.status.as_str())
            });
        }
        let (tx, rx) = oneshot::channel();
        entries.insert(pairing.id.clone(), (pairing, Some(tx)));
        Ok(rx)
    }

    pub fn decide(&self, id: &str, accepted: bool) -> Result<()> {
        let mut entries = self.entries.lock().unwrap();
        let (entry, sender) = entries
            .get_mut(id)
            .ok_or_else(|| anyhow!("配对请求不存在或已过期"))?;
        let sender = sender.take().ok_or_else(|| anyhow!("配对请求已经处理"))?;
        entry.status = "waiting_peer".into();
        sender
            .send(accepted)
            .map_err(|_| anyhow!("配对请求已经过期"))
    }

    pub fn finish(&self, id: &str, result: &Result<bool>) {
        if let Some((entry, sender)) = self.entries.lock().unwrap().get_mut(id) {
            sender.take();
            entry.status = match result {
                Ok(true) => "paired",
                Ok(false) => "rejected",
                Err(_) => "failed",
            }
            .into();
            entry.error = result.as_ref().err().map(|error| format!("{error:#}"));
        }
    }

    pub fn snapshot(&self) -> Vec<Pairing> {
        let mut entries: Vec<_> = self
            .entries
            .lock()
            .unwrap()
            .values()
            .map(|(entry, _)| entry.clone())
            .collect();
        entries.sort_by(|a, b| a.id.cmp(&b.id));
        entries
    }
}

#[derive(Default)]
pub struct Receipts(Mutex<HashMap<String, oneshot::Sender<bool>>>);

impl Receipts {
    pub fn register(&self, id: &str) -> Result<oneshot::Receiver<bool>> {
        let mut entries = self.0.lock().unwrap();
        ensure!(
            entries.len() < 8 && !entries.contains_key(id),
            "待接收任务过多或任务 ID 重复"
        );
        let (tx, rx) = oneshot::channel();
        entries.insert(id.to_owned(), tx);
        Ok(rx)
    }

    pub fn decide(&self, id: &str, accepted: bool) -> Result<()> {
        self.0
            .lock()
            .unwrap()
            .remove(id)
            .ok_or_else(|| anyhow!("接收请求不存在或已过期"))?
            .send(accepted)
            .map_err(|_| anyhow!("接收请求已过期"))
    }

    pub fn remove(&self, id: &str) {
        self.0.lock().unwrap().remove(id);
    }
}
