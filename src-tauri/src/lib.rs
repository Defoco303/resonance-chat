mod packets;
mod protocol;

use tauri::{AppHandle, Manager, Window, WindowEvent};
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
            if window.label() == "main" {
                packets::capture::request_stop();
                packets::capture::stop_driver();
                if let Some(dps_window) = window.app_handle().get_webview_window("dps") {
                    let _ = dps_window.close();
                }
            }
        }
        _ => {}
    }
}

#[tauri::command]
fn close_dps_window(app: AppHandle) -> Result<(), String> {
    if let Some(dps_window) = app.get_webview_window("dps") {
        dps_window.destroy().map_err(|err| err.to_string())?;
    }
    Ok(())
}

#[tauri::command]
fn get_boss_list() -> Vec<packets::dps::BossEntry> {
    packets::dps::get_boss_list()
}

#[tauri::command]
fn set_boss_list(entries: Vec<packets::dps::BossEntry>) {
    packets::dps::set_boss_list(entries);
}

#[tauri::command]
fn reset_boss_list() -> Vec<packets::dps::BossEntry> {
    packets::dps::reset_boss_list()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let result = tauri::Builder::default()
        .plugin(tauri_plugin_window_state::Builder::default().build())
        .invoke_handler(tauri::generate_handler![
            close_dps_window,
            get_boss_list,
            set_boss_list,
            reset_boss_list
        ])
        .on_window_event(on_window_event)
        .setup(|app| {
            let handle = app.handle().clone();
            std::thread::spawn(move || {
                packets::capture::start_capture(handle);
            });
            Ok(())
        })
        .run(tauri::generate_context!());

    packets::capture::request_stop();
    packets::capture::stop_driver();

    result.expect("error while running tauri application");
}

