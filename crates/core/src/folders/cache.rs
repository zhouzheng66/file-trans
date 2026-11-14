use crate::transfers::model::Record;
use anyhow::Result;
use std::{collections::HashSet, path::Path};

/// 启动时尚未调度发送任务，可以回收准备阶段崩溃留下的未引用缓存。
pub(crate) async fn recover(directory: &Path, records: &[Record]) -> Result<()> {
    let retained: HashSet<_> = records
        .iter()
        .filter_map(|record| record.source_path.as_deref())
        .collect();
    let mut reader = match tokio::fs::read_dir(directory).await {
        Ok(reader) => reader,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    while let Some(entry) = reader.next_entry().await? {
        let path = entry.path();
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        let managed = name.starts_with(".building-")
            || name
                .strip_suffix(".bundle")
                .is_some_and(|id| uuid::Uuid::parse_str(id).is_ok());
        if managed && !retained.contains(path.as_path()) && entry.file_type().await?.is_file() {
            tokio::fs::remove_file(path).await?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transfers::{Direction, TransferKey};

    #[tokio::test]
    async fn startup_keeps_referenced_snapshots_and_removes_only_managed_orphans() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let retained = directory
            .path()
            .join(format!("{}.bundle", uuid::Uuid::new_v4()));
        let orphan = directory
            .path()
            .join(format!("{}.bundle", uuid::Uuid::new_v4()));
        let building = directory.path().join(".building-interrupted");
        let other = directory.path().join("unrelated.txt");
        for path in [&retained, &orphan, &building, &other] {
            tokio::fs::write(path, b"sample").await?;
        }
        let mut record = Record::new(
            TransferKey {
                id: uuid::Uuid::new_v4().to_string(),
                peer_id: "0".repeat(64),
                direction: Direction::Outgoing,
            },
            "folder".into(),
            0,
        );
        record.source_path = Some(retained.clone());
        recover(directory.path(), &[record]).await?;
        assert!(retained.exists() && other.exists());
        assert!(!orphan.exists() && !building.exists());
        Ok(())
    }
}
