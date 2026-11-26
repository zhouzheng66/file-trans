use gtk::{gdk, gio, prelude::*, TargetList};
use std::{cell::RefCell, path::PathBuf, rc::Rc};
use tauri::WebviewWindow;

pub(super) fn start_copy(
    window: &WebviewWindow,
    path: PathBuf,
    finished: impl Fn() + 'static,
) -> Result<(), String> {
    let window = window.gtk_window().map_err(|error| error.to_string())?;
    let targets = TargetList::new(&[]);
    targets.add_uri_targets(0);
    let context = window
        .drag_begin_with_coordinates(&targets, gdk::DragAction::COPY, 1, None, -1, -1)
        .ok_or("系统未能启动文件拖拽")?;

    // 交给 GIO 编码特殊字符，并在目标实际读取 URI 时提供本地文件。
    let uri = gio::File::for_path(path).uri();
    let session = context.clone();
    let provide = window.connect_drag_data_get(move |_, drag, data, _, _| {
        if drag == &session {
            data.set_uris(&[uri.as_str()]);
        }
    });

    let handlers = Rc::new(RefCell::new(vec![provide]));
    let cleanup = handlers.clone();
    let ended = window.connect_drag_end(move |window, drag| {
        if drag == &context {
            for handler in cleanup.borrow_mut().drain(..) {
                window.disconnect(handler);
            }
            finished();
        }
    });
    handlers.borrow_mut().push(ended);
    Ok(())
}
