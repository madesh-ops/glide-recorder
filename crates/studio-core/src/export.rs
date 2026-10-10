//! Export: decode → time-map → composite → encode, streaming one frame at a time.

use crate::camera::CameraPath;
use crate::cursor::{active_clicks, CursorPath, CursorTrack};
use crate::geometry::Vec2;
use crate::media::{
    choose_encoder, probe_media, raw_input_args, video_codec_args, EncoderInfo, FfmpegPaths, FfmpegReader,
    FfmpegWriter, VideoEncodeSpec,
};
use crate::project::{resolve_media, ExportSettings, Project};
use crate::render::{Compositor, FrameInput};
use crate::timeline::Timeline;
use crate::{Error, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportProgress {
    pub frame: u64,
    pub total_frames: u64,
    pub fps: f64,
    pub eta_secs: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportResult {
    pub output: PathBuf,
    pub duration: f64,
    pub frames: u64,
    pub width: u32,
    pub height: u32,
    pub encoder: String,
    pub size_bytes: u64,
    pub elapsed_secs: f64,
}

/// `atempo` only accepts 0.5..=2 per instance on older builds; chain them.
fn atempo_chain(speed: f64) -> String {
    let mut s = speed;
    let mut parts = Vec::new();
    while s > 2.0 {
        parts.push("atempo=2.0".to_string());
        s /= 2.0;
    }
    while s < 0.5 {
        parts.push("atempo=0.5".to_string());
        s /= 0.5;
    }
    if (s - 1.0).abs() > 1e-6 {
        parts.push(format!("atempo={s:.6}"));
    }
    parts.join(",")
}

/// Build the audio filter graph that applies the timeline (trim, cuts, speed)
/// and per-track volume to each input, then mixes them into `[aout]`.
/// `inputs` are (ffmpeg input index, volume).
pub fn audio_filter_graph(timeline: &Timeline, inputs: &[(usize, f64)]) -> Option<String> {
    if inputs.is_empty() || timeline.clips.is_empty() {
        return None;
    }
    let n = timeline.clips.len();
    let mut g = Vec::new();
    for &(idx, vol) in inputs {
        let splits: String = (0..n).map(|j| format!("[a{idx}s{j}]")).collect();
        g.push(format!("[{idx}:a]aresample=48000,aformat=sample_fmts=fltp:channel_layouts=stereo,asplit={n}{splits}"));
        for (j, c) in timeline.clips.iter().enumerate() {
            let tempo = atempo_chain(c.speed);
            let tempo = if tempo.is_empty() { String::new() } else { format!(",{tempo}") };
            g.push(format!(
                "[a{idx}s{j}]atrim=start={:.6}:end={:.6},asetpts=PTS-STARTPTS{tempo}[a{idx}c{j}]",
                c.start, c.end
            ));
        }
        let parts: String = (0..n).map(|j| format!("[a{idx}c{j}]")).collect();
        g.push(format!("{parts}concat=n={n}:v=0:a=1,volume={vol:.4}[a{idx}]"));
    }
    if inputs.len() == 1 {
        g.push(format!("[a{}]apad[aout]", inputs[0].0));
    } else {
        let ins: String = inputs.iter().map(|(i, _)| format!("[a{i}]")).collect();
        g.push(format!("{ins}amix=inputs={}:duration=longest:normalize=0,apad[aout]", inputs.len()));
    }
    Some(g.join(";"))
}

fn usable_audio(dir: &Path, rel: &Option<String>, muted: bool, volume: f64) -> Result<Option<PathBuf>> {
    let Some(rel) = rel else { return Ok(None) };
    if muted || volume <= 0.0 {
        return Ok(None);
    }
    let p = resolve_media(dir, rel)?;
    // A WAV header alone (44 bytes) means nothing was captured.
    match std::fs::metadata(&p) {
        Ok(m) if m.len() > 1024 => Ok(Some(p)),
        _ => Ok(None),
    }
}

pub struct ExportRequest<'a> {
    pub project_dir: &'a Path,
    pub output: &'a Path,
    /// Overrides the project's stored export settings when given.
    pub settings: Option<ExportSettings>,
}

pub fn export(
    ff: &FfmpegPaths,
    encoders: &[EncoderInfo],
    req: ExportRequest,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(&ExportProgress),
) -> Result<ExportResult> {
    let started = Instant::now();
    let dir = req.project_dir;
    let project = Project::load(dir)?;
    let mut settings = req.settings.unwrap_or_else(|| project.export.clone());
    settings.sanitize();
    let track = project.load_cursor(dir)?.unwrap_or_else(CursorTrack::default);

    let video_path = resolve_media(dir, &project.media.video)?;
    let info = probe_media(&ff.ffprobe, &video_path)?;
    let (src_w, src_h) = (info.width, info.height);
    let src_fps = if info.fps > 0.0 { info.fps } else { project.source.fps as f64 };

    let layout = project.layout();
    let (out_w, out_h) = layout.output_size(settings.height);
    let duration = project.recording.duration.max(info.duration.min(project.recording.duration + 1.0));
    let camera = CameraPath::compute(
        &project.edit.zoom.segments,
        &track,
        &layout,
        &project.edit.zoom.settings,
        duration,
        120.0,
    );
    let cursor_path = CursorPath::compute(&track, &project.edit.cursor, duration, 120.0);
    let compositor = Compositor::new(
        layout,
        &project.edit.style,
        &project.edit.cursor,
        &track.shapes,
        Some(dir),
        out_w,
        out_h,
    );

    let timeline = &project.edit.timeline;
    let out_duration = timeline.output_duration();
    if out_duration <= 0.0 {
        return Err(Error::Invalid("nothing to export: the timeline is empty".into()));
    }
    let fps = settings.fps;
    let total_frames = (out_duration * fps as f64).round().max(1.0) as u64;

    // Audio inputs.
    let a = &project.edit.audio;
    let mut audio_files = Vec::new();
    if let Some(p) = usable_audio(dir, &project.media.mic, a.mic_muted, a.mic_volume)? {
        audio_files.push((p, a.mic_volume));
    }
    if let Some(p) = usable_audio(dir, &project.media.system_audio, a.system_muted, a.system_volume)? {
        audio_files.push((p, a.system_volume));
    }

    let encoder = choose_encoder(encoders, &settings.encoder)?;
    let spec = VideoEncodeSpec {
        width: out_w,
        height: out_h,
        fps,
        input_pix_fmt: "rgba",
        encoder: encoder.clone(),
        quality: settings.quality,
        realtime: false,
    };
    let partial = req.output.with_extension("partial.mp4");
    let mut args = raw_input_args(&spec);
    for (p, _) in &audio_files {
        args.push("-i".into());
        args.push(p.to_string_lossy().into_owned());
    }
    let inputs: Vec<(usize, f64)> = audio_files.iter().enumerate().map(|(i, (_, v))| (i + 1, *v)).collect();
    args.extend(["-map".into(), "0:v".into()]);
    if let Some(graph) = audio_filter_graph(timeline, &inputs) {
        args.extend(["-filter_complex".into(), graph, "-map".into(), "[aout]".into()]);
        args.extend(["-c:a".into(), "aac".into(), "-b:a".into(), "192k".into()]);
    }
    args.extend(video_codec_args(&spec));
    args.extend(["-t".into(), format!("{:.6}", total_frames as f64 / fps as f64)]);
    args.extend(["-movflags".into(), "+faststart".into(), "-f".into(), "mp4".into()]);
    args.push(partial.to_string_lossy().into_owned());

    let mut writer = FfmpegWriter::spawn(&ff.ffmpeg, &args)?;
    let mut reader = FfmpegReader::spawn(&ff.ffmpeg, &video_path, src_w, src_h)?;
    let mut src = vec![0u8; reader.frame_len()];
    let mut have_frame = false;
    let mut src_index: i64 = -1;
    let mut out = vec![0u8; (out_w * out_h * 4) as usize];
    let mut clicks_buf: Vec<(Vec2, f64)> = Vec::new();
    let mut last_report = Instant::now();

    let result = (|| -> Result<()> {
        for k in 0..total_frames {
            if cancel.load(Ordering::Relaxed) {
                return Err(Error::Cancelled);
            }
            let t_out = k as f64 / fps as f64;
            let t_src = timeline.output_to_source(t_out);
            let want = (t_src * src_fps + 1e-4).floor() as i64;
            while src_index < want {
                if reader.read_frame(&mut src)? {
                    src_index += 1;
                    have_frame = true;
                } else {
                    break; // End of stream: keep showing the last frame.
                }
            }
            if !have_frame {
                return Err(Error::InvalidMedia("the recording contains no video frames".into()));
            }
            clicks_buf.clear();
            clicks_buf.extend(
                active_clicks(&track.clicks, t_src, project.edit.cursor.click_duration)
                    .map(|(c, p)| (Vec2::new(c.x as f64, c.y as f64), p)),
            );
            compositor.render(
                &FrameInput {
                    src: &src,
                    src_w,
                    src_h,
                    camera: camera.sample(t_src),
                    cursor: cursor_path.sample(t_src),
                    clicks: &clicks_buf,
                },
                &mut out,
            );
            writer.write_frame(&out)?;
            if last_report.elapsed().as_millis() >= 200 || k + 1 == total_frames {
                last_report = Instant::now();
                let el = started.elapsed().as_secs_f64().max(1e-3);
                let rate = (k + 1) as f64 / el;
                progress(&ExportProgress {
                    frame: k + 1,
                    total_frames,
                    fps: rate,
                    eta_secs: (total_frames - k - 1) as f64 / rate.max(1e-3),
                });
            }
        }
        Ok(())
    })();

    drop(reader);
    if let Err(e) = result {
        writer.kill();
        let _ = std::fs::remove_file(&partial);
        return Err(e);
    }
    if let Err(e) = writer.finish() {
        let _ = std::fs::remove_file(&partial);
        return Err(e);
    }

    // Validate before declaring success.
    let expected = total_frames as f64 / fps as f64;
    let check = probe_media(&ff.ffprobe, &partial).and_then(|m| {
        if (m.duration - expected).abs() > (2.0 / fps as f64).max(0.25) {
            return Err(Error::InvalidMedia(format!("duration {:.2}s, expected {:.2}s", m.duration, expected)));
        }
        if m.width != out_w || m.height != out_h {
            return Err(Error::InvalidMedia(format!("size {}x{}, expected {out_w}x{out_h}", m.width, m.height)));
        }
        if !audio_files.is_empty() && !m.has_audio {
            return Err(Error::InvalidMedia("audio track missing".into()));
        }
        Ok(m)
    });
    if let Err(e) = check {
        let _ = std::fs::remove_file(&partial);
        return Err(e);
    }
    if req.output.exists() {
        std::fs::remove_file(req.output)?;
    }
    std::fs::rename(&partial, req.output)?;
    let size_bytes = std::fs::metadata(req.output).map(|m| m.len()).unwrap_or(0);
    Ok(ExportResult {
        output: req.output.to_path_buf(),
        duration: expected,
        frames: total_frames,
        width: out_w,
        height: out_h,
        encoder,
        size_bytes,
        elapsed_secs: started.elapsed().as_secs_f64(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atempo_chains_cover_range() {
        assert_eq!(atempo_chain(1.0), "");
        assert_eq!(atempo_chain(1.5), "atempo=1.500000");
        assert_eq!(atempo_chain(8.0), "atempo=2.0,atempo=2.0,atempo=2.000000");
        assert_eq!(atempo_chain(0.25), "atempo=0.5,atempo=0.500000");
    }

    #[test]
    fn filter_graph_has_one_chain_per_clip_and_mix() {
        let mut t = Timeline::new(10.0);
        t.cut(2.0, 3.0);
        t.clips[1].speed = 2.0;
        let g = audio_filter_graph(&t, &[(1, 1.0), (2, 0.5)]).unwrap();
        assert!(g.contains("[1:a]"));
        assert!(g.contains("[2:a]"));
        assert!(g.contains("atrim=start=0.000000:end=2.000000"));
        assert!(g.contains("atrim=start=3.000000:end=10.000000,asetpts=PTS-STARTPTS,atempo=2.000000"));
        assert!(g.contains("amix=inputs=2"));
        assert!(g.ends_with("[aout]"));
        assert!(audio_filter_graph(&t, &[]).is_none());
    }
}
