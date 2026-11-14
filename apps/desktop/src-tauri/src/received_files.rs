use crate::AppState;
use std::path::PathBuf;
use tauri::{AppHandle, State};
use tauri_plugin_opener::OpenerExt;

#[tauri::command]
pub async fn open_received_location(
    app: AppHandle,
    state: State<'_, AppState>,
    peer_id: String,
    id: String,
) -> Result<(), String> {
    // 从已完成的接收记录取路径，不接受前端传入任意文件路径。
    let activity = state
        .0
        .snapshot()
        .map_err(|error| format!("{error:#}"))?
        .activities
        .into_iter()
        .find(|activity| {
            activity.id == id
                && activity.peer_id == peer_id
                && activity.direction == "incoming"
                && activity.status == "completed"
                && ["file", "folder"].contains(&activity.kind.as_str())
        })
        .ok_or("找不到已接收完成的文件或文件夹")?;
    let path = PathBuf::from(activity.path.ok_or("接收记录缺少保存位置")?);
    tauri::async_runtime::spawn_blocking(move || {
        if !path.try_exists().map_err(|error| error.to_string())? {
            return Err("文件或文件夹已被移动或删除，无法定位原位置".into());
        }
        let result = app.opener().reveal_item_in_dir(&path);
        #[cfg(target_os = "linux")]
        let result = result.or_else(|error| match path.parent() {
            Some(parent) if parent.is_dir() => app
                .opener()
                .open_path(parent.to_string_lossy().into_owned(), None::<&str>),
            _ => Err(error),
        });
        result.map_err(|error| format!("无法打开文件管理器：{error}"))
    })
    .await
    .map_err(|error| error.to_string())?
}
