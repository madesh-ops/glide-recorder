//! FFmpeg integration via controlled subprocesses.
//!
//! FFmpeg is always spawned with an explicit argument vector (never through a
//! shell). Encoders are *probed by running them*: a listed encoder is never
//! assumed to work.

use crate::project::Quality;
use crate::{Error, Result};
use tail::TailBuffer;
use serde::{Deserialize, Serialize};
use std::io::{BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FfmpegPaths {
    pub ffmpeg: PathBuf,
    pub ffprobe: PathBuf,
}

fn exe_name(base: &str) -> String {
    if cfg!(windows) {
        format!("{base}.exe")
    } else {
        base.to_string()
    }
}

/// Locate ffmpeg/ffprobe: `STUDIO_FFMPEG_DIR`, next to the executable (also
/// `resources/` and `ffmpeg/` sub-folders used by the installer), then `PATH`.
pub fn locate_ffmpeg() -> Result<FfmpegPaths> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    if let Some(d) = std::env::var_os("STUDIO_FFMPEG_DIR") {
        dirs.push(PathBuf::from(d));
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(d) = exe.parent() {
            dirs.push(d.to_path_buf());
            dirs.push(d.join("resources"));
            dirs.push(d.join("ffmpeg"));
        }
    }
    if let Some(path) = std::env::var_os("PATH") {
        dirs.extend(std::env::split_paths(&path));
    }
    for d in dirs {
        let ffmpeg = d.join(exe_name("ffmpeg"));
        let ffprobe = d.join(exe_name("ffprobe"));
        if ffmpeg.is_file() && ffprobe.is_file() {
            return Ok(FfmpegPaths { ffmpeg, ffprobe });
        }
    }
    Err(Error::FfmpegMissing)
}

/// A `Command` that never flashes a console window on Windows.
pub fn command(program: &Path) -> Command {
    let mut c = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        c.creation_flags(CREATE_NO_WINDOW);
    }
    c
}

/// Run a command with a timeout, returning (success, stderr).
fn run_with_timeout(mut cmd: Command, timeout: Duration) -> std::io::Result<(bool, String)> {
    cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::piped());
    let mut child = cmd.spawn()?;
    let mut stderr = child.stderr.take().unwrap();
    let reader = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = stderr.read_to_string(&mut s);
        s
    });
    let start = Instant::now();
    let status = loop {
        if let Some(st) = child.try_wait()? {
            break Some(st);
        }
        if start.elapsed() > timeout {
            let _ = child.kill();
            let _ = child.wait();
            break None;
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let err = reader.join().unwrap_or_default();
    Ok((status.is_some_and(|s| s.success()), err))
}

// ---------------------------------------------------------------------------
// Encoders
// ---------------------------------------------------------------------------

pub const CANDIDATE_ENCODERS: &[&str] = &["h264_nvenc", "h264_amf", "h264_qsv", "h264_mf", "libx264"];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EncoderInfo {
    pub name: String,
    pub label: String,
    pub hardware: bool,
}

pub fn encoder_label(name: &str) -> &'static str {
    match name {
        "h264_nvenc" => "NVIDIA NVENC (hardware)",
        "h264_amf" => "AMD AMF (hardware)",
        "h264_qsv" => "Intel Quick Sync (hardware)",
        "h264_mf" => "Windows Media Foundation",
        "libx264" => "x264 (software)",
        _ => "Unknown",
    }
}

