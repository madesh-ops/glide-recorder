//! System-wide shortcuts that work while Glide is hidden during recording.
//!
//! * Ctrl+Alt+R — stop the current recording, or open "New recording".
//! * Ctrl+Alt+P — pause / resume the current recording.

use crate::state::AppState;
use studio_core::recording::RecordingState;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};

pub const RECORD_LABEL: &str = "Ctrl+Alt+R";
pub const PAUSE_LABEL: &str = "Ctrl+Alt+P";

fn record() -> Shortcut {
    Shortcut::new(Some(Modifiers::CONTROL | Modifiers::ALT), Code::KeyR)
}
fn pause() -> Shortcut {
    Shortcut::new(Some(Modifiers::CONTROL | Modifiers::ALT), Code::KeyP)
}

pub fn plugin() -> tauri::plugin::TauriPlugin<tauri::Wry> {
    tauri_plugin_global_shortcut::Builder::new()
        .with_handler(|app, shortcut, event| {
            if event.state != ShortcutState::Pressed {
                return;
            }
            if shortcut.id() == record().id() {
                on_record(app);
            } else if shortcut.id() == pause().id() {
                on_pause(app);
            }
        })
        .build()
}

/// Register the shortcuts. Failure (e.g. another app owns the combination) is
/// logged and otherwise ignored; the on-screen controls still work.
pub fn register(app: &AppHandle) {
    for (s, label) in [(record(), RECORD_LABEL), (pause(), PAUSE_LABEL)] {
        if let Err(e) = app.global_shortcut().register(s) {
            log::warn!("could not register {label}: {e}");
        }
    }
}

fn on_record(app: &AppHandle) {
    let state = app.state::<AppState>();
    let session = state.session.lock().clone();
    match session {
        Some(s) if matches!(s.state(), RecordingState::Recording | RecordingState::Paused) => {
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                if let Err(e) = crate::commands::finish_recording(app).await {
                    log::error!("stop via shortcut failed: {e}");
                }
            });
        }
        Some(_) => {} // preparing or finalizing: ignore
        None if state.pending.lock().is_some() => {} // countdown running
        None => {
            crate::windows::show_main(app);
            let _ = app.emit("open-new-recording", ());
        }
    }
}

fn on_pause(app: &AppHandle) {
    let session = app.state::<AppState>().session.lock().clone();
    if let Some(s) = session {
        let r = match s.state() {
            RecordingState::Recording => s.pause(),
            RecordingState::Paused => s.resume(),
            _ => Ok(()),
        };
        if let Err(e) = r {
            log::warn!("pause via shortcut: {e}");
        }
    }
}
