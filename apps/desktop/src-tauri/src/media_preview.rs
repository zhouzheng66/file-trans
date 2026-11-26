use crate::AppState;
use file_trans_core::transfers::TransferKey;
use std::{fs::File, io::Read, path::Path};
use tauri::{utils::mime_type::MimeType, Manager, State, WebviewWindow};

#[cfg(any(target_os = "linux", test))]
mod loopback;

const MAX_IMAGE_BYTES: u64 = 32 * 1024 * 1024;

#[tauri::command]
pub async fn prepare_media_preview(
    window: WebviewWindow,
    state: State<'_, AppState>,
    key: TransferKey,
) -> Result<String, String> {
    let engine = state.0.clone();
    let lookup_key = key.clone();
    let (path, _mime) = tauri::async_runtime::spawn_blocking(move || {
        let path = engine
            .local_attachment_path(&lookup_key)
            .map_err(|error| format!("{error:#}"))?;
        let mime = validate_media(&path)?;
        Ok::<_, String>((path, mime))
    })
    .await
    .map_err(|error| error.to_string())??;
    #[cfg(target_os = "linux")]
    if _mime.starts_with("video/") {
        return loopback::video_url(state.0.clone(), &key).await;
    }
    window
        .asset_protocol_scope()
        .allow_file(&path)
        .map_err(|error| format!("无法读取预览文件：{error}"))?;
    window
        .convert_file_src(path, None)
        .map_err(|error| error.to_string())
}

fn validate_media(path: &Path) -> Result<&'static str, String> {
    let file = File::open(path).map_err(|error| format!("无法打开预览文件：{error}"))?;
    let metadata = file.metadata().map_err(|error| error.to_string())?;
    if !metadata.is_file() {
        return Err("仅支持预览普通文件".into());
    }
    let extension = path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    validate_contents(&extension, metadata.len(), file)
}

fn validate_contents(extension: &str, size: u64, file: impl Read) -> Result<&'static str, String> {
    let expected_mime = match extension {
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "webp" => "image/webp",
        "gif" => "image/gif",
        "bmp" => "image/bmp",
        "avif" => "image/avif",
        "mp4" | "m4v" | "mov" => "video/mp4",
        "webm" => "video/webm",
        "ogv" => "audio/ogg", // Tauri 的内容检测将 Ogg 容器统一标记为 audio/ogg。
        _ => return Err("此文件格式不支持预览".into()),
    };
    if expected_mime.starts_with("image/") && size > MAX_IMAGE_BYTES {
        return Err("图片超过 32 MiB，无法在聊天中预览".into());
    }

    // 和 Tauri asset 协议读取相同长度的文件头，禁用根据扩展名猜测 MIME 的回退。
    let mut header = Vec::new();
    file.take(8192)
        .read_to_end(&mut header)
        .map_err(|error| format!("无法读取预览文件：{error}"))?;
    let mime = MimeType::parse_with_fallback(&header, "", MimeType::OctetStream);
    let compatible = mime == expected_mime
        || (expected_mime == "video/mp4"
            && matches!(mime.as_str(), "video/quicktime" | "video/x-m4v"));
    if !compatible {
        return Err("文件内容与媒体格式不符，无法预览".into());
    }
    Ok(match mime.as_str() {
        "video/quicktime" => "video/quicktime",
        "audio/ogg" => "video/ogg",
        _ => expected_mime,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checks_image_size_and_content_before_granting_access() {
        let png = include_bytes!("../icons/32x32.png");
        assert!(validate_contents("png", png.len() as u64, png.as_slice()).is_ok());
        assert!(validate_contents("png", MAX_IMAGE_BYTES + 1, png.as_slice()).is_err());
        assert!(validate_contents("png", 20, b"<html>preview</html>".as_slice()).is_err());
        assert!(validate_contents("mp4", 20, b"<html>preview</html>".as_slice()).is_err());
    }

    #[test]
    fn large_video_only_requires_its_header() {
        let mp4 = b"\x00\x00\x00\x18ftypisom\x00\x00\x02\x00isomiso2";
        assert!(validate_contents("mp4", 4_000_000_000, mp4.as_slice()).is_ok());
    }
}
