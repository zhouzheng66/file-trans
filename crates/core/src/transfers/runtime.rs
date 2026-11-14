use super::model::TransferKey;
use anyhow::{ensure, Result};
use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex, Weak},
};
use tokio::sync::watch;
use tokio_util::task::TaskTracker;

pub(crate) struct Control {
    pub revision: watch::Sender<u64>,
    pub receive_lock: Arc<tokio::sync::Mutex<()>>,
}

#[derive(Default)]
pub(crate) struct Runtime {
    controls: Mutex<HashMap<TransferKey, Weak<Control>>>,
    running: Mutex<HashSet<TransferKey>>,
    pub tracker: TaskTracker,
}

impl Runtime {
    pub fn control(&self, key: &TransferKey) -> Arc<Control> {
        let mut controls = self.controls.lock().unwrap();
        if let Some(control) = controls.get(key).and_then(Weak::upgrade) {
            return control;
        }
        if controls.len() >= 128 {
            controls.retain(|_, control| control.strong_count() > 0);
        }
        let (revision, _) = watch::channel(0);
        let control = Arc::new(Control {
            revision,
            receive_lock: Arc::new(tokio::sync::Mutex::new(())),
        });
        controls.insert(key.clone(), Arc::downgrade(&control));
        control
    }
    pub fn wake(&self, key: &TransferKey) {
        self.control(key)
            .revision
            .send_modify(|value| *value = value.wrapping_add(1));
    }
    pub fn reserve(self: &Arc<Self>, key: &TransferKey) -> Result<Option<Running>> {
        let mut running = self.running.lock().unwrap();
        if running.contains(key) {
            return Ok(None);
        }
        ensure!(
            running.len() < 128,
            "未完成发送任务过多，请暂停后取消不再需要的任务"
        );
        running.insert(key.clone());
        Ok(Some(Running {
            runtime: self.clone(),
            key: key.clone(),
        }))
    }
}

pub(crate) struct Running {
    runtime: Arc<Runtime>,
    key: TransferKey,
}
impl Drop for Running {
    fn drop(&mut self) {
        self.runtime.running.lock().unwrap().remove(&self.key);
    }
}
