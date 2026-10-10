//! Persistent application settings (`%APPDATA%\Glide\settings.json`).

use crate::camera::ZoomSettings;
use crate::project::write_json_atomic;
use crate::Result;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const APP_DIR_NAME: &str = "Glide";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AppSettings {
    /// Where new recordings are stored. `None` = `Videos\Glide`.
    pub output_dir: Option<PathBuf>,
    pub fps: u32,
    /// Recording encoder ("auto" or an FFmpeg encoder name).
    pub encoder: String,
    pub record_mic: bool,
    /// cpal device name; `None` = system default.
    pub mic_device: Option<String>,
    pub record_system_audio: bool,
    pub countdown_secs: u32,
    pub hide_window_while_recording: bool,
    pub default_zoom: ZoomSettings,
    /// The last recording request (source, mic, audio), reused by the
    /// "start recording" shortcut. Opaque to the core crate.
    pub last_recording: Option<serde_json::Value>,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            output_dir: None,
            fps: 60,
            encoder: "auto".into(),
            record_mic: false,
            mic_device: None,
            record_system_audio: true,
            countdown_secs: 3,
            hide_window_while_recording: true,
            default_zoom: ZoomSettings::default(),
            last_recording: None,
        }
    }
}

impl AppSettings {
    pub fn config_path() -> Option<PathBuf> {
        dirs::config_dir().map(|d| d.join(APP_DIR_NAME).join("settings.json"))
    }

    pub fn default_output_dir() -> PathBuf {
        dirs::video_dir()
            .or_else(dirs::home_dir)
            .unwrap_or_else(std::env::temp_dir)
            .join("Glide")
    }

    pub fn output_dir(&self) -> PathBuf {
        self.output_dir.clone().unwrap_or_else(Self::default_output_dir)
    }

    pub fn sanitize(&mut self) {
        self.fps = match self.fps {
            0..=24 => 24,
            25..=30 => 30,
            _ => 60,
        };
        self.countdown_secs = self.countdown_secs.min(10);
        if self.encoder.trim().is_empty() {
            self.encoder = "auto".into();
        }
        if let Some(d) = &self.output_dir {
            if !d.is_absolute() {
                self.output_dir = None;
            }
        }
        self.default_zoom.sanitize();
    }

    /// Load settings; a missing or corrupt file yields defaults (corrupt files
    /// are kept as `settings.json.bad` for inspection).
    pub fn load_from(path: &Path) -> Self {
        let mut s = match std::fs::read_to_string(path) {
            Ok(text) => serde_json::from_str::<AppSettings>(&text).unwrap_or_else(|e| {
                log::warn!("settings file is invalid ({e}); using defaults");
                let _ = std::fs::rename(path, path.with_extension("json.bad"));
                AppSettings::default()
            }),
            Err(_) => AppSettings::default(),
        };
        s.sanitize();
        s
    }

    pub fn save_to(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        write_json_atomic(path, self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s").join("settings.json");
        assert_eq!(AppSettings::load_from(&path), AppSettings::default());
        let s = AppSettings { fps: 30, record_mic: true, mic_device: Some("USB Mic".into()), ..Default::default() };
        s.save_to(&path).unwrap();
        assert_eq!(AppSettings::load_from(&path), s);
    }

    #[test]
    fn corrupt_file_falls_back_and_is_preserved() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(&path, "{{{").unwrap();
        assert_eq!(AppSettings::load_from(&path), AppSettings::default());
        assert!(path.with_extension("json.bad").exists());
    }

    #[test]
    fn partial_files_use_defaults_for_missing_fields_and_sanitize() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(&path, r#"{"fps": 45, "outputDir": "relative\\path"}"#).unwrap();
        let s = AppSettings::load_from(&path);
        assert_eq!(s.fps, 60);
        assert_eq!(s.output_dir, None);
        assert!(s.record_system_audio);
    }
}
