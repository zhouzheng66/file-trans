use super::manifest::{self, Entry, MAX_MANIFEST, MAX_SIZE};
use anyhow::{ensure, Context, Result};
use sha2::{Digest, Sha256};
use std::path::Path;
use tokio::{
    fs::{File, OpenOptions},
    io::{AsyncReadExt, AsyncWrite, AsyncWriteExt},
};

const MAGIC: &[u8; 8] = b"FTDIR001";

pub(super) async fn encode(
    root: &Path,
    entries: &[Entry],
    output: &mut (impl AsyncWrite + Unpin),
    stop: &impl Fn() -> bool,
) -> Result<(u64, String)> {
    let header = serde_json::to_vec(entries)?;
    ensure!(header.len() <= MAX_MANIFEST, "目录清单超过 16 MiB");
    let size = manifest::validate(entries)?
        .checked_add(12 + header.len() as u64)
        .context("目录大小溢出")?;
    ensure!(size <= MAX_SIZE, "目录快照超过 8 TiB");
    let mut hash = Sha256::new();
    let mut prefix = MAGIC.to_vec();
    prefix.extend_from_slice(&(header.len() as u32).to_be_bytes());
    prefix.extend_from_slice(&header);
    output.write_all(&prefix).await?;
    hash.update(&prefix);
    let mut bytes = vec![0; file_trans_protocol::CHUNK_SIZE];
    for entry in entries.iter().filter(|entry| !entry.directory) {
        ensure!(!stop(), "目录准备已暂停或取消");
        let path = root.join(&entry.path);
        let metadata = tokio::fs::symlink_metadata(&path).await?;
        check(&metadata, entry)?;
        let mut input = File::open(&path).await?;
        check(&input.metadata().await?, entry)?;
        let mut remaining = entry.size;
        while remaining > 0 {
            ensure!(!stop(), "目录准备已暂停或取消");
            let count = remaining.min(bytes.len() as u64) as usize;
            input.read_exact(&mut bytes[..count]).await?;
            output.write_all(&bytes[..count]).await?;
            hash.update(&bytes[..count]);
            remaining -= count as u64;
        }
        check(&input.metadata().await?, entry)?;
        check(&tokio::fs::symlink_metadata(&path).await?, entry)?;
    }
    ensure!(
        manifest::scan(root, stop).await? == entries,
        "准备期间目录内容发生变化，请重新发送"
    );
    output.flush().await?;
    Ok((size, hex::encode(hash.finalize())))
}

fn check(metadata: &std::fs::Metadata, entry: &Entry) -> Result<()> {
    ensure!(
        manifest::ordinary(metadata)
            && metadata.is_file()
            && metadata.len() == entry.size
            && crate::transfers::source::modified(metadata) == entry.modified,
        "准备期间源文件发生变化：{}",
        entry.path
    );
    Ok(())
}

pub(super) async fn unpack(
    path: &Path,
    destination: &Path,
    stop: &impl Fn() -> bool,
) -> Result<()> {
    let mut input = File::open(path).await?;
    let mut magic = [0; 8];
    input.read_exact(&mut magic).await?;
    ensure!(&magic == MAGIC, "目录快照格式错误");
    let length = input.read_u32().await? as usize;
    ensure!(length > 0 && length <= MAX_MANIFEST, "目录清单长度无效");
    let mut header = vec![0; length];
    input.read_exact(&mut header).await?;
    let entries: Vec<Entry> = serde_json::from_slice(&header)?;
    let total = manifest::validate(&entries)?;
    ensure!(
        total + 12 + length as u64 == input.metadata().await?.len(),
        "目录快照长度不一致"
    );
    let mut bytes = vec![0; file_trans_protocol::CHUNK_SIZE];
    for entry in &entries {
        ensure!(!stop(), "目录展开已暂停或取消");
        let output = destination.join(&entry.path);
        if entry.directory {
            tokio::fs::create_dir(output).await?;
            continue;
        }
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            options.mode(0o600);
        }
        let mut file = options.open(output).await?;
        let mut remaining = entry.size;
        while remaining > 0 {
            ensure!(!stop(), "目录展开已暂停或取消");
            let count = remaining.min(bytes.len() as u64) as usize;
            input.read_exact(&mut bytes[..count]).await?;
            file.write_all(&bytes[..count]).await?;
            file.flush().await?;
            remaining -= count as u64;
        }
        file.flush().await?;
        file.sync_data().await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn rejects_unsafe_duplicate_and_conflicting_manifest_paths_before_writing() -> Result<()>
    {
        let bad = [
            serde_json::json!([{"path":"../outside","directory":false,"size":0}]),
            serde_json::json!([{"path":"/absolute","directory":false,"size":0}]),
            serde_json::json!([{"path":"C:\\secret","directory":false,"size":0}]),
            serde_json::json!([{"path":"CON.txt","directory":false,"size":0}]),
            serde_json::json!([{"path":"a","directory":false,"size":0},{"path":"a","directory":false,"size":0}]),
            serde_json::json!([{"path":"A","directory":true,"size":0},{"path":"a","directory":false,"size":0}]),
            serde_json::json!([{"path":"a","directory":false,"size":0},{"path":"a/b","directory":false,"size":0}]),
            serde_json::json!([{"path":"a/b","directory":false,"size":0}]),
            serde_json::json!([{"path":"dir","directory":true,"size":1}]),
            serde_json::json!([{"path":"link","directory":false,"size":0,"symlink":"outside"}]),
        ];
        let temp = tempfile::tempdir()?;
        let output = temp.path().join("output");
        tokio::fs::create_dir(&output).await?;
        let archive = temp.path().join("archive");
        for manifest in bad {
            let header = serde_json::to_vec(&manifest)?;
            let mut bytes = MAGIC.to_vec();
            bytes.extend_from_slice(&(header.len() as u32).to_be_bytes());
            bytes.extend_from_slice(&header);
            tokio::fs::write(&archive, bytes).await?;
            assert!(
                unpack(&archive, &output, &|| false).await.is_err(),
                "accepted {manifest}"
            );
            assert!(std::fs::read_dir(&output)?.next().is_none());
        }
        let mut bytes = MAGIC.to_vec();
        bytes.extend_from_slice(&u32::MAX.to_be_bytes());
        tokio::fs::write(&archive, bytes).await?;
        assert!(unpack(&archive, &output, &|| false).await.is_err());
        Ok(())
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn rejects_source_symbolic_links_and_case_collisions() -> Result<()> {
        let root = tempfile::tempdir()?;
        let target = root.path().join("outside");
        tokio::fs::write(&target, b"outside").await?;
        let folder = root.path().join("source");
        tokio::fs::create_dir(&folder).await?;
        std::os::unix::fs::symlink(&target, folder.join("link"))?;
        assert!(manifest::scan(&folder, &|| false).await.is_err());
        tokio::fs::remove_file(folder.join("link")).await?;
        let entries = vec![
            Entry {
                path: "A".into(),
                directory: true,
                size: 0,
                modified: None,
            },
            Entry {
                path: "a".into(),
                directory: false,
                size: 0,
                modified: None,
            },
        ];
        assert!(manifest::validate(&entries).is_err());
        Ok(())
    }
}
