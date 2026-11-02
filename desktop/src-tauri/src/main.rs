#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod panes;

use tauri::{Manager, RunEvent};

fn main() {
    let builder = tauri::Builder::default();
    #[cfg(target_os = "macos")]
    let builder = builder.activate_ignoring_other_apps(false);
    let app = builder
        .manage(panes::Panes::default())
        .invoke_handler(tauri::generate_handler![
            panes::open_pane,
            panes::write_pane,
            panes::resize_pane,
            panes::close_pane,
            panes::pane_info,
            panes::log_frontend,
        ])
        .build(tauri::generate_context!())
        .expect("failed to build GridBash desktop");
    app.run(|handle, event| {
        if let RunEvent::Exit = event {
            handle.state::<panes::Panes>().close_all();
        }
    });
}
