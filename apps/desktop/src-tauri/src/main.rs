#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;

use file_trans_core::{Engine, EngineConfig};
use file_trans_identity::Identity;
use std::{path::PathBuf, sync::Arc};
use tauri::Manager;

pub struct AppState(Arc<Engine>);

fn main() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .setup(|app| {
            let data = std::env::var_os("FILE_TRANS_DATA_DIR")
                .map(PathBuf::from)
                .unwrap_or(app.path().app_data_dir()?);
            let downloads = std::env::var_os("FILE_TRANS_DOWNLOAD_DIR")
                .map(PathBuf::from)
                .unwrap_or(app.path().download_dir()?.join("File Trans"));
            let name = std::env::var("FILE_TRANS_DEVICE_NAME").unwrap_or_else(|_| {
                hostname::get()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|_| "我的电脑".into())
            });
            let identity = Identity::load_or_create(&data)?;
            let engine = tauri::async_runtime::block_on(Engine::start(
                EngineConfig::new(data, downloads, name),
                identity,
            ))?;
            app.manage(AppState(engine));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_snapshot,
            commands::pair_device,
            commands::decide_pairing,
            commands::decide_file,
            commands::send_text,
            commands::send_file,
            commands::forget_peer,
            commands::set_download_dir
        ])
        .build(tauri::generate_context!())
        .expect("File Trans 启动失败，请检查系统凭据库、本地网络和接收目录权限");
    app.run(|app, event| {
        if matches!(event, tauri::RunEvent::Exit) {
            tauri::async_runtime::block_on(app.state::<AppState>().0.shutdown());
        }
    });
}
