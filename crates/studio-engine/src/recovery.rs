//! Recovery of recordings interrupted by a crash, power loss or kill.
//!
//! While recording, a folder contains `recording.json` (this manifest),
//! `screen.partial.mp4` (fragmented MP4, readable up to the last fragment),
//! WAV files whose headers were never finalized, and `cursor.log`. Recovery
//! turns that into a normal project.

use crate::input::CURSOR_LOG;
use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use studio_core::camera::ZoomSettings;
use studio_core::cursor::CursorTrack;
use studio_core::media::{command, probe_media, write_thumbnail, FfmpegPaths};
use studio_core::project::{write_json_atomic, MediaRefs, Project, RecordingInfo, SourceInfo, PROJECT_FILE};

pub const MANIFEST: &str = "recording.json";
const PARTIAL_VIDEO: &str = "screen.partial.mp4";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordingManifest {
    pub name: String,
    pub created_at: i64,
    pub source: SourceInfo,
    pub zoom: ZoomSettings,
    pub encoder: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Unfinished {
    pub dir: PathBuf,
    pub name: String,
    pub created_at: i64,
    pub bytes: u64,
}

/// Folders under `root` that hold an interrupted recording. `exclude` is the
/// folder of a recording that is still in progress.
pub fn find_unfinished(root: &Path, exclude: Option<&Path>) -> Vec<Unfinished> {
    let Ok(entries) = std::fs::read_dir(root) else { return Vec::new() };
    let mut out: Vec<Unfinished> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|d| d.join(MANIFEST).is_file() && !d.join(PROJECT_FILE).is_file())
        .filter(|d| exclude.is_none_or(|x| x != d.as_path()))
        .filter_map(|dir| {
            let m: RecordingManifest = serde_json::from_slice(&std::fs::read(dir.join(MANIFEST)).ok()?).ok()?;
            let bytes = std::fs::read_dir(&dir)
                .map(|it| it.flatten().filter_map(|e| e.metadata().ok()).map(|m| m.len()).sum())
                .unwrap_or(0);
            Some(Unfinished { name: m.name, created_at: m.created_at, bytes, dir })
        })
        .collect();
    out.sort_by_key(|u| std::cmp::Reverse(u.created_at));
    out
}

/// Fix RIFF/data chunk sizes of a WAV whose writer never finalized it.
/// Returns the number of complete sample frames now described by the header.
pub fn repair_wav(path: &Path) -> Result<u64> {
    let mut f = std::fs::OpenOptions::new().read(true).write(true).open(path)?;
    let len = f.metadata()?.len();
    let mut head = [0u8; 12];
    f.read_exact(&mut head).context("WAV too short")?;
    if &head[0..4] != b"RIFF" || &head[8..12] != b"WAVE" {
        return Err(anyhow!("not a WAV file"));
    }
    let mut pos = 12u64;
    let mut block_align = 0u64;
    loop {
        if pos + 8 > len {
            return Err(anyhow!("no data chunk"));
        }
        f.seek(SeekFrom::Start(pos))?;
        let mut ch = [0u8; 8];
        f.read_exact(&mut ch)?;
        let size = u32::from_le_bytes([ch[4], ch[5], ch[6], ch[7]]) as u64;
        if &ch[0..4] == b"fmt " {
            let mut fmt = [0u8; 16];
            f.read_exact(&mut fmt)?;
            block_align = u16::from_le_bytes([fmt[12], fmt[13]]) as u64;
        }
        if &ch[0..4] == b"data" {
            let data_start = pos + 8;
            let align = block_align.max(1);
            let data_len = ((len - data_start) / align) * align;
            f.seek(SeekFrom::Start(pos + 4))?;
            f.write_all(&(data_len.min(u32::MAX as u64) as u32).to_le_bytes())?;
            f.seek(SeekFrom::Start(4))?;
            f.write_all(&((data_start + data_len - 8).min(u32::MAX as u64) as u32).to_le_bytes())?;
            f.set_len(data_start + data_len)?;
            return Ok(data_len / align);
        }
        pos += 8 + size + (size & 1);
    }
}

