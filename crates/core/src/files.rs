use anyhow::{ensure, Context, Result};
use std::path::{Path, PathBuf};
use tempfile::NamedTempFile;

pub fn safe_file_name(name: &str) -> Result<String> {
    ensure!(
        !name.is_empty()
            && ![".", ".."].contains(&name)
            && !name.contains(['/', '\\'])
            && !name.chars().any(char::is_control),
        "文件名称不安全"
    );
    let mut result: String = name
        .chars()
        .map(|c| if ":*?\"<>|".contains(c) { '_' } else { c })
        .collect();
    result = result.trim_end_matches([' ', '.']).to_owned();
    ensure!(!result.is_empty(), "文件名称为空");
    let stem = result
        .split('.')
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase();
    if [
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
        "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    ]
    .contains(&stem.as_str())
    {
        result.insert(0, '_');
    }
    ensure!(result.len() <= 220, "文件名称过长");
    Ok(result)
}

pub fn persist_without_overwrite(
    mut temporary: NamedTempFile,
    directory: &Path,
    name: &str,
) -> Result<PathBuf> {
    let original = Path::new(name);
    let stem = original
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or("file");
    let extension = original
        .extension()
        .and_then(|extension| extension.to_str());
    for index in 0..10_000 {
        let candidate = if index == 0 {
            name.to_owned()
        } else {
            match extension {
                Some(extension) => format!("{stem} ({index}).{extension}"),
                None => format!("{stem} ({index})"),
            }
        };
        let path = directory.join(candidate);
        match temporary.persist_noclobber(&path) {
            Ok(_) => return Ok(path),
            Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => {
                temporary = error.file
            }
            Err(error) => return Err(error.error).context("保存接收文件失败"),
        }
    }
    anyhow::bail!("同名文件过多，请更换接收目录")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_traversal_and_handles_windows_names() {
        for name in ["../secret", "C:\\secret", "/tmp/a", "..", "a\n.txt"] {
            assert!(safe_file_name(name).is_err());
        }
        assert_eq!(safe_file_name("CON.txt").unwrap(), "_CON.txt");
        assert_eq!(safe_file_name("你好?.bin").unwrap(), "你好_.bin");
    }

    #[test]
    fn never_overwrites_existing_files() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("a.txt"), b"original").unwrap();
        let temporary = NamedTempFile::new_in(directory.path()).unwrap();
        let path = persist_without_overwrite(temporary, directory.path(), "a.txt").unwrap();
        assert_eq!(path.file_name().unwrap(), "a (1).txt");
        assert_eq!(
            std::fs::read(directory.path().join("a.txt")).unwrap(),
            b"original"
        );
    }
}
