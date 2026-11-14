use super::model::Record;
use anyhow::{ensure, Context, Result};
use file_trans_protocol::CHUNK_SIZE;
use sha2::{Digest, Sha256};
use std::fs::Metadata;
use tokio::{
    fs::File,
    io::{AsyncReadExt, AsyncSeekExt},
};

pub(crate) fn modified(metadata: &Metadata) -> Option<u64> {
    metadata
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_nanos()
        .try_into()
        .ok()
}

pub(crate) fn check(metadata: &Metadata, record: &Record) -> Result<()> {
    ensure!(
        metadata.is_file()
            && metadata.len() == record.activity.size
            && modified(metadata) == record.source_modified,
        "源文件已发生变化，请取消此任务并重新发送"
    );
    Ok(())
}

pub(crate) async fn open(record: &Record) -> Result<File> {
    let path = record
        .source_path
        .as_deref()
        .or_else(|| record.activity.path.as_deref().map(std::path::Path::new))
        .context("源文件路径丢失")?;
    let file = File::open(path)
        .await
        .context("源文件不可用，请恢复原路径后重试")?;
    check(&file.metadata().await?, record)?;
    Ok(file)
}

pub(crate) async fn prefix(file: &mut File, length: u64) -> Result<Sha256> {
    file.seek(std::io::SeekFrom::Start(0)).await?;
    let mut hash = Sha256::new();
    let mut remaining = length;
    let mut buffer = vec![0; CHUNK_SIZE];
    while remaining > 0 {
        let count = remaining.min(buffer.len() as u64) as usize;
        file.read_exact(&mut buffer[..count])
            .await
            .context("本地文件内容不足，无法恢复断点")?;
        hash.update(&buffer[..count]);
        remaining -= count as u64;
    }
    Ok(hash)
}

pub(crate) fn digest(hash: &Sha256) -> String {
    hex::encode(hash.clone().finalize())
}
