use crate::AppState;
use file_trans_core::model::Snapshot;
use std::path::PathBuf;
use tauri::State;

fn error(error: impl std::fmt::Display) -> String {
    error.to_string()
}

#[tauri::command]
pub fn get_snapshot(state: State<'_, AppState>) -> Result<Snapshot, String> {
    state.0.snapshot().map_err(error)
}

#[tauri::command]
pub async fn pair_device(state: State<'_, AppState>, address: String) -> Result<String, String> {
    state
        .0
        .request_pair(
            address
                .trim()
                .parse()
                .map_err(|_| "请输入 IP:端口，例如 192.168.1.20:45678")?,
        )
        .await
        .map_err(error)
}

#[tauri::command]
pub fn decide_pairing(
    state: State<'_, AppState>,
    id: String,
    accepted: bool,
) -> Result<(), String> {
    state.0.decide_pairing(&id, accepted).map_err(error)
}

#[tauri::command]
pub fn decide_file(state: State<'_, AppState>, id: String, accepted: bool) -> Result<(), String> {
    state.0.decide_file(&id, accepted).map_err(error)
}

#[tauri::command]
pub async fn send_text(
    state: State<'_, AppState>,
    peer_id: String,
    text: String,
) -> Result<(), String> {
    state.0.send_text(&peer_id, text).await.map_err(error)
}

#[tauri::command]
pub async fn send_file(
    state: State<'_, AppState>,
    peer_id: String,
    path: String,
) -> Result<String, String> {
    state
        .0
        .send_file(&peer_id, PathBuf::from(path))
        .await
        .map_err(error)
}

#[tauri::command]
pub fn forget_peer(state: State<'_, AppState>, id: String) -> Result<(), String> {
    state.0.forget_peer(&id).map_err(error)
}

#[tauri::command]
pub async fn set_download_dir(state: State<'_, AppState>, path: String) -> Result<(), String> {
    state
        .0
        .set_download_dir(PathBuf::from(path))
        .await
        .map_err(error)
}
