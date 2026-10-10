//! Project format: a folder containing `project.json`, media and metadata.
//!
//! Editing never touches the media; all decisions live in [`EditState`].

use crate::camera::{generate_segments, ZoomSegment, ZoomSettings};
use crate::cursor::{CursorSettings, CursorTrack};
use crate::layout::{Layout, StyleSettings};
use crate::timeline::Timeline;
use crate::{Error, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Component, Path, PathBuf};

pub const PROJECT_FILE: &str = "project.json";
pub const PROJECT_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SourceKind {
    Display,
    Window,
    Imported,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceInfo {
    pub kind: SourceKind,
    pub label: String,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    /// DPI scale of the captured display (1.0 = 96 DPI).
    #[serde(default = "one")]
    pub scale_factor: f64,
}

fn one() -> f64 {
    1.0
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaRefs {
    pub video: String,
    #[serde(default)]
    pub mic: Option<String>,
    #[serde(default)]
    pub system_audio: Option<String>,
    #[serde(default)]
    pub cursor: Option<String>,
    #[serde(default)]
    pub thumbnail: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct RecordingInfo {
    /// Recorded duration in seconds (pauses excluded).
    pub duration: f64,
    pub frames_written: u64,
    pub frames_dropped: u64,
    pub encoder: String,
    pub pause_count: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AudioMix {
    pub mic_volume: f64,
    pub system_volume: f64,
    pub mic_muted: bool,
    pub system_muted: bool,
}

impl Default for AudioMix {
    fn default() -> Self {
        Self { mic_volume: 1.0, system_volume: 1.0, mic_muted: false, system_muted: false }
    }
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ZoomState {
    #[serde(default)]
    pub settings: ZoomSettings,
    #[serde(default)]
    pub segments: Vec<ZoomSegment>,
}

/// Everything the user can change in the editor. Undo/redo snapshots this.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EditState {
    pub timeline: Timeline,
    #[serde(default)]
    pub zoom: ZoomState,
    #[serde(default)]
    pub cursor: CursorSettings,
    #[serde(default)]
    pub style: StyleSettings,
    #[serde(default)]
    pub audio: AudioMix,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Quality {
    Low,
    #[default]
    High,
    Best,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ExportSettings {
    /// Output height in pixels (width follows the canvas aspect).
    pub height: u32,
    pub fps: u32,
    /// FFmpeg encoder name, or "auto".
    pub encoder: String,
    pub quality: Quality,
}

impl Default for ExportSettings {
    fn default() -> Self {
        Self { height: 1080, fps: 60, encoder: "auto".into(), quality: Quality::High }
    }
}

impl ExportSettings {
    pub fn sanitize(&mut self) {
        self.height = self.height.clamp(240, 4320);
        self.fps = self.fps.clamp(10, 120);
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    pub version: u32,
    pub id: String,
    pub name: String,
    /// Unix milliseconds.
    pub created_at: i64,
    pub source: SourceInfo,
    pub media: MediaRefs,
    #[serde(default)]
    pub recording: RecordingInfo,
    pub edit: EditState,
    #[serde(default)]
    pub export: ExportSettings,
}

impl Project {
    /// A fresh project for a just-finished recording, with auto-zoom applied.
    pub fn new_for_recording(
        name: String,
        created_at: i64,
        source: SourceInfo,
        media: MediaRefs,
        recording: RecordingInfo,
        cursor: Option<&CursorTrack>,
        zoom_settings: ZoomSettings,
    ) -> Self {
        let style = StyleSettings::default();
        let layout = Layout::compute(source.width, source.height, &style);
        let segments = cursor
            .map(|c| generate_segments(c, recording.duration, &layout, &zoom_settings))
            .unwrap_or_default();
        Self {
            version: PROJECT_VERSION,
            id: uuid::Uuid::new_v4().to_string(),
            name,
            created_at,
            edit: EditState {
                timeline: Timeline::new(recording.duration),
                zoom: ZoomState { settings: zoom_settings, segments },
                cursor: CursorSettings::default(),
                style,
                audio: AudioMix::default(),
            },
            source,
            media,
            recording,
            export: ExportSettings::default(),
        }
    }

    /// Clamp all user-editable values into valid ranges.
    pub fn sanitize(&mut self) {
        let e = &mut self.edit;
        if self.recording.duration.is_finite() && self.recording.duration > 0.0 {
            e.timeline.source_duration = self.recording.duration;
        }
        e.timeline.normalize();
        e.zoom.settings.sanitize();
        let dur = self.recording.duration;
        let zs = &e.zoom.settings;
        e.zoom.segments.retain(|s| s.start.is_finite() && s.end.is_finite());
        for s in &mut e.zoom.segments {
            s.start = s.start.clamp(0.0, dur);
            s.end = s.end.clamp(s.start, dur);
            s.level = crate::layout::finite_or(s.level, zs.default_level).clamp(1.0, zs.max_level.max(1.0));
            if let crate::camera::ZoomFocus::Manual { x, y } = &mut s.focus {
                *x = x.clamp(0.0, 1.0);
                *y = y.clamp(0.0, 1.0);
            }
            if s.id.is_empty() {
                s.id = crate::camera::new_segment_id();
            }
        }
        e.zoom.segments.retain(|s| s.end - s.start >= 0.1);
        e.zoom.segments.sort_by(|a, b| a.start.total_cmp(&b.start));
        e.cursor.sanitize();
        e.style.sanitize();
        e.audio.mic_volume = crate::layout::finite_or(e.audio.mic_volume, 1.0).clamp(0.0, 4.0);
        e.audio.system_volume = crate::layout::finite_or(e.audio.system_volume, 1.0).clamp(0.0, 4.0);
        self.export.sanitize();
    }

    pub fn load(dir: &Path) -> Result<Self> {
        let path = dir.join(PROJECT_FILE);
        let text = fs::read_to_string(&path)?;
        let mut p: Project =
            serde_json::from_str(&text).map_err(|source| Error::Json { path: path.clone(), source })?;
        if p.version > PROJECT_VERSION {
            return Err(Error::UnsupportedVersion { found: p.version, supported: PROJECT_VERSION });
        }
        // Validate media references up front so later code can trust them.
        resolve_media(dir, &p.media.video)?;
        for r in [&p.media.mic, &p.media.system_audio, &p.media.cursor, &p.media.thumbnail].into_iter().flatten() {
            resolve_media(dir, r)?;
        }
        p.sanitize();
        Ok(p)
    }

    /// Atomic save: write a temp file, then rename over the old one.
    pub fn save(&self, dir: &Path) -> Result<()> {
        write_json_atomic(&dir.join(PROJECT_FILE), self)
    }

    pub fn load_cursor(&self, dir: &Path) -> Result<Option<CursorTrack>> {
        let Some(rel) = &self.media.cursor else { return Ok(None) };
        let path = resolve_media(dir, rel)?;
        if !path.exists() {
            return Ok(None);
        }
        let text = fs::read_to_string(&path)?;
        let track: CursorTrack = serde_json::from_str(&text).map_err(|source| Error::Json { path, source })?;
        Ok(Some(track))
    }

    pub fn layout(&self) -> Layout {
        Layout::compute(self.source.width, self.source.height, &self.edit.style)
    }
}

pub fn write_json_atomic<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let tmp = path.with_extension("json.tmp");
    let data = serde_json::to_vec_pretty(value).map_err(|source| Error::Json { path: path.to_path_buf(), source })?;
    fs::write(&tmp, data)?;
    fs::rename(&tmp, path)?;
    Ok(())
}

/// Resolve a project-relative media path, rejecting anything that could escape
/// the project folder (absolute paths, `..`, drive prefixes).
pub fn resolve_media(dir: &Path, rel: &str) -> Result<PathBuf> {
    let p = Path::new(rel);
    if rel.is_empty() || p.is_absolute() {
        return Err(Error::PathEscape(rel.to_string()));
    }
    for c in p.components() {
        match c {
            Component::Normal(_) | Component::CurDir => {}
            _ => return Err(Error::PathEscape(rel.to_string())),
        }
    }
    Ok(dir.join(p))
}

/// Summary for the dashboard.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectSummary {
    pub id: String,
    pub name: String,
    pub dir: PathBuf,
    pub created_at: i64,
    pub duration: f64,
    pub width: u32,
    pub height: u32,
    pub thumbnail: Option<PathBuf>,
}

/// List projects directly inside `root`, newest first. Unreadable folders are skipped.
pub fn list_projects(root: &Path) -> Vec<ProjectSummary> {
    let Ok(entries) = fs::read_dir(root) else { return Vec::new() };
    let mut out: Vec<ProjectSummary> = entries
        .flatten()
        .filter(|e| e.path().join(PROJECT_FILE).is_file())
        .filter_map(|e| {
            let dir = e.path();
            let p = Project::load(&dir).map_err(|err| log::warn!("skipping {}: {err}", dir.display())).ok()?;
            let thumbnail = p
                .media
                .thumbnail
                .as_deref()
                .and_then(|t| resolve_media(&dir, t).ok())
                .filter(|t| t.is_file());
            Some(ProjectSummary {
                id: p.id,
                name: p.name,
                created_at: p.created_at,
                duration: p.edit.timeline.output_duration(),
                width: p.source.width,
                height: p.source.height,
                thumbnail,
                dir,
            })
        })
        .collect();
    out.sort_by_key(|p| std::cmp::Reverse(p.created_at));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cursor::{ClickEvent, CursorSample, MouseButton};

    fn sample_project() -> Project {
        let track = CursorTrack {
            version: 1,
            source_width: 1920,
            source_height: 1080,
            samples: vec![CursorSample { t: 0.0, x: 10.0, y: 10.0, shape: 0, visible: true }],
            clicks: vec![ClickEvent { t: 3.0, x: 500.0, y: 400.0, button: MouseButton::Left, up: None }],
            shapes: vec![],
        };
        Project::new_for_recording(
            "Demo".into(),
            1_700_000_000_000,
            SourceInfo { kind: SourceKind::Display, label: "Display 1".into(), width: 1920, height: 1080, fps: 60, scale_factor: 1.0 },
            MediaRefs { video: "screen.mp4".into(), mic: Some("mic.wav".into()), system_audio: None, cursor: Some("cursor.json".into()), thumbnail: None },
            RecordingInfo { duration: 10.0, ..Default::default() },
            Some(&track),
            ZoomSettings::default(),
        )
    }

    #[test]
    fn new_project_has_auto_zoom_from_clicks() {
        let p = sample_project();
        assert_eq!(p.edit.zoom.segments.len(), 1);
        assert!((p.edit.timeline.output_duration() - 10.0).abs() < 1e-9);
    }

    #[test]
    fn save_and_reload_preserves_edit_state() {
        let dir = tempfile::tempdir().unwrap();
        let mut p = sample_project();
        p.edit.timeline.cut(2.0, 4.0);
        p.edit.zoom.segments[0].level = 3.0;
        p.edit.cursor.scale = 2.5;
        p.edit.style.padding = 0.12;
        p.save(dir.path()).unwrap();
        let q = Project::load(dir.path()).unwrap();
        assert_eq!(p, q);
    }

    #[test]
    fn rejects_escaping_media_paths() {
        let d = Path::new("C:\\proj");
        assert!(resolve_media(d, "screen.mp4").is_ok());
        assert!(resolve_media(d, "sub/a.png").is_ok());
        assert!(resolve_media(d, "../x.mp4").is_err());
        assert!(resolve_media(d, "C:\\Windows\\x").is_err());
        assert!(resolve_media(d, "\\\\server\\share").is_err());
        assert!(resolve_media(d, "").is_err());

        let dir = tempfile::tempdir().unwrap();
        let mut p = sample_project();
        p.media.video = "..\\..\\evil.mp4".into();
        p.save(dir.path()).unwrap();
        assert!(matches!(Project::load(dir.path()), Err(Error::PathEscape(_))));
    }

    #[test]
    fn rejects_future_versions_and_sanitizes_values() {
        let dir = tempfile::tempdir().unwrap();
        let mut p = sample_project();
        p.version = 99;
        p.save(dir.path()).unwrap();
        assert!(matches!(Project::load(dir.path()), Err(Error::UnsupportedVersion { .. })));

        let mut p = sample_project();
        p.edit.zoom.segments[0].level = 100.0;
        p.edit.zoom.segments[0].end = 1e9;
        p.edit.cursor.scale = -3.0;
        p.sanitize();
        assert!(p.edit.zoom.segments[0].level <= p.edit.zoom.settings.max_level);
        assert!(p.edit.zoom.segments[0].end <= 10.0);
        assert!(p.edit.cursor.scale > 0.0);
    }

    #[test]
    fn lists_projects_newest_first() {
        let root = tempfile::tempdir().unwrap();
        for (i, name) in ["a", "b"].iter().enumerate() {
            let d = root.path().join(name);
            fs::create_dir(&d).unwrap();
            let mut p = sample_project();
            p.created_at += i as i64;
            p.name = name.to_string();
            p.save(&d).unwrap();
        }
        fs::create_dir(root.path().join("junk")).unwrap();
        fs::write(root.path().join("junk").join(PROJECT_FILE), "{not json").unwrap();
        let list = list_projects(root.path());
        assert_eq!(list.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), ["b", "a"]);
    }
}
