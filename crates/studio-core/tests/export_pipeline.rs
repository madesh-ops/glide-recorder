//! End-to-end export test with real FFmpeg: synthetic source video + audio +
//! cursor clicks → project → export (trim, cut, speed, zoom) → validated MP4.
//! Skipped (with a message) when FFmpeg is not installed.

use std::sync::atomic::AtomicBool;
use studio_core::camera::ZoomSettings;
use studio_core::cursor::{ClickEvent, CursorSample, CursorTrack, MouseButton};
use studio_core::export::{export, ExportRequest};
use studio_core::media::{command, locate_ffmpeg, probe_encoders, probe_media};
use studio_core::project::{write_json_atomic, ExportSettings, MediaRefs, Project, Quality, RecordingInfo, SourceInfo, SourceKind};

fn run(ffmpeg: &std::path::Path, args: &[&str]) {
    let st = command(ffmpeg).args(["-hide_banner", "-loglevel", "error", "-y"]).args(args).status().unwrap();
    assert!(st.success(), "ffmpeg {args:?} failed");
}

#[test]
fn export_applies_edits_and_produces_valid_mp4() {
    let Ok(ff) = locate_ffmpeg() else {
        eprintln!("FFmpeg not found; skipping export integration test");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path();
    // 6 s, 640x360 @ 30 fps source and a 6 s tone.
    run(&ff.ffmpeg, &["-f", "lavfi", "-i", "testsrc2=size=640x360:rate=30", "-t", "6", "-pix_fmt", "yuv420p", "-c:v", "libx264", d.join("screen.mp4").to_str().unwrap()]);
    run(&ff.ffmpeg, &["-f", "lavfi", "-i", "sine=frequency=440:sample_rate=48000", "-t", "6", "-ac", "2", d.join("mic.wav").to_str().unwrap()]);

    let track = CursorTrack {
        version: 1,
        source_width: 640,
        source_height: 360,
        samples: (0..=60).map(|i| CursorSample { t: i as f64 * 0.1, x: 100.0 + i as f32 * 5.0, y: 120.0, shape: u32::MAX, visible: true }).collect(),
        clicks: vec![ClickEvent { t: 2.0, x: 200.0, y: 120.0, button: MouseButton::Left, up: Some(2.08) }],
        shapes: vec![],
    };
    write_json_atomic(&d.join("cursor.json"), &track).unwrap();
    let mut project = Project::new_for_recording(
        "Integration".into(),
        0,
        SourceInfo { kind: SourceKind::Display, label: "Test".into(), width: 640, height: 360, fps: 30, scale_factor: 1.0 },
        MediaRefs { video: "screen.mp4".into(), mic: Some("mic.wav".into()), system_audio: None, cursor: Some("cursor.json".into()), thumbnail: None },
        RecordingInfo { duration: 6.0, ..Default::default() },
        Some(&track),
        ZoomSettings::default(),
    );
    assert_eq!(project.edit.zoom.segments.len(), 1, "click produced an automatic zoom");
    // Trim 0.5 s from the start, cut 3–4 s, play the rest at 2×.
    let tl = &mut project.edit.timeline;
    tl.clips[0].start = 0.5;
    tl.cut(3.0, 4.0);
    tl.clips[1].speed = 2.0;
    let expected = tl.output_duration(); // 2.5 + 1.0 = 3.5 s
    assert!((expected - 3.5).abs() < 1e-9);
    project.save(d).unwrap();

    let encoders = probe_encoders(&ff);
    let out = d.join("out.mp4");
    let settings = ExportSettings { height: 360, fps: 30, encoder: "libx264".into(), quality: Quality::Low };
    let mut progress_calls = 0;
    let res = export(
        &ff,
        &encoders,
        ExportRequest { project_dir: d, output: &out, settings: Some(settings) },
        &AtomicBool::new(false),
        &mut |_| progress_calls += 1,
    )
    .expect("export succeeds");
    assert!(progress_calls > 0);
    assert_eq!(res.frames, 105);
    let info = probe_media(&ff.ffprobe, &out).unwrap();
    assert_eq!((info.width, info.height), (640, 360));
    assert!((info.duration - expected).abs() < 0.1, "duration {}", info.duration);
    assert!(info.has_audio, "audio track present");
    assert!(!d.join("out.partial.mp4").exists(), "temporary file cleaned up");
}

#[test]
fn cancelled_export_leaves_no_files() {
    let Ok(ff) = locate_ffmpeg() else { return };
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path();
    run(&ff.ffmpeg, &["-f", "lavfi", "-i", "testsrc2=size=320x180:rate=30", "-t", "3", "-pix_fmt", "yuv420p", "-c:v", "libx264", d.join("screen.mp4").to_str().unwrap()]);
    let project = Project::new_for_recording(
        "Cancel".into(),
        0,
        SourceInfo { kind: SourceKind::Imported, label: "Test".into(), width: 320, height: 180, fps: 30, scale_factor: 1.0 },
        MediaRefs { video: "screen.mp4".into(), mic: None, system_audio: None, cursor: None, thumbnail: None },
        RecordingInfo { duration: 3.0, ..Default::default() },
        None,
        ZoomSettings::default(),
    );
    project.save(d).unwrap();
    let out = d.join("out.mp4");
    let cancel = AtomicBool::new(true);
    let err = export(&ff, &probe_encoders(&ff), ExportRequest { project_dir: d, output: &out, settings: None }, &cancel, &mut |_| {});
    assert!(matches!(err, Err(studio_core::Error::Cancelled)));
    assert!(!out.exists());
    assert!(!d.join("out.partial.mp4").exists());
}
