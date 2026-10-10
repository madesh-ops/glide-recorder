//! Commands invoked by the UI. Inputs from the webview are validated here;
//! long-running work runs on background threads so the UI never blocks.

use crate::paths::{self, allow_asset_dir};
use crate::state::AppState;
use crate::windows as win;
use base64::Engine as _;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use studio_core::camera::ZoomSegment;
use studio_core::export::{export, ExportProgress, ExportRequest, ExportResult};
use studio_core::media::{
    probe_media, video_codec_args, write_thumbnail, EncoderInfo, VideoEncodeSpec,
};
use studio_core::preview::{build_preview, regenerate_auto_segments, PreviewModel};
use studio_core::project::{
    list_projects as scan_projects, EditState, ExportSettings, MediaRefs, Project, ProjectSummary, Quality,
    RecordingInfo, SourceInfo, SourceKind,
};
use studio_core::settings::AppSettings;
use studio_engine::audio::{list_input_devices, AudioDeviceInfo};
use studio_engine::capture::{thumbnail_png, CaptureTarget};
use studio_engine::session::{sanitize_name, RecordOptions, RecordingSession, SessionStatus, SourceSelection};
use studio_engine::sources::{find_display, list_displays, list_windows, DisplayInfo, WindowInfo};
use tauri::{AppHandle, Emitter, State};

type CmdResult<T> = Result<T, String>;

fn err<E: std::fmt::Display>(e: E) -> String {
    format!("{e:#}")
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> CmdResult<T> + Send + 'static) -> CmdResult<T> {
    tauri::async_runtime::spawn_blocking(f).await.map_err(err)?
}

// ---------------------------------------------------------------------------
// App / settings
// ---------------------------------------------------------------------------

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    version: String,
    ffmpeg: Option<String>,
    ffmpeg_error: Option<String>,
    capture_supported: bool,
    output_dir: PathBuf,
}

#[tauri::command]
pub fn app_info(state: State<AppState>) -> AppInfo {
    let ff = state.ffmpeg();
    AppInfo {
        version: env!("CARGO_PKG_VERSION").into(),
        ffmpeg: ff.as_ref().ok().map(|f| f.ffmpeg.to_string_lossy().into_owned()),
        ffmpeg_error: ff.err(),
        capture_supported: studio_engine::capture::is_supported(),
        output_dir: state.settings.lock().output_dir(),
    }
}

#[tauri::command]
pub async fn list_encoders(app: AppHandle) -> CmdResult<Vec<EncoderInfo>> {
    use tauri::Manager;
    blocking(move || Ok(app.state::<AppState>().encoders())).await
}

#[tauri::command]
pub fn get_settings(state: State<AppState>) -> AppSettings {
    state.settings.lock().clone()
}

#[tauri::command]
pub fn save_settings(app: AppHandle, state: State<AppState>, mut settings: AppSettings) -> CmdResult<AppSettings> {
    settings.sanitize();
    // Owned by the backend (set when a recording starts); the UI's copy may be stale.
    settings.last_recording = state.settings.lock().last_recording.clone();
    if let Some(dir) = &settings.output_dir {
        std::fs::create_dir_all(dir).map_err(|e| format!("cannot use {}: {e}", dir.display()))?;
    }
    settings.save_to(&state.settings_path).map_err(err)?;
    allow_asset_dir(&app, &settings.output_dir());
    *state.settings.lock() = settings.clone();
    Ok(settings)
}

// ---------------------------------------------------------------------------
// Sources
// ---------------------------------------------------------------------------

#[derive(Serialize)]
pub struct Sources {
    displays: Vec<DisplayInfo>,
    windows: Vec<WindowInfo>,
}

#[tauri::command]
pub async fn list_sources() -> CmdResult<Sources> {
    blocking(|| Ok(Sources { displays: list_displays(), windows: list_windows() })).await
}

/// PNG data URL preview of a display or window.
#[tauri::command]
pub async fn source_thumbnail(source: SourceSelection, max_width: Option<u32>) -> CmdResult<String> {
    blocking(move || {
        let target = match &source {
            SourceSelection::Display { id } | SourceSelection::Area { id, .. } => {
                CaptureTarget::Display(find_display(id).ok_or("display not found")?.hmonitor())
            }
            SourceSelection::Window { id } => {
                let w = list_windows().into_iter().find(|w| w.id == *id).ok_or("window not found")?;
                CaptureTarget::Window(w.hwnd())
            }
        };
        let png = thumbnail_png(target, max_width.unwrap_or(480).clamp(64, 960)).map_err(err)?;
        Ok(format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(png)))
    })
    .await
}