/// Rebuild a project from an interrupted recording folder.
pub fn recover(ff: &FfmpegPaths, dir: &Path) -> Result<PathBuf> {
    let manifest: RecordingManifest = serde_json::from_slice(&std::fs::read(dir.join(MANIFEST))?)
        .context("the recovery manifest is unreadable")?;
    let partial = dir.join(PARTIAL_VIDEO);
    let video = dir.join("screen.mp4");
    if partial.is_file() {
        // A truncated fragmented MP4 makes FFmpeg report an error at the end even
        // though everything up to the last complete fragment is copied; judge the
        // result by probing the output instead of the exit status.
        let out = command(&ff.ffmpeg)
            .args(["-hide_banner", "-loglevel", "error", "-y", "-i"])
            .arg(&partial)
            .args(["-map", "0", "-c", "copy", "-movflags", "+faststart"])
            .arg(&video)
            .stdin(std::process::Stdio::null())
            .output()?;
        if !out.status.success() {
            log::warn!("remux of interrupted video reported: {}", String::from_utf8_lossy(&out.stderr).trim());
        }
    }
    let info = probe_media(&ff.ffprobe, &video).context("no usable video was recorded before the interruption")?;
    if info.duration <= 0.1 {
        return Err(anyhow!("the interrupted recording is too short to recover"));
    }
    let duration = info.duration;

    let audio = |name: &str| -> Option<String> {
        let p = dir.join(name);
        if !p.is_file() {
            return None;
        }
        match repair_wav(&p) {
            Ok(frames) if frames > 0 => Some(name.to_string()),
            other => {
                if let Err(e) = other {
                    log::info!("{name} has no recoverable audio: {e}");
                }
                let _ = std::fs::remove_file(&p);
                None
            }
        }
    };
    let mic = audio("mic.wav");
    let system = audio("system.wav");

    let track = if dir.join("cursor.json").is_file() {
        serde_json::from_slice::<CursorTrack>(&std::fs::read(dir.join("cursor.json"))?).ok()
    } else if dir.join(CURSOR_LOG).is_file() {
        let text = String::from_utf8_lossy(&std::fs::read(dir.join(CURSOR_LOG))?).into_owned();
        let t = CursorTrack::from_log(&text, manifest.source.width, manifest.source.height);
        write_json_atomic(&dir.join("cursor.json"), &t)?;
        Some(t)
    } else {
        None
    };

    let thumb = write_thumbnail(&ff.ffmpeg, &video, &dir.join("thumbnail.jpg"), (duration / 2.0).min(1.0), 480)
        .ok()
        .map(|_| "thumbnail.jpg".to_string());
    let project = Project::new_for_recording(
        manifest.name.clone(),
        manifest.created_at,
        manifest.source.clone(),
        MediaRefs {
            video: "screen.mp4".into(),
            mic,
            system_audio: system,
            cursor: track.as_ref().map(|_| "cursor.json".to_string()),
            thumbnail: thumb,
        },
        RecordingInfo {
            duration,
            frames_written: (duration * manifest.source.fps as f64).round() as u64,
            frames_dropped: 0,
            encoder: manifest.encoder.clone(),
            pause_count: 0,
        },
        track.as_ref(),
        manifest.zoom.clone(),
    );
    project.save(dir)?;
    for f in [MANIFEST, CURSOR_LOG, PARTIAL_VIDEO] {
        let _ = std::fs::remove_file(dir.join(f));
    }
    log::info!("recovered {} ({duration:.2}s)", dir.display());
    Ok(dir.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repairs_unfinalized_wav_header() {
        let dir = std::env::temp_dir().join(format!("glide-wav-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("a.wav");
        // Write a valid WAV, then corrupt the sizes the way an unfinalized writer leaves them.
        let spec = hound::WavSpec { channels: 2, sample_rate: 48000, bits_per_sample: 16, sample_format: hound::SampleFormat::Int };
        let mut w = hound::WavWriter::create(&path, spec).unwrap();
        for i in 0..4800 * 2 {
            w.write_sample((i % 100) as i16).unwrap();
        }
        w.finalize().unwrap();
        let mut bytes = std::fs::read(&path).unwrap();
        bytes[4..8].copy_from_slice(&0u32.to_le_bytes());
        let n = bytes.len();
        bytes[n - 8..n].fill(0); // trailing data stays; zero the last bytes
        let data_at = bytes.windows(4).position(|w| w == b"data").unwrap();
        bytes[data_at + 4..data_at + 8].copy_from_slice(&0u32.to_le_bytes());
        bytes.push(7); // a partial (half) sample frame at the end
        std::fs::write(&path, &bytes).unwrap();

        assert_eq!(repair_wav(&path).unwrap(), 4800);
        let r = hound::WavReader::open(&path).unwrap();
        assert_eq!(r.duration(), 4800);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
