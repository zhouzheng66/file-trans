use crate::AppState;
use file_trans_core::model::{NetworkSettings, Route, Snapshot};
use file_trans_core::transfers::TransferKey;
use std::path::PathBuf;
use tauri::State;

fn error(error: impl std::fmt::Display) -> String {
    format!("{error:#}")
}

#[tauri::command]
pub async fn set_network_settings(
    state: State<'_, AppState>,
    settings: NetworkSettings,
) -> Result<(), String> {
    state.0.set_network_settings(settings).await.map_err(error)
}

#[tauri::command]
pub async fn create_invite(state: State<'_, AppState>) -> Result<(), String> {
    state.0.create_invite().await.map_err(error)
}

#[tauri::command]
pub async fn pair_by_code(state: State<'_, AppState>, code: String) -> Result<String, String> {
    state.0.request_pair_code(code).await.map_err(error)
}

#[tauri::command]
pub fn get_snapshot(state: State<'_, AppState>) -> Result<Snapshot, String> {
    state.0.snapshot().map_err(error)
}

#[tauri::command]
pub async fn refresh_peer_connections(
    state: State<'_, AppState>,
) -> Result<std::collections::HashMap<String, Option<Route>>, String> {
    state.0.refresh_peer_connections().await.map_err(error)
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
pub fn decide_file(
    state: State<'_, AppState>,
    id: String,
    peer_id: Option<String>,
    accepted: bool,
) -> Result<(), String> {
    match peer_id {
        Some(peer_id) => state.0.decide_file_from(&peer_id, &id, accepted),
        None => state.0.decide_file(&id, accepted),
    }
    .map_err(error)
}

#[tauri::command]
pub async fn pause_transfer(state: State<'_, AppState>, key: TransferKey) -> Result<(), String> {
    state.0.pause_transfer(key).await.map_err(error)
}

#[tauri::command]
pub async fn resume_transfer(state: State<'_, AppState>, key: TransferKey) -> Result<(), String> {
    state.0.resume_transfer(key).await.map_err(error)
}

#[tauri::command]
pub async fn cancel_transfer(state: State<'_, AppState>, key: TransferKey) -> Result<(), String> {
    state.0.cancel_transfer(key).await.map_err(error)
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
pub async fn send_path(
    state: State<'_, AppState>,
    peer_id: String,
    path: String,
) -> Result<String, String> {
    state
        .0
        .send_path(&peer_id, PathBuf::from(path))
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