/// Probe which H.264 encoders actually work on this machine, in preference order.
pub fn probe_encoders(ff: &FfmpegPaths) -> Vec<EncoderInfo> {
    let handles: Vec<_> = CANDIDATE_ENCODERS
        .iter()
        .map(|&name| {
            let ffmpeg = ff.ffmpeg.clone();
            std::thread::spawn(move || {
                let mut cmd = command(&ffmpeg);
                cmd.args(["-hide_banner", "-loglevel", "error", "-f", "lavfi", "-i", "color=c=gray:s=320x240:r=30"]);
                cmd.args(["-frames:v", "5", "-pix_fmt", pix_fmt_for(name), "-c:v", name, "-f", "null", "-"]);
                let ok = match run_with_timeout(cmd, Duration::from_secs(15)) {
                    Ok((ok, err)) => {
                        if !ok {
                            log::info!("encoder {name} unavailable: {}", err.lines().last().unwrap_or(""));
                        }
                        ok
                    }
                    Err(e) => {
                        log::warn!("encoder probe {name} failed to run: {e}");
                        false
                    }
                };
                (name, ok)
            })
        })
        .collect();
    handles
        .into_iter()
        .filter_map(|h| h.join().ok())
        .filter(|(_, ok)| *ok)
        .map(|(name, _)| EncoderInfo {
            name: name.to_string(),
            label: encoder_label(name).to_string(),
            hardware: !matches!(name, "libx264" | "h264_mf"),
        })
        .collect()
}

/// Pick the encoder: the requested one if it works, else the first working one.
pub fn choose_encoder(available: &[EncoderInfo], requested: &str) -> Result<String> {
    if requested != "auto" {
        if let Some(e) = available.iter().find(|e| e.name == requested) {
            return Ok(e.name.clone());
        }
        log::warn!("requested encoder {requested} is unavailable; falling back");
    }
    available.first().map(|e| e.name.clone()).ok_or(Error::NoEncoder)
}

fn pix_fmt_for(encoder: &str) -> &'static str {
    match encoder {
        "libx264" => "yuv420p",
        _ => "nv12",
    }
}

#[derive(Debug, Clone)]
pub struct VideoEncodeSpec {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    /// Raw input pixel format on stdin: "bgra" (capture) or "rgba" (compositor).
    pub input_pix_fmt: &'static str,
    pub encoder: String,
    pub quality: Quality,
    /// Recording (favour speed / low latency) vs export (favour quality).
    pub realtime: bool,
}

/// Arguments for raw video on stdin. The caller appends extra inputs/outputs.
pub fn raw_input_args(spec: &VideoEncodeSpec) -> Vec<String> {
    vec![
        "-f".into(),
        "rawvideo".into(),
        "-pix_fmt".into(),
        spec.input_pix_fmt.into(),
        "-video_size".into(),
        format!("{}x{}", spec.width, spec.height),
        "-framerate".into(),
        spec.fps.to_string(),
        "-i".into(),
        "pipe:0".into(),
    ]
}

/// Video codec arguments (conversion to BT.709 limited-range 4:2:0 + encoder settings).
pub fn video_codec_args(spec: &VideoEncodeSpec) -> Vec<String> {
    let q_x264 = match spec.quality {
        Quality::Low => 28,
        Quality::High => 21,
        Quality::Best => 17,
    };
    let q_hw = match spec.quality {
        Quality::Low => 30,
        Quality::High => 22,
        Quality::Best => 18,
    };
    let mut a: Vec<String> = vec![
        "-vf".into(),
        format!("scale=out_color_matrix=bt709:out_range=tv,format={}", pix_fmt_for(&spec.encoder)),
        "-c:v".into(),
        spec.encoder.clone(),
    ];
    let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    match spec.encoder.as_str() {
        "libx264" => {
            a.extend(s(&["-preset", if spec.realtime { "superfast" } else { "medium" }]));
            a.extend(s(&["-crf", &q_x264.to_string()]));
            if spec.realtime {
                a.extend(s(&["-tune", "zerolatency"]));
            }
        }
        "h264_amf" => {
            a.extend(s(&["-usage", if spec.realtime { "lowlatency" } else { "transcoding" }]));
            a.extend(s(&["-quality", if spec.realtime { "speed" } else { "quality" }]));
            let q = q_hw.to_string();
            a.extend(s(&["-rc", "cqp", "-qp_i", &q, "-qp_p", &q, "-qp_b", &q]));
        }
        "h264_nvenc" => {
            a.extend(s(&["-preset", if spec.realtime { "p2" } else { "p5" }]));
            a.extend(s(&["-rc", "vbr", "-cq", &q_hw.to_string(), "-b:v", "0"]));
        }
        "h264_qsv" => {
            a.extend(s(&["-preset", if spec.realtime { "veryfast" } else { "medium" }]));
            a.extend(s(&["-global_quality", &q_hw.to_string()]));
        }
        _ => {
            // h264_mf and unknown encoders: bitrate-based.
            let bpp = match spec.quality {
                Quality::Low => 0.05,
                Quality::High => 0.1,
                Quality::Best => 0.16,
            };
            let bitrate = (spec.width as f64 * spec.height as f64 * spec.fps as f64 * bpp) as u64;
            a.extend(s(&["-b:v", &bitrate.max(1_000_000).to_string()]));
            if spec.encoder == "h264_mf" {
                a.extend(s(&["-rate_control", "u_vbr", "-scenario", "display_remoting"]));
            }
        }
    }
    a.extend(s(&["-g", &spec.fps.to_string(), "-bf", "0"]));
    a.extend(s(&[
        "-colorspace", "bt709", "-color_primaries", "bt709", "-color_trc", "bt709", "-color_range", "tv",
    ]));
    a
}

