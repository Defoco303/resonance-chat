mod packets;
mod protocol;

use tauri::{AppHandle, Emitter, Manager, Window, WindowEvent};
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
fn get_diag_logging() -> bool {
    packets::diag::saved_setting()
}

#[tauri::command]
fn set_diag_logging(enabled: bool) -> bool {
    packets::diag::set_enabled(enabled)
}

#[tauri::command]
fn reset_dps_meter(app: AppHandle, state: tauri::State<packets::dps::DpsState>) {
    let mut meter = state.lock();
    meter.reset_manually();
    packets::capture::emit_encounter_ends(&app, &mut meter);
    let _ = app.emit("dps-meter", meter.snapshot());
}

/// 戦闘中のプレイヤー詳細・比較用。boss_only ならボス本体へのダメージだけ
#[tauri::command]
fn get_dps_details(
    state: tauri::State<packets::dps::DpsState>,
    boss_only: bool,
) -> Vec<packets::dps::PlayerDetail> {
    state.lock().player_details(boss_only)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let result = tauri::Builder::default()
        .plugin(tauri_plugin_window_state::Builder::default().build())
        .manage(packets::dps::DpsState::default())
        .invoke_handler(tauri::generate_handler![
            close_dps_window,
            reset_dps_meter,
            get_dps_details,
            get_diag_logging,
            set_diag_logging
        ])
        .on_window_event(on_window_event)
        .setup(|app| {
            packets::diag::init(app.path().app_log_dir().ok(), app.path().app_config_dir().ok());
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

