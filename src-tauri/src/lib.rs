//! Glide desktop shell: Tauri setup, application state and window management.
//! All heavy work lives in `studio-core` / `studio-engine`; commands here only
//! validate input, dispatch to background threads and report results.

mod commands;
mod paths;
mod state;
mod windows;

use state::AppState;
use tauri::Manager;

pub fn run() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info,tao=warn,wry=warn")).init();
    studio_engine::sources::ensure_dpi_awareness();

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let state = AppState::load();
            // Recordings folder is readable by the webview (thumbnails, video).
            let root = state.settings.lock().output_dir();
            let _ = std::fs::create_dir_all(&root);
            paths::allow_asset_dir(app.handle(), &root);
            state.probe_in_background();
            app.manage(state);
            Ok(())
        })
        .on_window_event(|window, event| {
            // Closing the main window while recording: finish the recording first.
            if let tauri::WindowEvent::CloseRequested { .. } = event {
                if window.label() == "main" {
                    let state = window.state::<AppState>();
                    let session = state.session.lock().take();
                    if let Some(s) = session {
                        log::warn!("main window closed during recording; finalizing");
                        let _ = s.stop();
                    }
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::app_info,
            commands::list_encoders,
            commands::get_settings,
            commands::save_settings,
            commands::list_sources,
            commands::source_thumbnail,
            commands::list_audio_devices,
            commands::prepare_recording,
            commands::pending_recording,
            commands::cancel_pending,
            commands::start_recording,
            commands::pause_recording,
            commands::resume_recording,
            commands::recording_status,
            commands::stop_recording,
            commands::discard_recording,
            commands::list_projects,
            commands::open_project,
            commands::compute_preview,
            commands::regenerate_zoom,
            commands::save_project,
            commands::rename_project,
            commands::delete_project,
            commands::import_video,
            commands::start_export,
            commands::cancel_export,
            commands::set_background_image,
            commands::reveal_path,
            commands::open_path,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Glide");
}
