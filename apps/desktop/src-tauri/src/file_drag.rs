use crate::AppState;
use file_trans_core::transfers::TransferKey;
#[cfg(any(target_os = "macos", target_os = "windows"))]
use std::path::PathBuf;
use tauri::{async_runtime, State, WebviewWindow};

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
use linux::start_copy;

#[tauri::command]
pub async fn start_file_drag(
    state: State<'_, AppState>,
    window: WebviewWindow,
    key: TransferKey,
) -> Result<(), String> {
    let engine = state.0.clone();
    let path = async_runtime::spawn_blocking(move || engine.local_attachment_path(&key))
        .await
        .map_err(|error| error.to_string())?
        .map_err(|error| format!("{error:#}"))?;

    let (done, mut result) = async_runtime::channel(1);
    let source_window = window.clone();
    window
        .run_on_main_thread(move || {
            let finished = done.clone();
            let started = start_copy(&source_window, path, move || {
                let _ = finished.try_send(Ok(()));
            });
            if let Err(error) = started {
                let _ = done.try_send(Err(error));
            }
        })
        .map_err(|error| error.to_string())?;
    // macOS 和 GTK 在拖动开始后立即返回，等待原生会话结束再恢复窗口的拖入处理。
    result.recv().await.ok_or("系统拖拽会话已中断")?
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn start_copy(
    window: &WebviewWindow,
    path: PathBuf,
    finished: impl Fn() + Send + 'static,
) -> Result<(), String> {
    drag::start_drag(
        window,
        drag::DragItem::Files(vec![path]),
        drag::Image::Raw(include_bytes!("../icons/32x32.png").to_vec()),
        move |_, _| finished(),
        drag::Options {
            mode: drag::DragMode::Copy,
            ..Default::default()
        },
    )
    .map_err(|error| format!("无法拖出文件：{error}"))
}