// ---------------------------------------------------------------------------
// Subprocess wrappers
// ---------------------------------------------------------------------------

mod tail {
    use std::collections::VecDeque;

    /// Keeps the last N lines of a child's stderr for error reports.
    #[derive(Default)]
    pub struct TailBuffer {
        lines: VecDeque<String>,
    }
    impl TailBuffer {
        pub fn push(&mut self, line: String) {
            if self.lines.len() >= 40 {
                self.lines.pop_front();
            }
            self.lines.push_back(line);
        }
        pub fn text(&self) -> String {
            self.lines.iter().cloned().collect::<Vec<_>>().join("\n")
        }
        pub fn last_meaningful(&self) -> String {
            self.lines
                .iter()
                .rev()
                .find(|l| !l.trim().is_empty())
                .cloned()
                .unwrap_or_default()
        }
    }
}

fn spawn_stderr_reader(child: &mut Child) -> (Arc<Mutex<TailBuffer>>, Option<JoinHandle<()>>) {
    let tail = Arc::new(Mutex::new(TailBuffer::default()));
    let handle = child.stderr.take().map(|stderr| {
        let tail = tail.clone();
        std::thread::spawn(move || {
            use std::io::BufRead;
            let reader = std::io::BufReader::new(stderr);
            for line in reader.split(b'\n').map_while(|l| l.ok()) {
                let line = String::from_utf8_lossy(&line).trim_end().to_string();
                log::debug!("ffmpeg: {line}");
                tail.lock().unwrap().push(line);
            }
        })
    });
    (tail, handle)
}

/// An FFmpeg process consuming raw frames on stdin.
pub struct FfmpegWriter {
    child: Child,
    stdin: Option<ChildStdin>,
    tail: Arc<Mutex<TailBuffer>>,
    stderr_thread: Option<JoinHandle<()>>,
}

impl FfmpegWriter {
    pub fn spawn(ffmpeg: &Path, args: &[String]) -> Result<Self> {
        log::info!("spawning ffmpeg {}", args.join(" "));
        let mut child = command(ffmpeg)
            .args(["-hide_banner", "-loglevel", "error", "-nostats", "-y"])
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()?;
        let stdin = child.stdin.take();
        let (tail, stderr_thread) = spawn_stderr_reader(&mut child);
        Ok(Self { child, stdin, tail, stderr_thread })
    }

    pub fn write_frame(&mut self, data: &[u8]) -> Result<()> {
        let stdin = self.stdin.as_mut().ok_or_else(|| Error::Ffmpeg("encoder input closed".into()))?;
        stdin.write_all(data).map_err(|e| {
            let detail = self.tail.lock().unwrap().last_meaningful();
            Error::Ffmpeg(if detail.is_empty() { e.to_string() } else { detail })
        })
    }

