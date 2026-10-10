use parking_lot::Mutex;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, OnceLock};
use studio_core::media::{locate_ffmpeg, probe_encoders, EncoderInfo, FfmpegPaths};
use studio_core::settings::AppSettings;
use studio_engine::session::{RecordOptions, RecordingSession};

pub struct AppState {
    pub settings: Mutex<AppSettings>,
    pub settings_path: PathBuf,
    ffmpeg: OnceLock<Result<FfmpegPaths, String>>,
    encoders: Mutex<Option<Vec<EncoderInfo>>>,
    pub session: Mutex<Option<Arc<RecordingSession>>>,
    pub pending: Mutex<Option<RecordOptions>>,
    pub exports: Mutex<HashMap<String, Arc<AtomicBool>>>,
}

impl AppState {
    pub fn load() -> Self {
        let settings_path = AppSettings::config_path().unwrap_or_else(|| PathBuf::from("settings.json"));
        let settings = AppSettings::load_from(&settings_path);
        Self {
            settings: Mutex::new(settings),
            settings_path,
            ffmpeg: OnceLock::new(),
            encoders: Mutex::new(None),
            session: Mutex::new(None),
            pending: Mutex::new(None),
            exports: Mutex::new(HashMap::new()),
        }
    }

    pub fn ffmpeg(&self) -> Result<FfmpegPaths, String> {
        self.ffmpeg.get_or_init(|| locate_ffmpeg().map_err(|e| e.to_string())).clone()
    }

    /// Working encoders (probed once, by actually encoding a few frames).
    pub fn encoders(&self) -> Vec<EncoderInfo> {
        if let Some(e) = self.encoders.lock().as_ref() {
            return e.clone();
        }
        let list = match self.ffmpeg() {
            Ok(ff) => probe_encoders(&ff),
            Err(_) => Vec::new(),
        };
        log::info!("working encoders: {:?}", list.iter().map(|e| &e.name).collect::<Vec<_>>());
        *self.encoders.lock() = Some(list.clone());
        list
    }

    pub fn probe_in_background(&self) {
        // Locating FFmpeg is cheap; do it now so the UI can report problems early.
        let _ = self.ffmpeg();
    }
}
