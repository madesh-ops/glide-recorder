//! Data the editor needs to render a live preview that matches export exactly.
//!
//! The UI plays `screen.mp4` in a `<video>` element and applies the camera /
//! cursor from these Rust-computed tracks, so preview and export share one
//! source of truth for geometry and motion.

use crate::camera::{generate_segments, CameraPath, ZoomSegment};
use crate::cursor::{ClickEvent, CursorPath, CursorShape, CursorTrack};
use crate::layout::Layout;
use crate::project::Project;
use serde::Serialize;

pub const PREVIEW_RATE: f64 = 60.0;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewModel {
    pub layout: Layout,
    pub rate: f64,
    /// Flat `[x, y, zoom]` per frame (source time = index / rate).
    pub camera: Vec<f32>,
    /// Flat `[x, y, shape, opacity]` per frame in source pixels; shape −1 = default arrow.
    pub cursor: Vec<f32>,
    pub clicks: Vec<ClickEvent>,
    pub shapes: Vec<CursorShape>,
}

pub fn build_preview(project: &Project, track: Option<&CursorTrack>) -> PreviewModel {
    let layout = project.layout();
    let duration = project.recording.duration;
    let empty = CursorTrack::default();
    let track = track.unwrap_or(&empty);
    let cam = CameraPath::compute(
        &project.edit.zoom.segments,
        track,
        &layout,
        &project.edit.zoom.settings,
        duration,
        PREVIEW_RATE,
    );
    let cur = CursorPath::compute(track, &project.edit.cursor, duration, PREVIEW_RATE);
    let mut camera = Vec::with_capacity(cam.frames.len() * 3);
    for f in &cam.frames {
        camera.extend_from_slice(&[f.x as f32, f.y as f32, f.zoom as f32]);
    }
    let mut cursor = Vec::with_capacity(cur.frames.len() * 4);
    for f in &cur.frames {
        let shape = if (f.shape as usize) < track.shapes.len() { f.shape as f32 } else { -1.0 };
        cursor.extend_from_slice(&[f.x as f32, f.y as f32, shape, f.opacity as f32]);
    }
    PreviewModel {
        layout,
        rate: PREVIEW_RATE,
        camera,
        cursor,
        clicks: track.clicks.clone(),
        shapes: track.shapes.clone(),
    }
}

/// Re-run automatic zoom generation. Segments the user created or edited
/// (`auto == false`) are kept; generated ones overlapping them are dropped.
pub fn regenerate_auto_segments(project: &Project, track: Option<&CursorTrack>) -> Vec<ZoomSegment> {
    let manual: Vec<ZoomSegment> = project.edit.zoom.segments.iter().filter(|s| !s.auto).cloned().collect();
    let Some(track) = track else { return manual };
    let generated = generate_segments(track, project.recording.duration, &project.layout(), &project.edit.zoom.settings);
    let mut out = manual.clone();
    out.extend(
        generated
            .into_iter()
            .filter(|g| !manual.iter().any(|m| g.start < m.end && m.start < g.end)),
    );
    out.sort_by(|a, b| a.start.total_cmp(&b.start));
    out
}
