use crate::model::{Activity, Peer};
use anyhow::{ensure, Result};
use rusqlite::{params, Connection, OptionalExtension};
use std::{path::Path, sync::Mutex};
mod transfers;

pub struct Store(Mutex<Connection>);

impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        let connection = Connection::open(path)?;
        connection.execute_batch("PRAGMA journal_mode=WAL;
            CREATE TABLE IF NOT EXISTS peers (id TEXT PRIMARY KEY, name TEXT NOT NULL, address TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS activities (id TEXT NOT NULL, direction TEXT NOT NULL, peer_id TEXT NOT NULL, timestamp INTEGER NOT NULL, data TEXT NOT NULL, PRIMARY KEY(id, direction, peer_id));
            CREATE TABLE IF NOT EXISTS settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);")?;
        let store = Self(Mutex::new(connection));
        for mut activity in store.activities()? {
            if !activity.resumable
                && !["completed", "failed", "rejected"].contains(&activity.status.as_str())
            {
                activity.status = "failed".into();
                activity.error = Some("上次运行已中断，请重新发送".into());
                store.save_activity(&activity)?;
            }
        }
        store.init_transfers()?;
        Ok(store)
    }

    pub fn save_peer(&self, peer: &Peer) -> Result<()> {
        self.0.lock().unwrap().execute("INSERT INTO peers VALUES (?1, ?2, ?3) ON CONFLICT(id) DO UPDATE SET name=excluded.name, address=excluded.address", params![peer.id, peer.name, peer.address])?;
        Ok(())
    }

    pub fn peers(&self) -> Result<Vec<Peer>> {
        let db = self.0.lock().unwrap();
        let mut query = db.prepare("SELECT id, name, address FROM peers ORDER BY name")?;
        let records = query
            .query_map([], |row| {
                Ok(Peer {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    address: row.get(2)?,
                })
            })?
            .collect::<std::result::Result<_, _>>()?;
        Ok(records)
    }

    pub fn peer(&self, id: &str) -> Result<Peer> {
        self.peers()?
            .into_iter()
            .find(|peer| peer.id == id)
            .ok_or_else(|| anyhow::anyhow!("设备尚未配对或信任已移除"))
    }

    pub fn forget_peer(&self, id: &str) -> Result<()> {
        self.0
            .lock()
            .unwrap()
            .execute("DELETE FROM peers WHERE id=?1", [id])?;
        Ok(())
    }

    pub fn save_activity(&self, activity: &Activity) -> Result<()> {
        self.0.lock().unwrap().execute("INSERT INTO activities VALUES (?1, ?2, ?3, ?4, ?5) ON CONFLICT(id, direction, peer_id) DO UPDATE SET data=excluded.data", params![activity.id, activity.direction, activity.peer_id, activity.timestamp, serde_json::to_string(activity)?])?;
        Ok(())
    }

    pub fn activities(&self) -> Result<Vec<Activity>> {
        let db = self.0.lock().unwrap();
        let mut query =
            db.prepare("SELECT data FROM activities ORDER BY CASE WHEN json_extract(data,'$.status') IN ('completed','failed','rejected','cancelled') THEN 1 ELSE 0 END, timestamp DESC LIMIT 328")?;
        let records = query
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        records
            .into_iter()
            .map(|json| Ok(serde_json::from_str(&json)?))
            .collect()
    }

    pub fn received(&self, id: &str, peer_id: &str) -> Result<bool> {
        let db = self.0.lock().unwrap();
        let json: Option<String> = db
            .query_row(
                "SELECT data FROM activities WHERE id=?1 AND direction='incoming' AND peer_id=?2",
                params![id, peer_id],
                |row| row.get(0),
            )
            .optional()?;
        Ok(json
            .and_then(|json| serde_json::from_str::<Activity>(&json).ok())
            .is_some_and(|activity| activity.status == "completed"))
    }

    pub fn receive_text(&self, activity: &Activity) -> Result<()> {
        let db = self.0.lock().unwrap();
        let existing: Option<String> = db
            .query_row(
                "SELECT data FROM activities WHERE id=?1 AND direction='incoming' AND peer_id=?2",
                params![activity.id, activity.peer_id],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(existing) = existing {
            let existing: Activity = serde_json::from_str(&existing)?;
            ensure!(
                existing.kind == "text"
                    && existing.content == activity.content
                    && existing.status == "completed",
                "消息 ID 与已有内容冲突"
            );
        } else {
            db.execute(
                "INSERT INTO activities VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    activity.id,
                    activity.direction,
                    activity.peer_id,
                    activity.timestamp,
                    serde_json::to_string(activity)?
                ],
            )?;
        }
        Ok(())
    }

    pub fn setting(&self, key: &str) -> Result<Option<String>> {
        Ok(self
            .0
            .lock()
            .unwrap()
            .query_row("SELECT value FROM settings WHERE key=?1", [key], |row| {
                row.get(0)
            })
            .optional()?)
    }

    pub fn set_setting(&self, key: &str, value: &str) -> Result<()> {
        ensure!(value.len() < 4096, "设置值过长");
        self.0.lock().unwrap().execute("INSERT INTO settings VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value", params![key, value])?;
        Ok(())
    }
}
