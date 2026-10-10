//! Recording session: capture → CFR pacer → FFmpeg, plus input and audio,
//! driven by the recording state machine and finalized into a project folder.

use crate::audio::{AudioRecorder, AudioSource, AudioStats};
use crate::capture::{Capture, CaptureTarget, FrameSlot};
use crate::clock::SessionClock;
use crate::input::{CaptureArea, InputRecorder};
use crate::sources::{find_display, is_window_alive, window_rect};
use anyhow::{anyhow, Context, Result};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use studio_core::camera::ZoomSettings;
use studio_core::media::{
    choose_encoder, raw_input_args, remux_faststart, video_codec_args, write_thumbnail, EncoderInfo, FfmpegPaths,
    FfmpegWriter, VideoEncodeSpec,
};
use studio_core::project::{
    write_json_atomic, MediaRefs, Project, Quality, RecordingInfo, SourceInfo, SourceKind,
};
use studio_core::recording::{RecordingEvent, RecordingState};
use windows::Win32::Foundation::HWND;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum SourceSelection {
    Display { id: String },
    Window { id: u64 },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordOptions {
    pub source: SourceSelection,
    pub fps: u32,
    pub encoder: String,
    /// `None` = no microphone; `Some(None)` = default microphone.
    pub mic: Option<Option<String>>,
    pub system_audio: bool,
    pub output_root: PathBuf,
    pub name: Option<String>,
    #[serde(default)]
    pub zoom: ZoomSettings,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionStatus {
    pub state: RecordingState,
    pub elapsed: f64,
    pub frames_written: u64,
    pub frames_captured: u64,
    /// Seconds the encoder is behind real time (should stay near 0).
    pub lag: f64,
    pub bytes: u64,
    pub mic_level: Option<f32>,
    pub system_level: Option<f32>,
    pub warnings: Vec<String>,
    /// The source disappeared or the encoder failed; the UI should call stop.
    pub needs_stop: bool,
    pub dir: PathBuf,
}

struct PacerResult {
    writer: FfmpegWriter,
    frames: u64,
    error: Option<String>,
}

pub struct RecordingSession {
    state: Mutex<RecordingState>,
    clock: Arc<SessionClock>,
    slot: Arc<FrameSlot>,
    capture: Mutex<Option<Capture>>,
    pacer: Mutex<Option<JoinHandle<PacerResult>>>,
    input: Mutex<Option<InputRecorder>>,
    mic: Mutex<Option<AudioRecorder>>,
    system: Mutex<Option<AudioRecorder>>,
    stop_flag: Arc<AtomicBool>,
    needs_stop: Arc<AtomicBool>,
    frames_written: Arc<AtomicU64>,
    lag_ms: Arc<AtomicU64>,
    warnings: Mutex<Vec<String>>,
    pause_count: Mutex<u32>,
    pub dir: PathBuf,
    ffmpeg: FfmpegPaths,
    encoder: String,
    fps: u32,
    width: u32,
    height: u32,
    source: SourceInfo,
    name: String,
    created_at: i64,
    zoom: ZoomSettings,
}

const PARTIAL_VIDEO: &str = "screen.partial.mp4";

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

fn default_name() -> String {
    let t = unsafe { windows::Win32::System::SystemInformation::GetLocalTime() };
    format!(
        "Recording {:04}-{:02}-{:02} at {:02}.{:02}.{:02}",
        t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond
    )
}

/// Make a user-supplied name safe as a folder name.
pub fn sanitize_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| if c.is_control() || r#"<>:"/\|?*"#.contains(c) { ' ' } else { c })
        .collect();
    let cleaned = cleaned.trim().trim_end_matches('.').trim().to_string();
    let reserved = ["CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "LPT1", "LPT2", "LPT3"];
    if cleaned.is_empty() || reserved.contains(&cleaned.to_ascii_uppercase().as_str()) {
        "Recording".to_string()
    } else {
        cleaned.chars().take(120).collect()
    }
}

fn unique_dir(root: &Path, name: &str) -> PathBuf {
    let mut dir = root.join(name);
    let mut i = 2;
    while dir.exists() {
        dir = root.join(format!("{name} ({i})"));
        i += 1;
    }
    dir
}

impl RecordingSession {
    pub fn start(ff: &FfmpegPaths, encoders: &[EncoderInfo], opts: RecordOptions) -> Result<Arc<Self>> {
        let mut state = RecordingState::Idle.apply(RecordingEvent::Prepare)?;
        let fps = opts.fps.clamp(10, 120);

        // Resolve the source.
        let (target, area, label, kind, scale) = match &opts.source {
            SourceSelection::Display { id } => {
                let d = find_display(id).ok_or_else(|| anyhow!("display {id} is no longer connected"))?;
                (CaptureTarget::Display(d.hmonitor()), CaptureArea::Fixed { x: d.x, y: d.y }, d.name.clone(), SourceKind::Display, d.scale)
            }
            SourceSelection::Window { id } => {
                let hwnd = HWND(*id as usize as *mut _);
                if !is_window_alive(hwnd) || window_rect(hwnd).is_none() {
                    return Err(anyhow!("the selected window is no longer available"));
                }
                let title = crate::sources::list_windows()
                    .into_iter()
                    .find(|w| w.id == *id)
                    .map(|w| if w.app.is_empty() { w.title } else { format!("{} — {}", w.app, w.title) })
                    .unwrap_or_else(|| "Window".into());
                (CaptureTarget::Window(hwnd), CaptureArea::Window(*id as isize), title, SourceKind::Window, 1.0)
            }
        };

        let name = sanitize_name(&opts.name.clone().unwrap_or_else(default_name));
        std::fs::create_dir_all(&opts.output_root)
            .with_context(|| format!("cannot create {}", opts.output_root.display()))?;
        let dir = unique_dir(&opts.output_root, &name);
        std::fs::create_dir_all(&dir)?;

        let encoder = choose_encoder(encoders, &opts.encoder)?;
        let slot = Arc::new(FrameSlot::default());
        let capture = Capture::start(target, slot.clone()).inspect_err(|_| {
            let _ = std::fs::remove_dir_all(&dir);
        })?;
        let (width, height) = (capture.width, capture.height);
        if !slot.wait_for_frame(Duration::from_secs(3)) {
            capture.stop();
            let _ = std::fs::remove_dir_all(&dir);
            return Err(anyhow!("no frames received from the capture source"));
        }

        let spec = VideoEncodeSpec {
            width,
            height,
            fps,
            input_pix_fmt: "bgra",
            encoder: encoder.clone(),
            quality: Quality::High,
            realtime: true,
        };
        let mut args = raw_input_args(&spec);
        args.extend(video_codec_args(&spec));
        // Fragmented MP4: playable even if the app crashes mid-recording.
        args.extend(["-movflags", "+frag_keyframe+empty_moov+default_base_moof", "-f", "mp4"].map(String::from));
        args.push(dir.join(PARTIAL_VIDEO).to_string_lossy().into_owned());
        let writer = match FfmpegWriter::spawn(&ff.ffmpeg, &args) {
            Ok(w) => w,
            Err(e) => {
                capture.stop();
                let _ = std::fs::remove_dir_all(&dir);
                return Err(e.into());
            }
        };

        let clock = Arc::new(SessionClock::new());
        let mut warnings = Vec::new();

        let session = Arc::new(RecordingSession {
            state: Mutex::new(state.clone()),
            clock: clock.clone(),
            slot: slot.clone(),
            capture: Mutex::new(Some(capture)),
            pacer: Mutex::new(None),
            input: Mutex::new(None),
            mic: Mutex::new(None),
            system: Mutex::new(None),
            stop_flag: Arc::new(AtomicBool::new(false)),
            needs_stop: Arc::new(AtomicBool::new(false)),
            frames_written: Arc::new(AtomicU64::new(0)),
            lag_ms: Arc::new(AtomicU64::new(0)),
            warnings: Mutex::new(Vec::new()),
            pause_count: Mutex::new(0),
            dir: dir.clone(),
            ffmpeg: ff.clone(),
            encoder,
            fps,
            width,
            height,
            source: SourceInfo { kind, label, width, height, fps, scale_factor: scale },
            name,
            created_at: now_ms(),
            zoom: opts.zoom.clone(),
        });

        // Everything is ready: start the clock and all recorders together.
        clock.start();
        match InputRecorder::start(clock.clone(), area, width, height, dir.clone()) {
            Ok(i) => *session.input.lock() = Some(i),
            Err(e) => warnings.push(format!("Cursor tracking unavailable: {e}")),
        }
        if let Some(mic) = &opts.mic {
            match AudioRecorder::start(AudioSource::Microphone(mic.clone()), clock.clone(), &dir.join("mic.wav")) {
                Ok(a) => *session.mic.lock() = Some(a),
                Err(e) => warnings.push(format!("Microphone unavailable: {e}")),
            }
        }
        if opts.system_audio {
            match AudioRecorder::start(AudioSource::SystemLoopback, clock.clone(), &dir.join("system.wav")) {
                Ok(a) => *session.system.lock() = Some(a),
                Err(e) => warnings.push(format!("System audio unavailable: {e}")),
            }
        }
        for w in &warnings {
            log::warn!("{w}");
        }
        session.warnings.lock().extend(warnings);

        let pacer = {
            let s = session.clone();
            std::thread::Builder::new()
                .name("studio-video-pacer".into())
                .spawn(move || pacer_loop(&s, writer))?
        };
        *session.pacer.lock() = Some(pacer);
        state = state.apply(RecordingEvent::Started)?;
        *session.state.lock() = state;
        log::info!("recording started: {}x{} @ {} fps, encoder {}", width, height, fps, session.encoder);
        Ok(session)
    }

    fn transition(&self, ev: RecordingEvent) -> Result<()> {
        let mut st = self.state.lock();
        *st = st.apply(ev)?;
        Ok(())
    }

    pub fn pause(&self) -> Result<()> {
        self.transition(RecordingEvent::Pause)?;
        self.clock.pause();
        *self.pause_count.lock() += 1;
        Ok(())
    }

    pub fn resume(&self) -> Result<()> {
        self.transition(RecordingEvent::Resume)?;
        self.clock.resume();
        Ok(())
    }

    pub fn state(&self) -> RecordingState {
        self.state.lock().clone()
    }

    pub fn status(&self) -> SessionStatus {
        SessionStatus {
            state: self.state(),
            elapsed: self.clock.now(),
            frames_written: self.frames_written.load(Ordering::Relaxed),
            frames_captured: self.slot.frames_arrived.load(Ordering::Relaxed),
            lag: self.lag_ms.load(Ordering::Relaxed) as f64 / 1000.0,
            bytes: dir_size(&self.dir),
            mic_level: self.mic.lock().as_ref().map(|a| a.level()),
            system_level: self.system.lock().as_ref().map(|a| a.level()),
            warnings: self.warnings.lock().clone(),
            needs_stop: self.needs_stop.load(Ordering::Relaxed),
            dir: self.dir.clone(),
        }
    }

    /// Stop recording and finalize the project. Returns the project folder.
    pub fn stop(&self) -> Result<PathBuf> {
        self.transition(RecordingEvent::Stop)?;
        match self.finalize() {
            Ok(dir) => {
                self.transition(RecordingEvent::Finalized)?;
                Ok(dir)
            }
            Err(e) => {
                let _ = self.transition(RecordingEvent::Fail(e.to_string()));
                Err(e)
            }
        }
    }

    /// Stop and delete everything recorded.
    pub fn discard(&self) {
        self.stop_flag.store(true, Ordering::SeqCst);
        self.clock.resume();
        if let Some(c) = self.capture.lock().take() {
            c.stop();
        }
        if let Some(p) = self.pacer.lock().take() {
            if let Ok(r) = p.join() {
                r.writer.kill();
            }
        }
        drop(self.input.lock().take());
        if let Some(a) = self.mic.lock().take() {
            let _ = a.finish(0.0);
        }
        if let Some(a) = self.system.lock().take() {
            let _ = a.finish(0.0);
        }
        let _ = std::fs::remove_dir_all(&self.dir);
        let mut st = self.state.lock();
        *st = RecordingState::Idle;
    }

    fn finalize(&self) -> Result<PathBuf> {
        // Unpause so the clock is consistent, then stop producing frames.
        self.clock.resume();
        self.stop_flag.store(true, Ordering::SeqCst);
        log::debug!("finalize: stopping capture");
        if let Some(c) = self.capture.lock().take() {
            c.stop();
        }
        log::debug!("finalize: joining pacer");
        let pacer = self.pacer.lock().take().ok_or_else(|| anyhow!("recording was not running"))?;
        let PacerResult { writer, frames, error } = pacer.join().map_err(|_| anyhow!("video thread panicked"))?;
        let duration = frames as f64 / self.fps as f64;
        log::debug!("finalize: stopping input");
        let track = self.input.lock().take().map(|i| i.finish());
        log::debug!("finalize: stopping audio");
        let mic = self.mic.lock().take().map(|a| a.finish(duration));
        let system = self.system.lock().take().map(|a| a.finish(duration));
        log::debug!("finalize: flushing encoder");
        if let Err(e) = writer.finish() {
            // The fragmented file may still be usable; try to keep it.
            log::error!("encoder exited with error: {e}");
            if error.is_none() {
                self.warnings.lock().push(format!("Encoder error: {e}"));
            }
        }
        if frames == 0 {
            return Err(anyhow!("no video frames were recorded"));
        }

        let partial = self.dir.join(PARTIAL_VIDEO);
        let video = self.dir.join("screen.mp4");
        remux_faststart(&self.ffmpeg.ffmpeg, &partial, &video).context("could not finalize the video file")?;
        let _ = std::fs::remove_file(&partial);

        let audio_rel = |r: Option<Result<AudioStats>>, name: &str| -> Option<String> {
            match r {
                Some(Ok(s)) => {
                    log::info!(
                        "{name}: {} frames @ {} Hz, {} silence inserted, {} dropped",
                        s.frames, s.sample_rate, s.silence_inserted_frames, s.dropped_frames
                    );
                    Some(s.path.file_name()?.to_string_lossy().into_owned())
                }
                Some(Err(e)) => {
                    log::error!("{name} failed: {e}");
                    None
                }
                None => None,
            }
        };
        let mic_rel = audio_rel(mic, "microphone");
        let system_rel = audio_rel(system, "system audio");

        let cursor_rel = match &track {
            Some(t) => {
                write_json_atomic(&self.dir.join("cursor.json"), t)?;
                Some("cursor.json".to_string())
            }
            None => None,
        };
        let thumb = self.dir.join("thumbnail.jpg");
        let thumb_rel = write_thumbnail(&self.ffmpeg.ffmpeg, &video, &thumb, (duration / 2.0).min(1.0), 480)
            .map_err(|e| log::warn!("thumbnail: {e}"))
            .ok()
            .map(|_| "thumbnail.jpg".to_string());

        let project = Project::new_for_recording(
            self.name.clone(),
            self.created_at,
            self.source.clone(),
            MediaRefs {
                video: "screen.mp4".into(),
                mic: mic_rel,
                system_audio: system_rel,
                cursor: cursor_rel,
                thumbnail: thumb_rel,
            },
            RecordingInfo {
                duration,
                frames_written: frames,
                frames_dropped: 0,
                encoder: self.encoder.clone(),
                pause_count: *self.pause_count.lock(),
            },
            track.as_ref(),
            self.zoom.clone(),
        );
        project.save(&self.dir)?;
        log::info!("recording finalized: {} ({duration:.2}s, {frames} frames)", self.dir.display());
        Ok(self.dir.clone())
    }
}

/// Total size of files in `dir`. Sizes are read through a file handle: NTFS
/// directory entries are not updated while FFmpeg still has the file open.
fn dir_size(dir: &Path) -> u64 {
    std::fs::read_dir(dir)
        .map(|it| {
            it.flatten()
                .filter_map(|e| std::fs::File::open(e.path()).and_then(|f| f.metadata()).ok())
                .filter(|m| m.is_file())
                .map(|m| m.len())
                .sum()
        })
        .unwrap_or(0)
}

/// Emit exactly one frame per tick of the recording clock (CFR), using the
/// newest captured frame. Pauses freeze the clock, so no frames are written
/// while paused and timestamps stay continuous.
fn pacer_loop(s: &RecordingSession, mut writer: FfmpegWriter) -> PacerResult {
    let frame_dur = 1.0 / s.fps as f64;
    let mut frames: u64 = 0;
    let mut current: Vec<u8> = s.slot.take_latest().map(|f| f.data).unwrap_or_else(|| vec![0; (s.width * s.height * 4) as usize]);
    let mut error = None;
    loop {
        if s.stop_flag.load(Ordering::SeqCst) {
            break;
        }
        if s.clock.is_paused() {
            std::thread::sleep(Duration::from_millis(5));
            continue;
        }
        let (closed, why) = s.slot.closed();
        if closed {
            let msg = why.unwrap_or_else(|| "capture source closed".into());
            s.warnings.lock().push(msg);
            s.needs_stop.store(true, Ordering::SeqCst);
            break;
        }
        let t = s.clock.now();
        let due = frames as f64 * frame_dur;
        if t < due {
            let wait = (due - t).min(0.005);
            std::thread::sleep(Duration::from_secs_f64(wait));
            continue;
        }
        if let Some(f) = s.slot.take_latest() {
            s.slot.recycle(std::mem::replace(&mut current, f.data));
        }
        let started = Instant::now();
        if let Err(e) = writer.write_frame(&current) {
            log::error!("video encoder failed: {e}");
            s.warnings.lock().push(format!("Video encoder failed: {e}"));
            s.needs_stop.store(true, Ordering::SeqCst);
            error = Some(e.to_string());
            break;
        }
        frames += 1;
        s.frames_written.store(frames, Ordering::Relaxed);
        let lag = (s.clock.now() - frames as f64 * frame_dur).max(0.0);
        s.lag_ms.store((lag * 1000.0) as u64, Ordering::Relaxed);
        if started.elapsed() > Duration::from_millis(500) {
            log::warn!("encoder write stalled for {:?}", started.elapsed());
        }
    }
    PacerResult { writer, frames, error }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_sanitized() {
        assert_eq!(sanitize_name("My: demo/clip?"), "My  demo clip");
        assert_eq!(sanitize_name("  CON "), "Recording");
        assert_eq!(sanitize_name("..."), "Recording");
        assert_eq!(sanitize_name("ok."), "ok");
    }
}
