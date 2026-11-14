use super::Store;
use crate::{
    model::Activity,
    transfers::model::{Record, TransferKey},
};
use anyhow::{ensure, Context, Result};
use rusqlite::{params, OptionalExtension, Transaction};

impl Store {
    pub(crate) fn init_transfers(&self) -> Result<()> {
        self.0.lock().unwrap().execute_batch("CREATE TABLE IF NOT EXISTS file_transfers (id TEXT NOT NULL, peer_id TEXT NOT NULL, direction TEXT NOT NULL, data TEXT NOT NULL, PRIMARY KEY(id, peer_id, direction));")?;
        for record in self.transfers()? {
            if !record.terminal() {
                self.update_transfer(&record.key, |record| {
                    record.state(
                        if record.key.direction == file_trans_protocol::Direction::Outgoing {
                            "waiting_retry"
                        } else if record.accepted {
                            "waiting_connection"
                        } else {
                            "awaiting_accept"
                        },
                    );
                    Ok(())
                })?;
            }
        }
        Ok(())
    }

    pub(crate) fn transfers(&self) -> Result<Vec<Record>> {
        let db = self.0.lock().unwrap();
        let mut query = db.prepare("SELECT data FROM file_transfers")?;
        let values = query
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        values
            .into_iter()
            .map(|value| Ok(serde_json::from_str(&value)?))
            .collect()
    }

    pub(crate) fn transfer(&self, key: &TransferKey) -> Result<Option<Record>> {
        let json: Option<String> = self
            .0
            .lock()
            .unwrap()
            .query_row(
                "SELECT data FROM file_transfers WHERE id=?1 AND peer_id=?2 AND direction=?3",
                params![key.id, key.peer_id, key.direction.as_str()],
                |row| row.get(0),
            )
            .optional()?;
        json.map(|json| Ok(serde_json::from_str(&json)?))
            .transpose()
    }

    pub(crate) fn insert_transfer(&self, record: &Record) -> Result<()> {
        let mut db = self.0.lock().unwrap();
        let tx = db.transaction()?;
        let count: usize = tx.query_row("SELECT COUNT(*) FROM file_transfers WHERE json_extract(data,'$.activity.status') NOT IN ('completed','rejected','cancelled','failed')", [], |row| row.get(0))?;
        ensure!(count < 128, "未完成文件任务过多，请取消不再需要的任务");
        tx.execute(
            "INSERT INTO file_transfers VALUES (?1,?2,?3,?4)",
            params![
                record.key.id,
                record.key.peer_id,
                record.key.direction.as_str(),
                serde_json::to_string(record)?
            ],
        )?;
        save_activity(&tx, &record.activity)?;
        tx.commit()?;
        Ok(())
    }

    pub(crate) fn update_transfer(
        &self,
        key: &TransferKey,
        update: impl FnOnce(&mut Record) -> Result<()>,
    ) -> Result<Record> {
        let mut db = self.0.lock().unwrap();
        let tx = db.transaction()?;
        let value: String = tx
            .query_row(
                "SELECT data FROM file_transfers WHERE id=?1 AND peer_id=?2 AND direction=?3",
                params![key.id, key.peer_id, key.direction.as_str()],
                |row| row.get(0),
            )
            .optional()?
            .context("文件任务不存在")?;
        let mut record: Record = serde_json::from_str(&value)?;
        update(&mut record)?;
        tx.execute(
            "UPDATE file_transfers SET data=?4 WHERE id=?1 AND peer_id=?2 AND direction=?3",
            params![
                key.id,
                key.peer_id,
                key.direction.as_str(),
                serde_json::to_string(&record)?
            ],
        )?;
        save_activity(&tx, &record.activity)?;
        tx.commit()?;
        Ok(record)
    }
}

fn save_activity(tx: &Transaction<'_>, activity: &Activity) -> Result<()> {
    tx.execute("INSERT INTO activities VALUES (?1,?2,?3,?4,?5) ON CONFLICT(id,direction,peer_id) DO UPDATE SET data=excluded.data", params![activity.id, activity.direction, activity.peer_id, activity.timestamp, serde_json::to_string(activity)?])?;
    Ok(())
}