#[tauri::command]
pub async fn list_audio_devices() -> CmdResult<Vec<AudioDeviceInfo>> {
    blocking(|| Ok(list_input_devices())).await
}

// ---------------------------------------------------------------------------
// Recording
// ---------------------------------------------------------------------------

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordRequest {
    pub source: SourceSelection,
    /// "none", "default" or a device id.
    mic: String,
    system_audio: bool,
    fps: Option<u32>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingInfo {
    countdown: u32,
}

/// Async on purpose: creating a webview window from a synchronous command (which
/// runs on the main thread) can deadlock on Windows.
#[tauri::command]
pub async fn prepare_recording(app: AppHandle, request: RecordRequest) -> CmdResult<()> {
    use tauri::Manager;
    prepare(&app, &app.state::<AppState>(), request)
}

/// Queue a recording and show the countdown bar. Also remembers the request so
/// the global shortcut can repeat it.
pub fn prepare(app: &AppHandle, state: &AppState, request: RecordRequest) -> CmdResult<()> {
    if state.session.lock().is_some() || state.pending.lock().is_some() {
        return Err("a recording is already in progress".into());
    }
    state.ffmpeg()?;
    let settings = state.settings.lock().clone();
    let mic = match request.mic.as_str() {
        "none" => None,
        "default" => Some(None),
        id => Some(Some(id.to_string())),
    };
    {
        let mut s = state.settings.lock();
        s.last_recording = serde_json::to_value(&request).ok();
        if let Err(e) = s.save_to(&state.settings_path) {
            log::warn!("could not remember the recording source: {e}");
        }
    }
    let opts = RecordOptions {
        source: request.source,
        fps: request.fps.unwrap_or(settings.fps),
        encoder: settings.encoder.clone(),
        mic,
        system_audio: request.system_audio,
        output_root: settings.output_dir(),
        name: None,
        zoom: settings.default_zoom.clone(),
    };
    *state.pending.lock() = Some(opts);
    if settings.hide_window_while_recording {
        win::hide_main(app);
    }
    if let Err(e) = win::open_recorder_bar(app) {
        state.pending.lock().take();
        win::show_main(app);
        return Err(err(e));
    }
    Ok(())
}

/// The last recording request, if its source still exists.
pub fn last_request(state: &AppState) -> Option<RecordRequest> {
    let v = state.settings.lock().last_recording.clone()?;
    let req: RecordRequest = serde_json::from_value(v).ok()?;
    let ok = match &req.source {
        SourceSelection::Display { id } | SourceSelection::Area { id, .. } => find_display(id).is_some(),
        SourceSelection::Window { id } => list_windows().iter().any(|w| w.id == *id),
    };
    ok.then_some(req)
}

#[tauri::command]
pub fn pending_recording(state: State<AppState>) -> Option<PendingInfo> {
    state.pending.lock().as_ref().map(|_| PendingInfo { countdown: state.settings.lock().countdown_secs })
}

#[tauri::command]
pub fn cancel_pending(app: AppHandle, state: State<AppState>) {
    state.pending.lock().take();
    win::close_recorder_bar(&app);
    win::show_main(&app);
}

#[tauri::command]
pub async fn start_recording(app: AppHandle) -> CmdResult<()> {
    use tauri::Manager;
    blocking(move || {
        let state = app.state::<AppState>();
        let opts = state.pending.lock().take().ok_or("nothing to record")?;
        let ff = state.ffmpeg()?;
        let encoders = state.encoders();
        let session = RecordingSession::start(&ff, &encoders, opts).map_err(err)?;
        *state.session.lock() = Some(session);
        Ok(())
    })
    .await
}

fn current_session(state: &AppState) -> CmdResult<Arc<RecordingSession>> {
    state.session.lock().clone().ok_or_else(|| "no recording in progress".into())
}

#[tauri::command]
pub fn pause_recording(state: State<AppState>) -> CmdResult<()> {
    current_session(&state)?.pause().map_err(err)
}

#[tauri::command]
pub fn resume_recording(state: State<AppState>) -> CmdResult<()> {
    current_session(&state)?.resume().map_err(err)
}

#[tauri::command]
pub fn recording_status(state: State<AppState>) -> Option<SessionStatus> {
    state.session.lock().as_ref().map(|s| s.status())
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct RecordingFinished {
    dir: Option<PathBuf>,
    error: Option<String>,
}

#[tauri::command]
pub async fn stop_recording(app: AppHandle) -> CmdResult<PathBuf> {
    finish_recording(app).await
}

/// Stop, finalize, close the recording bar and open the result in the editor.
pub async fn finish_recording(app: AppHandle) -> CmdResult<PathBuf> {
    use tauri::Manager;
    let session = current_session(&app.state::<AppState>())?;
    let result = blocking(move || session.stop().map_err(err)).await;
    app.state::<AppState>().session.lock().take();
    win::close_recorder_bar(&app);
    win::show_main(&app);
    let payload = match &result {
        Ok(dir) => RecordingFinished { dir: Some(dir.clone()), error: None },
        Err(e) => RecordingFinished { dir: None, error: Some(e.clone()) },
    };
    let _ = app.emit("recording-finished", payload);
    result
}

#[tauri::command]
pub async fn discard_recording(app: AppHandle) -> CmdResult<()> {
    use tauri::Manager;
    let session = app.state::<AppState>().session.lock().take();
    if let Some(s) = session {
        blocking(move || {
            s.discard();
            Ok(())
        })
        .await?;
    }
    win::close_recorder_bar(&app);
    win::show_main(&app);
    Ok(())
}

// ---------------------------------------------------------------------------
// Projects
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn list_projects(app: AppHandle) -> CmdResult<Vec<ProjectSummary>> {
    use tauri::Manager;
    let root = app.state::<AppState>().settings.lock().output_dir();
    blocking(move || Ok(scan_projects(&root))).await
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenedProject {
    dir: PathBuf,
    project: Project,
    preview: PreviewModel,
    video_path: PathBuf,
    has_cursor: bool,
}

#[tauri::command]
pub async fn open_project(app: AppHandle, dir: String) -> CmdResult<OpenedProject> {
    let dir = paths::project_dir(&dir)?;
    allow_asset_dir(&app, &dir);
    blocking(move || {
        let project = Project::load(&dir).map_err(err)?;
        let track = project.load_cursor(&dir).map_err(err)?;
        let preview = build_preview(&project, track.as_ref());
        let video_path = studio_core::project::resolve_media(&dir, &project.media.video).map_err(err)?;
        if !video_path.is_file() {
            return Err("the recording's video file is missing".into());
        }
        Ok(OpenedProject { has_cursor: track.is_some(), dir, project, preview, video_path })
    })
    .await
}

fn with_edit(dir: &Path, edit: EditState) -> CmdResult<(Project, Option<studio_core::cursor::CursorTrack>)> {
    let mut project = Project::load(dir).map_err(err)?;
    project.edit = edit;
    project.sanitize();
    let track = project.load_cursor(dir).map_err(err)?;
    Ok((project, track))
}

/// Preview tracks for unsaved edit state (camera, cursor).
#[tauri::command]
pub async fn compute_preview(dir: String, edit: EditState) -> CmdResult<PreviewModel> {
    let dir = paths::project_dir(&dir)?;
    blocking(move || {
        let (project, track) = with_edit(&dir, edit)?;
        Ok(build_preview(&project, track.as_ref()))
    })
    .await
}

#[tauri::command]
pub async fn regenerate_zoom(dir: String, edit: EditState) -> CmdResult<Vec<ZoomSegment>> {
    let dir = paths::project_dir(&dir)?;
    blocking(move || {
        let (project, track) = with_edit(&dir, edit)?;
        Ok(regenerate_auto_segments(&project, track.as_ref()))
    })
    .await
}

/// Persist edit state (validated and sanitized). Returns the saved state.
#[tauri::command]
pub async fn save_project(dir: String, edit: EditState, export: Option<ExportSettings>) -> CmdResult<EditState> {
    let dir = paths::project_dir(&dir)?;
    blocking(move || {
        let mut project = Project::load(&dir).map_err(err)?;
        project.edit = edit;
        if let Some(e) = export {
            project.export = e;
        }
        project.sanitize();
        project.save(&dir).map_err(err)?;
        Ok(project.edit)
    })
    .await
}

#[tauri::command]
pub async fn rename_project(dir: String, name: String) -> CmdResult<String> {
    let dir = paths::project_dir(&dir)?;
    let name = name.trim().chars().filter(|c| !c.is_control()).take(120).collect::<String>();
    if name.is_empty() {
        return Err("name cannot be empty".into());
    }
    blocking(move || {
        let mut project = Project::load(&dir).map_err(err)?;
        project.name = name.clone();
        project.save(&dir).map_err(err)?;
        Ok(name)
    })
    .await
}

/// Move a project folder to the Recycle Bin (recoverable).
#[tauri::command]
pub async fn delete_project(app: AppHandle, dir: String) -> CmdResult<()> {
    use tauri::Manager;
    let dir = paths::project_dir(&dir)?;
    let root = app.state::<AppState>().settings.lock().output_dir();
    if !paths::is_within(&dir, &root) || dir == paths::dunce(root.canonicalize().map_err(err)?) {
        return Err("only projects inside the recordings folder can be deleted".into());
    }
    blocking(move || recycle(&dir)).await
}

#[cfg(windows)]
fn recycle(path: &Path) -> CmdResult<()> {
    use windows::core::PCWSTR;
    use windows::Win32::UI::Shell::{
        SHFileOperationW, FOF_ALLOWUNDO, FOF_NOCONFIRMATION, FOF_NOERRORUI, FOF_SILENT, FO_DELETE, SHFILEOPSTRUCTW,
    };
    let mut wide: Vec<u16> = path.as_os_str().encode_wide_vec();
    wide.extend([0, 0]);
    let mut op = SHFILEOPSTRUCTW {
        wFunc: FO_DELETE,
        pFrom: PCWSTR(wide.as_ptr()),
        fFlags: (FOF_ALLOWUNDO | FOF_NOCONFIRMATION | FOF_SILENT | FOF_NOERRORUI).0 as u16,
        ..Default::default()
    };
    let rc = unsafe { SHFileOperationW(&mut op) };
    if rc != 0 || op.fAnyOperationsAborted.as_bool() {
        return Err(format!("could not move the project to the Recycle Bin (code {rc})"));
    }
    Ok(())
}

#[cfg(windows)]
trait EncodeWideVec {
    fn encode_wide_vec(&self) -> Vec<u16>;
}
#[cfg(windows)]
impl EncodeWideVec for std::ffi::OsStr {
    fn encode_wide_vec(&self) -> Vec<u16> {
        use std::os::windows::ffi::OsStrExt;
        self.encode_wide().collect()
    }
}

#[cfg(not(windows))]
fn recycle(_: &Path) -> CmdResult<()> {
    Err("not supported".into())
}

/// Create a project from an existing video file (transcoded to H.264 so the
/// editor can play and scrub it). Imported videos have no cursor data.
#[tauri::command]
pub async fn import_video(app: AppHandle, path: String) -> CmdResult<PathBuf> {
    use tauri::Manager;
    let src = PathBuf::from(&path);
    if !src.is_file() {
        return Err("file not found".into());
    }
    let state = app.state::<AppState>();
    let ff = state.ffmpeg()?;
    let root = state.settings.lock().output_dir();
    let app2 = app.clone();
    let dir = blocking(move || {
        let encoders = app2.state::<AppState>().encoders();
        let info = probe_media(&ff.ffprobe, &src).map_err(err)?;
        if info.duration <= 0.0 {
            return Err("the video has no duration".into());
        }
        let stem = src.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "Imported".into());
        let name = sanitize_name(&stem);
        let mut dir = root.join(&name);
        let mut i = 2;
        while dir.exists() {
            dir = root.join(format!("{name} ({i})"));
            i += 1;
        }
        std::fs::create_dir_all(&dir).map_err(err)?;
        let fps = info.fps.round().clamp(10.0, 120.0) as u32;
        let (w, h) = (info.width & !1, info.height & !1);
        let encoder = studio_core::media::choose_encoder(&encoders, "auto").map_err(err)?;
        let spec = VideoEncodeSpec { width: w, height: h, fps, input_pix_fmt: "rgba", encoder: encoder.clone(), quality: Quality::Best, realtime: false };
        let mut codec = video_codec_args(&spec);
        // Replace the colour-conversion filter: inputs are already YUV video.
        if let Some(i) = codec.iter().position(|a| a == "-vf") {
            codec[i + 1] = format!("scale={w}:{h},fps={fps},format=nv12");
            if encoder == "libx264" {
                codec[i + 1] = format!("scale={w}:{h},fps={fps},format=yuv420p");
            }
        }
        let run = |args: Vec<String>| -> CmdResult<()> {
            let out = studio_core::media::command(&ff.ffmpeg)
                .args(["-hide_banner", "-loglevel", "error", "-y"])
                .args(&args)
                .stdin(std::process::Stdio::null())
                .output()
                .map_err(err)?;
            if out.status.success() {
                Ok(())
            } else {
                Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
            }
        };
        let fail = |e: String| {
            let _ = std::fs::remove_dir_all(&dir);
            e
        };
        let mut args = vec!["-i".to_string(), src.to_string_lossy().into_owned(), "-map".into(), "0:v:0".into(), "-an".into()];
        args.extend(codec);
        args.extend(["-movflags".into(), "+faststart".into(), dir.join("screen.mp4").to_string_lossy().into_owned()]);
        run(args).map_err(fail)?;
        let mut audio = None;
        if info.has_audio {
            let a = vec![
                "-i".into(), src.to_string_lossy().into_owned(), "-map".into(), "0:a:0".into(), "-vn".into(),
                "-ac".into(), "2".into(), "-ar".into(), "48000".into(), "-c:a".into(), "pcm_s16le".into(),
                dir.join("system.wav").to_string_lossy().into_owned(),
            ];
            if run(a).is_ok() {
                audio = Some("system.wav".to_string());
            }
        }
        let video = dir.join("screen.mp4");
        let vinfo = probe_media(&ff.ffprobe, &video).map_err(|e| fail(err(e)))?;
        let thumb = write_thumbnail(&ff.ffmpeg, &video, &dir.join("thumbnail.jpg"), (vinfo.duration / 2.0).min(1.0), 480)
            .ok()
            .map(|_| "thumbnail.jpg".to_string());
        let created_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        let project = Project::new_for_recording(
            stem,
            created_at,
            SourceInfo { kind: SourceKind::Imported, label: "Imported video".into(), width: w, height: h, fps, scale_factor: 1.0 },
            MediaRefs { video: "screen.mp4".into(), mic: None, system_audio: audio, cursor: None, thumbnail: thumb },
            RecordingInfo { duration: vinfo.duration, frames_written: 0, frames_dropped: 0, encoder, pause_count: 0 },
            None,
            Default::default(),
        );
        project.save(&dir).map_err(|e| fail(err(e)))?;
        Ok(dir)
    })
    .await?;
    allow_asset_dir(&app, &dir);
    Ok(dir)
}

// ---------------------------------------------------------------------------
// Crash recovery
// ---------------------------------------------------------------------------

/// Interrupted recordings in the recordings folder (excluding a live one).
#[tauri::command]
pub async fn list_unfinished(app: AppHandle) -> CmdResult<Vec<studio_engine::recovery::Unfinished>> {
    use tauri::Manager;
    let state = app.state::<AppState>();
    let root = state.settings.lock().output_dir();
    let active = state.session.lock().as_ref().map(|s| s.dir.clone());
    blocking(move || Ok(studio_engine::recovery::find_unfinished(&root, active.as_deref()))).await
}

fn unfinished_dir(app: &AppHandle, dir: &str) -> CmdResult<PathBuf> {
    use tauri::Manager;
    let state = app.state::<AppState>();
    let root = state.settings.lock().output_dir();
    let p = paths::dunce(PathBuf::from(dir).canonicalize().map_err(err)?);
    let is_active = state.session.lock().as_ref().is_some_and(|s| paths::is_within(&s.dir, &p) && paths::is_within(&p, &s.dir));
    if is_active || !paths::is_within(&p, &root) || !p.join(studio_engine::recovery::MANIFEST).is_file() {
        return Err("not an interrupted recording".into());
    }
    Ok(p)
}

#[tauri::command]
pub async fn recover_recording(app: AppHandle, dir: String) -> CmdResult<PathBuf> {
    use tauri::Manager;
    let dir = unfinished_dir(&app, &dir)?;
    let ff = app.state::<AppState>().ffmpeg()?;
    blocking(move || studio_engine::recovery::recover(&ff, &dir).map_err(err)).await
}

#[tauri::command]
pub async fn discard_unfinished(app: AppHandle, dir: String) -> CmdResult<()> {
    let dir = unfinished_dir(&app, &dir)?;
    blocking(move || recycle(&dir)).await
}

// ---------------------------------------------------------------------------
// Export
// ---------------------------------------------------------------------------

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ExportProgressEvent {
    job: String,
    progress: ExportProgress,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ExportFinishedEvent {
    job: String,
    result: Option<ExportResult>,
    error: Option<String>,
    cancelled: bool,
}

#[tauri::command]
pub fn start_export(
    app: AppHandle,
    state: State<AppState>,
    dir: String,
    output: String,
    settings: ExportSettings,
) -> CmdResult<String> {
    let dir = paths::project_dir(&dir)?;
    let output = paths::export_target(&output)?;
    if output.starts_with(&dir) && output.file_name().is_some_and(|n| n == "screen.mp4") {
        return Err("cannot overwrite the source recording".into());
    }
    let ff = state.ffmpeg()?;
    let job = uuid::Uuid::new_v4().simple().to_string();
    let cancel = Arc::new(AtomicBool::new(false));
    state.exports.lock().insert(job.clone(), cancel.clone());
    let app2 = app.clone();
    let job2 = job.clone();
    std::thread::Builder::new()
        .name("glide-export".into())
        .spawn(move || {
            use tauri::Manager;
            let encoders = app2.state::<AppState>().encoders();
            let res = export(
                &ff,
                &encoders,
                ExportRequest { project_dir: &dir, output: &output, settings: Some(settings) },
                &cancel,
                &mut |p| {
                    let _ = app2.emit("export-progress", ExportProgressEvent { job: job2.clone(), progress: p.clone() });
                },
            );
            app2.state::<AppState>().exports.lock().remove(&job2);
            let ev = match res {
                Ok(r) => ExportFinishedEvent { job: job2, result: Some(r), error: None, cancelled: false },
                Err(studio_core::Error::Cancelled) => {
                    ExportFinishedEvent { job: job2, result: None, error: None, cancelled: true }
                }
                Err(e) => {
                    log::error!("export failed: {e}");
                    ExportFinishedEvent { job: job2, result: None, error: Some(e.to_string()), cancelled: false }
                }
            };
            let _ = app2.emit("export-finished", ev);
        })
        .map_err(err)?;
    Ok(job)
}

#[tauri::command]
pub fn cancel_export(state: State<AppState>, job: String) {
    if let Some(flag) = state.exports.lock().get(&job) {
        flag.store(true, Ordering::SeqCst);
    }
}

#[tauri::command]
pub fn reveal_path(app: AppHandle, path: String) -> CmdResult<()> {
    use tauri_plugin_opener::OpenerExt;
    let p = PathBuf::from(path);
    if !p.exists() {
        return Err("file not found".into());
    }
    app.opener().reveal_item_in_dir(p).map_err(err)
}

/// Open an exported video with the default player (MP4 files only).
#[tauri::command]
pub fn open_path(app: AppHandle, path: String) -> CmdResult<()> {
    use tauri_plugin_opener::OpenerExt;
    let p = PathBuf::from(&path);
    if !p.is_file() || !p.extension().is_some_and(|e| e.eq_ignore_ascii_case("mp4")) {
        return Err("only exported MP4 files can be opened".into());
    }
    app.opener().open_path(path, None::<&str>).map_err(err)
}

/// Copy a user-chosen image into the project as PNG (the format the export
/// compositor reads). Each import gets a unique name so undo keeps working.
#[tauri::command]
pub async fn set_background_image(app: AppHandle, dir: String, path: String) -> CmdResult<String> {
    use tauri::Manager;
    let dir = paths::project_dir(&dir)?;
    let src = PathBuf::from(&path);
    if !src.is_file() {
        return Err("image not found".into());
    }
    let ff = app.state::<AppState>().ffmpeg()?;
    blocking(move || {
        let rel = format!("backgrounds/bg-{}.png", &uuid::Uuid::new_v4().simple().to_string()[..10]);
        let out = dir.join(&rel);
        std::fs::create_dir_all(out.parent().unwrap()).map_err(err)?;
        let res = studio_core::media::command(&ff.ffmpeg)
            .args(["-hide_banner", "-loglevel", "error", "-y", "-i"])
            .arg(&src)
            .args(["-frames:v", "1", "-vf", "scale='min(3840,iw)':-2,format=rgba"])
            .arg(&out)
            .stdin(std::process::Stdio::null())
            .output()
            .map_err(err)?;
        if !res.status.success() || !out.is_file() {
            return Err(format!("unsupported image: {}", String::from_utf8_lossy(&res.stderr).trim()));
        }
        Ok(rel)
    })
    .await
}
