use crate::AppState;
use file_trans_core::transfers::{Direction, TransferKey};
use tauri::{AppHandle, State};
use tauri_plugin_opener::OpenerExt;

#[tauri::command]
pub async fn open_received_location(
    app: AppHandle,
    state: State<'_, AppState>,
    peer_id: String,
    id: String,
) -> Result<(), String> {
    let engine = state.0.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let path = engine
            .local_attachment_path(&TransferKey {
                id,
                peer_id,
                direction: Direction::Incoming,
            })
            .map_err(|error| format!("{error:#}"))?;
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
