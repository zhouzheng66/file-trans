use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, fs::Metadata, path::Path};

pub(super) const MAX_MANIFEST: usize = 16 * 1024 * 1024;
pub(super) const MAX_ENTRIES: usize = 100_000;
pub(super) const MAX_SIZE: u64 = 8 * 1024_u64.pow(4);

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Entry {
    pub path: String,
    pub directory: bool,
    pub size: u64,
    #[serde(skip)]
    pub modified: Option<u64>,
}

pub(crate) fn ordinary(metadata: &Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return false;
        }
    }
    !metadata.is_symlink() && (metadata.is_file() || metadata.is_dir())
}

pub(crate) fn component(name: &str) -> Result<()> {
    ensure!(
        crate::files::safe_file_name(name)? == name,
        "名称不适合跨平台传输：{name}"
    );
    Ok(())
}

pub(super) fn validate(entries: &[Entry]) -> Result<u64> {
    ensure!(entries.len() <= MAX_ENTRIES, "目录条目过多");
    let mut paths = HashMap::new();
    let mut total = 0_u64;
    let mut previous: Option<&str> = None;
    for entry in entries {
        ensure!(
            !entry.path.is_empty() && entry.path.len() <= 1024,
            "相对路径长度无效"
        );
        let parts: Vec<_> = entry.path.split('/').collect();
        ensure!(parts.len() <= 64, "目录层级过深");
        for part in &parts {
            component(part)?;
        }
        ensure!(
            previous.is_none_or(|previous| previous < entry.path.as_str()),
            "目录清单未排序或包含重复路径"
        );
        previous = Some(&entry.path);
        let folded = entry.path.to_lowercase();
        ensure!(
            !paths.contains_key(&folded),
            "目录包含大小写冲突：{}",
            entry.path
        );
        if let Some((parent, _)) = entry.path.rsplit_once('/') {
            ensure!(
                paths.get(&parent.to_lowercase()) == Some(&(parent.to_owned(), true)),
                "目录父路径缺失、大小写冲突或不是目录"
            );
        }
        paths.insert(folded, (entry.path.clone(), entry.directory));
        ensure!(!entry.directory || entry.size == 0, "目录不能包含文件数据");
        total = total.checked_add(entry.size).context("目录大小溢出")?;
        ensure!(total <= MAX_SIZE, "文件夹超过 8 TiB");
    }
    Ok(total)
}

pub(super) async fn scan(root: &Path, stop: &impl Fn() -> bool) -> Result<Vec<Entry>> {
    let metadata = tokio::fs::symlink_metadata(root).await?;
    ensure!(
        ordinary(&metadata) && metadata.is_dir(),
        "请选择普通目录，不支持链接或重解析点"
    );
    let mut pending = vec![(root.to_path_buf(), String::new(), 0)];
    let mut entries = Vec::new();
    let mut manifest_bytes = 2;
    while let Some((directory, prefix, depth)) = pending.pop() {
        ensure!(!stop(), "目录准备已暂停或取消");
        ensure!(depth <= 64, "目录层级过深");
        let mut reader = tokio::fs::read_dir(directory).await?;
        while let Some(item) = reader.next_entry().await? {
            ensure!(!stop(), "目录准备已暂停或取消");
            let name = item
                .file_name()
                .into_string()
                .map_err(|_| anyhow::anyhow!("目录包含非 UTF-8 名称"))?;
            component(&name)?;
            let relative = if prefix.is_empty() {
                name
            } else {
                format!("{prefix}/{name}")
            };
            ensure!(
                relative.len() <= 1024 && entries.len() < MAX_ENTRIES,
                "目录路径或条目超过限制"
            );
            let metadata = tokio::fs::symlink_metadata(item.path()).await?;
            ensure!(
                ordinary(&metadata),
                "不支持链接、重解析点或特殊文件：{relative}"
            );
            if metadata.is_dir() {
                pending.push((item.path(), relative.clone(), depth + 1));
            }
            let entry = Entry {
                path: relative,
                directory: metadata.is_dir(),
                size: if metadata.is_file() {
                    metadata.len()
                } else {
                    0
                },
                modified: crate::transfers::source::modified(&metadata),
            };
            manifest_bytes += serde_json::to_vec(&entry)?.len() + 1;
            ensure!(manifest_bytes <= MAX_MANIFEST, "目录清单超过 16 MiB");
            entries.push(entry);
        }
    }
    entries.sort_by(|a, b| a.path.cmp(&b.path));
    validate(&entries)?;
    Ok(entries)
}
