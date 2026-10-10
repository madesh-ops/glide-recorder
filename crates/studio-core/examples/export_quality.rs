//! Export-quality harness: wrap a video in a project with chosen padding / zoom /
//! output size, run the real export, and print the geometry so the result can
//! be compared with an ideal resample of a lossless reference.
//!
//! ```text
//! cargo run --release -p studio-core --example export_quality -- \
//!     <screen.mp4> <work_dir> [--padding 0.08] [--zoom 2] [--height 1080] [--encoder auto] [--quality high]
//! ```

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use studio_core::camera::{ZoomFocus, ZoomSegment, ZoomSettings};
use studio_core::export::{export, ExportRequest};
use studio_core::media::{locate_ffmpeg, probe_encoders, probe_media};
use studio_core::project::{ExportSettings, MediaRefs, Project, Quality, RecordingInfo, SourceInfo, SourceKind};

fn arg<T: std::str::FromStr>(args: &[String], name: &str, default: T) -> T {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let src = PathBuf::from(&args[0]);
    let work = PathBuf::from(&args[1]);
    let padding: f64 = arg(&args, "--padding", 0.08);
    let zoom: f64 = arg(&args, "--zoom", 1.0);
    let height: u32 = arg(&args, "--height", 1080);
    let encoder: String = arg(&args, "--encoder", "auto".to_string());
    let quality = match arg(&args, "--quality", "high".to_string()).as_str() {
        "low" => Quality::Low,
        "best" => Quality::Best,
        _ => Quality::High,
    };

    let ff = locate_ffmpeg().expect("ffmpeg");
    let info = probe_media(&ff.ffprobe, &src).expect("probe");
    std::fs::create_dir_all(&work).unwrap();
    std::fs::copy(&src, work.join("screen.mp4")).unwrap();

    let mut p = Project::new_for_recording(
        "quality".into(),
        0,
        SourceInfo { kind: SourceKind::Imported, label: "q".into(), width: info.width, height: info.height, fps: info.fps.round() as u32, scale_factor: 1.0 },
        MediaRefs { video: "screen.mp4".into(), mic: None, system_audio: None, cursor: None, thumbnail: None },
        RecordingInfo { duration: info.duration, ..Default::default() },
        None,
        ZoomSettings { transition: 0.05, ..Default::default() },
    );
    p.edit.style.padding = padding;
    p.edit.style.shadow_opacity = 0.0;
    p.edit.cursor.visible = false;
    if zoom > 1.0 {
        p.edit.zoom.segments.push(ZoomSegment {
            id: "q".into(),
            start: 0.0,
            end: info.duration,
            level: zoom,
            focus: ZoomFocus::Manual { x: 0.5, y: 0.5 },
            enabled: true,
            auto: false,
            anchor: None,
            anchor_time: None,
        });
    }
    p.save(&work).unwrap();

    let layout = p.layout();
    let (ow, oh) = layout.output_size(height);
    let out = work.join("export.mp4");
    let settings = ExportSettings { height, fps: info.fps.round() as u32, encoder, quality };
    let res = export(&ff, &probe_encoders(&ff), ExportRequest { project_dir: &work, output: &out, settings: Some(settings) }, &AtomicBool::new(false), &mut |_| {})
        .expect("export");
    let r = layout.screen;
    println!(
        "{{\"out_w\":{ow},\"out_h\":{oh},\"rect\":[{},{},{},{}],\"encoder\":\"{}\",\"bytes\":{},\"secs\":{:.2}}}",
        r.x * ow as f64, r.y * oh as f64, r.w * ow as f64, r.h * oh as f64, res.encoder, res.size_bytes, res.elapsed_secs
    );
}