    /// Close stdin and wait for FFmpeg to finish writing the file.
    pub fn finish(mut self) -> Result<()> {
        drop(self.stdin.take());
        let status = self.child.wait()?;
        if let Some(h) = self.stderr_thread.take() {
            let _ = h.join();
        }
        if status.success() {
            Ok(())
        } else {
            Err(Error::Ffmpeg(format!("exit {status}: {}", self.tail.lock().unwrap().text())))
        }
    }

    pub fn kill(mut self) {
        drop(self.stdin.take());
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// An FFmpeg process decoding a video to raw RGBA frames on stdout.
pub struct FfmpegReader {
    child: Child,
    stdout: BufReader<ChildStdout>,
    tail: Arc<Mutex<TailBuffer>>,
    pub width: u32,
    pub height: u32,
}

impl FfmpegReader {
    pub fn spawn(ffmpeg: &Path, input: &Path, width: u32, height: u32) -> Result<Self> {
        let mut child = command(ffmpeg)
            .args(["-hide_banner", "-loglevel", "error", "-nostats", "-i"])
            .arg(input)
            .args(["-map", "0:v:0", "-f", "rawvideo", "-pix_fmt", "rgba", "-fps_mode", "passthrough"])
            .args(["-vf", &format!("scale={width}:{height}:in_color_matrix=bt709:in_range=tv")])
            .arg("pipe:1")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        let stdout = BufReader::with_capacity(1 << 20, child.stdout.take().unwrap());
        let (tail, _) = spawn_stderr_reader(&mut child);
        Ok(Self { child, stdout, tail, width, height })
    }

    pub fn frame_len(&self) -> usize {
        self.width as usize * self.height as usize * 4
    }

    /// Read the next frame into `buf`. Returns `Ok(false)` at end of stream.
    pub fn read_frame(&mut self, buf: &mut [u8]) -> Result<bool> {
        let mut read = 0;
        while read < buf.len() {
            match self.stdout.read(&mut buf[read..]) {
                Ok(0) => {
                    if read == 0 {
                        return Ok(false);
                    }
                    return Err(Error::Ffmpeg(format!(
                        "truncated frame from decoder: {}",
                        self.tail.lock().unwrap().last_meaningful()
                    )));
                }
                Ok(n) => read += n,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                Err(e) => return Err(e.into()),
            }
        }
        Ok(true)
    }
}

impl Drop for FfmpegReader {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

// ---------------------------------------------------------------------------
// Probing
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaInfo {
    pub duration: f64,
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub video_codec: Option<String>,
    pub has_audio: bool,
    pub audio_streams: u32,
}

fn parse_rate(s: &str) -> f64 {
    match s.split_once('/') {
        Some((a, b)) => {
            let (a, b) = (a.parse::<f64>().unwrap_or(0.0), b.parse::<f64>().unwrap_or(1.0));
            if b > 0.0 {
                a / b
            } else {
                0.0
            }
        }
        None => s.parse().unwrap_or(0.0),
    }
}

pub fn probe_media(ffprobe: &Path, path: &Path) -> Result<MediaInfo> {
    let out = command(ffprobe)
        .args(["-v", "error", "-print_format", "json", "-show_format", "-show_streams"])
        .arg(path)
        .stdin(Stdio::null())
        .output()?;
    if !out.status.success() {
        return Err(Error::InvalidMedia(format!(
            "{}: {}",
            path.display(),
            String::from_utf8_lossy(&out.stderr).trim()
        )));
    }
    let v: serde_json::Value = serde_json::from_slice(&out.stdout)
        .map_err(|e| Error::InvalidMedia(format!("ffprobe output: {e}")))?;
    let mut info = MediaInfo::default();
    if let Some(d) = v["format"]["duration"].as_str() {
        info.duration = d.parse().unwrap_or(0.0);
    }
    for s in v["streams"].as_array().into_iter().flatten() {
        match s["codec_type"].as_str() {
            Some("video") if info.video_codec.is_none() => {
                info.video_codec = s["codec_name"].as_str().map(str::to_string);
                info.width = s["width"].as_u64().unwrap_or(0) as u32;
                info.height = s["height"].as_u64().unwrap_or(0) as u32;
                info.fps = parse_rate(s["avg_frame_rate"].as_str().unwrap_or("0/1"));
                if info.fps <= 0.0 {
                    info.fps = parse_rate(s["r_frame_rate"].as_str().unwrap_or("0/1"));
                }
            }
            Some("audio") => {
                info.has_audio = true;
                info.audio_streams += 1;
            }
            _ => {}
        }
    }
    if info.video_codec.is_none() || info.width == 0 || info.height == 0 {
        return Err(Error::InvalidMedia(format!("{} has no video stream", path.display())));
    }
    Ok(info)
}

/// Remux (no re-encode) to a regular MP4 with the index at the front.
pub fn remux_faststart(ffmpeg: &Path, input: &Path, output: &Path) -> Result<()> {
    let out = command(ffmpeg)
        .args(["-hide_banner", "-loglevel", "error", "-y", "-i"])
        .arg(input)
        .args(["-map", "0", "-c", "copy", "-movflags", "+faststart"])
        .arg(output)
        .stdin(Stdio::null())
        .output()?;
    if out.status.success() {
        Ok(())
    } else {
        Err(Error::Ffmpeg(String::from_utf8_lossy(&out.stderr).trim().to_string()))
    }
}

/// Write a JPEG thumbnail of the frame at `at` seconds, scaled to `width`.
pub fn write_thumbnail(ffmpeg: &Path, input: &Path, output: &Path, at: f64, width: u32) -> Result<()> {
    let out = command(ffmpeg)
        .args(["-hide_banner", "-loglevel", "error", "-y", "-ss", &format!("{at:.3}"), "-i"])
        .arg(input)
        .args(["-frames:v", "1", "-vf", &format!("scale={width}:-2"), "-q:v", "4"])
        .arg(output)
        .stdin(Stdio::null())
        .output()?;
    if out.status.success() && output.is_file() {
        Ok(())
    } else {
        Err(Error::Ffmpeg(String::from_utf8_lossy(&out.stderr).trim().to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rate_parsing() {
        assert_eq!(parse_rate("60/1"), 60.0);
        assert!((parse_rate("30000/1001") - 29.97).abs() < 0.01);
        assert_eq!(parse_rate("0/0"), 0.0);
        assert_eq!(parse_rate("25"), 25.0);
    }

    #[test]
    fn choose_encoder_falls_back() {
        let avail = vec![
            EncoderInfo { name: "h264_amf".into(), label: "".into(), hardware: true },
            EncoderInfo { name: "libx264".into(), label: "".into(), hardware: false },
        ];
        assert_eq!(choose_encoder(&avail, "auto").unwrap(), "h264_amf");
        assert_eq!(choose_encoder(&avail, "libx264").unwrap(), "libx264");
        assert_eq!(choose_encoder(&avail, "h264_nvenc").unwrap(), "h264_amf");
        assert!(choose_encoder(&[], "auto").is_err());
    }

    #[test]
    fn codec_args_contain_color_tags_and_gop() {
        let spec = VideoEncodeSpec {
            width: 1920,
            height: 1080,
            fps: 60,
            input_pix_fmt: "bgra",
            encoder: "libx264".into(),
            quality: Quality::High,
            realtime: true,
        };
        let a = video_codec_args(&spec);
        let joined = a.join(" ");
        assert!(joined.contains("-c:v libx264"));
        assert!(joined.contains("format=yuv420p"));
        assert!(joined.contains("-g 60"));
        assert!(joined.contains("-colorspace bt709"));
    }
}
