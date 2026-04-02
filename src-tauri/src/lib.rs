mod packets;

use tauri::{Manager, Window, WindowEvent};
use tauri_plugin_window_state::{AppHandleExt, StateFlags};

fn save_window_state(window: &Window) {
    if let Err(err) = window.app_handle().save_window_state(StateFlags::all()) {
        eprintln!(
            "failed to save window state for {}: {}",
            window.label(),
            err
        );
    }
}

fn on_window_event(window: &Window, event: &WindowEvent) {
    match event {
        WindowEvent::Moved(_) | WindowEvent::Resized(_) | WindowEvent::Focused(false) => {
            save_window_state(window);
        }
        WindowEvent::CloseRequested { .. } => {
            save_window_state(window);
        }
        _ => {}
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_window_state::Builder::default().build())
        .on_window_event(on_window_event)
        .setup(|app| {
            let handle = app.handle().clone();
            std::thread::spawn(move || {
                packets::capture::start_capture(handle);
            });
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

