//! Cursor and click tracking.
//!
//! * A low-level mouse hook (`WH_MOUSE_LL`) on its own message-loop thread
//!   timestamps button presses/releases the moment they happen.
//! * A poll thread samples `GetCursorInfo` at 120 Hz for position, visibility
//!   and shape, and tracks the captured window's bounds.
//! * Each distinct cursor shape is extracted once (`GetIconInfoExW` +
//!   `GetDIBits`) and written to `cursors/cursor_N.png` with its hotspot.
//!
//! Positions are physical pixels relative to the captured area. Clicks on
//! Glide's own windows (e.g. the recording controls) are ignored.

use crate::clock::SessionClock;
use crate::sources::window_rect;
use anyhow::Result;
use crossbeam_channel::{bounded, Receiver, Sender};
use parking_lot::Mutex;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, OnceLock};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use std::io::Write;
use studio_core::cursor::{
    ClickEvent, CursorLogEntry, CursorSample, CursorShape, CursorTrack, MouseButton, CURSOR_TRACK_VERSION,
};

/// Append-only log replayed by crash recovery.
pub const CURSOR_LOG: &str = "cursor.log";

struct CursorLog(Option<std::io::BufWriter<std::fs::File>>);

impl CursorLog {
    fn open(dir: &Path) -> Self {
        match std::fs::File::create(dir.join(CURSOR_LOG)) {
            Ok(f) => Self(Some(std::io::BufWriter::new(f))),
            Err(e) => {
                log::warn!("cursor log unavailable (crash recovery will lack cursor data): {e}");
                Self(None)
            }
        }
    }
    fn write(&mut self, e: &CursorLogEntry) {
        if let Some(w) = &mut self.0 {
            if serde_json::to_writer(&mut *w, e).is_err() || w.write_all(b"
").is_err() {
                self.0 = None;
            }
        }
    }
    fn flush(&mut self) {
        if let Some(w) = &mut self.0 {
            let _ = w.flush();
        }
    }
}
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    DeleteObject, GetDC, GetDIBits, GetObjectW, ReleaseDC, BITMAP, BITMAPINFO, BITMAPINFOHEADER, BI_RGB,
    DIB_RGB_COLORS, HBITMAP, HGDIOBJ,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::{GetCurrentProcessId, GetCurrentThreadId};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, DispatchMessageW, GetAncestor, GetCursorInfo, GetIconInfoExW, GetMessageW,
    GetWindowThreadProcessId, LoadCursorW, PostThreadMessageW, SetWindowsHookExW, TranslateMessage,
    UnhookWindowsHookEx, WindowFromPoint, CURSORINFO, CURSOR_SHOWING, GA_ROOT, HCURSOR, HHOOK, ICONINFOEXW,
    IDC_APPSTARTING, IDC_ARROW, IDC_CROSS, IDC_HAND, IDC_IBEAM, IDC_NO, IDC_SIZEALL, IDC_SIZENESW, IDC_SIZENS,
    IDC_SIZENWSE, IDC_SIZEWE, IDC_WAIT, MSG, MSLLHOOKSTRUCT, WH_MOUSE_LL, WM_LBUTTONDOWN, WM_LBUTTONUP,
    WM_MBUTTONDOWN, WM_MBUTTONUP, WM_QUIT, WM_RBUTTONDOWN, WM_RBUTTONUP,
};

/// Where the captured area is on the virtual desktop.
#[derive(Debug, Clone, Copy)]
pub enum CaptureArea {
    /// Fixed rectangle (a display), physical pixels.
    Fixed { x: i32, y: i32 },
    /// A window whose position is tracked while recording.
    Window(isize),
}

#[derive(Debug, Clone, Copy)]
struct RawButton {
    at: Instant,
    pt: POINT,
    button: MouseButton,
    down: bool,
}

// The hook procedure is a plain function; it reaches the active recorder through this.
static HOOK_SINK: OnceLock<Mutex<Option<Sender<RawButton>>>> = OnceLock::new();

fn hook_sink() -> &'static Mutex<Option<Sender<RawButton>>> {
    HOOK_SINK.get_or_init(|| Mutex::new(None))
}

unsafe extern "system" fn mouse_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code >= 0 {
        let msg = wparam.0 as u32;
        let ev = match msg {
            WM_LBUTTONDOWN => Some((MouseButton::Left, true)),
            WM_LBUTTONUP => Some((MouseButton::Left, false)),
            WM_RBUTTONDOWN => Some((MouseButton::Right, true)),
            WM_RBUTTONUP => Some((MouseButton::Right, false)),
            WM_MBUTTONDOWN => Some((MouseButton::Middle, true)),
            WM_MBUTTONUP => Some((MouseButton::Middle, false)),
            _ => None,
        };
        if let Some((button, down)) = ev {
            let info = unsafe { &*(lparam.0 as *const MSLLHOOKSTRUCT) };
            // Must return quickly: never block here.
            if let Some(tx) = hook_sink().lock().as_ref() {
                let _ = tx.try_send(RawButton { at: Instant::now(), pt: info.pt, button, down });
            }
        }
    }
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

struct Shared {
    clock: Arc<SessionClock>,
    samples: Mutex<Vec<CursorSample>>,
    clicks: Mutex<Vec<ClickEvent>>,
    shapes: Mutex<Vec<CursorShape>>,
    width: u32,
    height: u32,
}

pub struct InputRecorder {
    shared: Arc<Shared>,
    stop: Arc<AtomicBool>,
    hook_thread: Option<JoinHandle<()>>,
    hook_thread_id: Arc<AtomicU32>,
    poll_thread: Option<JoinHandle<()>>,
}

impl InputRecorder {
    /// Start tracking. `cursor_dir` receives extracted cursor PNGs; `width`/`height`
    /// are the capture size used to discard far-off-screen positions.
    pub fn start(
        clock: Arc<SessionClock>,
        area: CaptureArea,
        width: u32,
        height: u32,
        project_dir: PathBuf,
    ) -> Result<Self> {
        let shared = Arc::new(Shared {
            clock,
            samples: Mutex::new(Vec::with_capacity(1 << 16)),
            clicks: Mutex::new(Vec::new()),
            shapes: Mutex::new(Vec::new()),
            width,
            height,
        });
        let stop = Arc::new(AtomicBool::new(false));
        let (tx, rx) = bounded::<RawButton>(4096);
        *hook_sink().lock() = Some(tx);

        let hook_thread_id = Arc::new(AtomicU32::new(0));
        let tid = hook_thread_id.clone();
        let hook_thread = std::thread::Builder::new().name("studio-mouse-hook".into()).spawn(move || unsafe {
            tid.store(GetCurrentThreadId(), Ordering::SeqCst);
            let hmod = GetModuleHandleW(None).ok();
            let hook: Option<HHOOK> = match SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_hook), hmod.map(|m| m.into()), 0) {
                Ok(h) => Some(h),
                Err(e) => {
                    log::error!("mouse hook failed: {e}; clicks will not be recorded");
                    None
                }
            };
            let mut msg = MSG::default();
            while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            if let Some(h) = hook {
                let _ = UnhookWindowsHookEx(h);
            }
        })?;

        let poll_shared = shared.clone();
        let poll_stop = stop.clone();
        let poll_thread = std::thread::Builder::new()
            .name("studio-cursor-poll".into())
            .spawn(move || poll_loop(poll_shared, poll_stop, rx, area, project_dir))?;

        Ok(Self {
            shared,
            stop,
            hook_thread: Some(hook_thread),
            hook_thread_id,
            poll_thread: Some(poll_thread),
        })
    }

    /// Stop all threads and return the recorded track.
    pub fn finish(mut self) -> CursorTrack {
        self.shutdown();
        CursorTrack {
            version: CURSOR_TRACK_VERSION,
            source_width: self.shared.width,
            source_height: self.shared.height,
            samples: std::mem::take(&mut *self.shared.samples.lock()),
            clicks: std::mem::take(&mut *self.shared.clicks.lock()),
            shapes: std::mem::take(&mut *self.shared.shapes.lock()),
        }
    }

    fn shutdown(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        *hook_sink().lock() = None;
        let tid = self.hook_thread_id.load(Ordering::SeqCst);
        if tid != 0 {
            unsafe {
                let _ = PostThreadMessageW(tid, WM_QUIT, WPARAM(0), LPARAM(0));
            }
        }
        if let Some(h) = self.hook_thread.take() {
            let _ = h.join();
        }
        if let Some(h) = self.poll_thread.take() {
            let _ = h.join();
        }
    }
}

impl Drop for InputRecorder {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn own_window_at(pt: POINT) -> bool {
    unsafe {
        let hwnd = WindowFromPoint(pt);
        if hwnd.0.is_null() {
            return false;
        }
        let root = GetAncestor(hwnd, GA_ROOT);
        let mut pid = 0u32;
        GetWindowThreadProcessId(if root.0.is_null() { hwnd } else { root }, Some(&mut pid));
        pid == GetCurrentProcessId()
    }
}

fn poll_loop(sh: Arc<Shared>, stop: Arc<AtomicBool>, rx: Receiver<RawButton>, area: CaptureArea, dir: PathBuf) {
    let period = Duration::from_micros(8_333); // 120 Hz
    let mut origin = match area {
        CaptureArea::Fixed { x, y } => (x, y),
        CaptureArea::Window(h) => window_rect(HWND(h as *mut _)).map(|r| (r.left, r.top)).unwrap_or((0, 0)),
    };
    let mut shapes: HashMap<isize, u32> = HashMap::new();
    let mut last: Option<(i32, i32, u32, bool)> = None;
    let mut tick: u64 = 0;
    let mut next = Instant::now();
    let mut log = CursorLog::open(&dir);
    let margin = 64i32;
    let in_bounds = |x: i32, y: i32| {
        x >= -margin && y >= -margin && x <= sh.width as i32 + margin && y <= sh.height as i32 + margin
    };

    while !stop.load(Ordering::Relaxed) {
        // Window capture: follow the window as it moves.
        if let CaptureArea::Window(h) = area {
            if tick % 6 == 0 {
                if let Some(r) = window_rect(HWND(h as *mut _)) {
                    origin = (r.left, r.top);
                }
            }
        }
        tick += 1;

        // Clicks from the hook.
        while let Ok(b) = rx.try_recv() {
            let Some(t) = sh.clock.at(b.at) else { continue };
            let (x, y) = (b.pt.x - origin.0, b.pt.y - origin.1);
            if b.down {
                if own_window_at(b.pt) || !in_bounds(x, y) {
                    continue;
                }
                let click = ClickEvent { t, x: x as f32, y: y as f32, button: b.button, up: None };
                log.write(&CursorLogEntry::Click(click));
                sh.clicks.lock().push(click);
            } else if let Some(c) = sh.clicks.lock().iter_mut().rev().find(|c| c.button == b.button && c.up.is_none()) {
                c.up = Some(t);
                log.write(&CursorLogEntry::ClickUp { button: b.button, t });
            }
        }

        // Cursor position / shape / visibility.
        let now = Instant::now();
        if let Some(t) = sh.clock.at(now) {
            let mut ci = CURSORINFO { cbSize: std::mem::size_of::<CURSORINFO>() as u32, ..Default::default() };
            if unsafe { GetCursorInfo(&mut ci) }.is_ok() {
                let visible = ci.flags.0 & CURSOR_SHOWING.0 != 0 && !ci.hCursor.0.is_null();
                let shape = if visible {
                    let key = ci.hCursor.0 as isize;
                    match shapes.get(&key) {
                        Some(&i) => i,
                        None => {
                            // Read the count first: a guard in the match scrutinee would
                            // live for the whole match and deadlock the push below.
                            let next_index = sh.shapes.lock().len();
                            let idx = match extract_cursor(ci.hCursor, &dir, next_index) {
                                Ok(s) => {
                                    log.write(&CursorLogEntry::Shape(s.clone()));
                                    let mut v = sh.shapes.lock();
                                    v.push(s);
                                    (v.len() - 1) as u32
                                }
                                Err(e) => {
                                    log::warn!("cursor shape extraction failed: {e}");
                                    u32::MAX
                                }
                            };
                            shapes.insert(key, idx);
                            idx
                        }
                    }
                } else {
                    last.map(|l| l.2).unwrap_or(u32::MAX)
                };
                let (x, y) = (ci.ptScreenPos.x - origin.0, ci.ptScreenPos.y - origin.1);
                let cur = (x, y, shape, visible);
                if last != Some(cur) {
                    last = Some(cur);
                    let sample = CursorSample { t, x: x as f32, y: y as f32, shape, visible };
                    log.write(&CursorLogEntry::Sample(sample));
                    sh.samples.lock().push(sample);
                }
            }
        }

        if tick % 30 == 0 {
            log.flush(); // at most ~250 ms of cursor data is lost on a crash
        }
        next += period;
        let now = Instant::now();
        if next > now {
            std::thread::sleep(next - now);
        } else {
            next = now;
        }
    }
    log.flush();
}

fn standard_cursor_kind(h: HCURSOR) -> Option<&'static str> {
    let ids = [
        (IDC_ARROW, "arrow"),
        (IDC_IBEAM, "ibeam"),
        (IDC_HAND, "hand"),
        (IDC_WAIT, "wait"),
        (IDC_APPSTARTING, "appstarting"),
        (IDC_CROSS, "crosshair"),
        (IDC_SIZEALL, "move"),
        (IDC_SIZEWE, "ew-resize"),
        (IDC_SIZENS, "ns-resize"),
        (IDC_SIZENWSE, "nwse-resize"),
        (IDC_SIZENESW, "nesw-resize"),
        (IDC_NO, "not-allowed"),
    ];
    ids.iter().find_map(|(id, name)| {
        let std = unsafe { LoadCursorW(None, *id) }.ok()?;
        (std.0 == h.0).then_some(*name)
    })
}

struct GdiBitmap(HBITMAP);
impl Drop for GdiBitmap {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            unsafe {
                let _ = DeleteObject(HGDIOBJ(self.0 .0));
            }
        }
    }
}

/// Read an HBITMAP as top-down 32-bit BGRA.
fn read_bitmap(hbm: HBITMAP) -> Option<(u32, u32, Vec<u8>)> {
    unsafe {
        let mut bm = BITMAP::default();
        if GetObjectW(HGDIOBJ(hbm.0), std::mem::size_of::<BITMAP>() as i32, Some(&mut bm as *mut _ as *mut _)) == 0 {
            return None;
        }
        let (w, h) = (bm.bmWidth.max(0) as u32, bm.bmHeight.max(0) as u32);
        if w == 0 || h == 0 || w > 1024 || h > 2048 {
            return None;
        }
        let mut bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w as i32,
                biHeight: -(h as i32),
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut buf = vec![0u8; (w * h * 4) as usize];
        let dc = GetDC(None);
        let lines = GetDIBits(dc, hbm, 0, h, Some(buf.as_mut_ptr() as *mut _), &mut bmi, DIB_RGB_COLORS);
        ReleaseDC(None, dc);
        (lines > 0).then_some((w, h, buf))
    }
}

/// Convert a cursor to straight RGBA + hotspot and write it as PNG.
fn extract_cursor(h: HCURSOR, project_dir: &Path, index: usize) -> Result<CursorShape> {
    let mut info = ICONINFOEXW { cbSize: std::mem::size_of::<ICONINFOEXW>() as u32, ..Default::default() };
    if !unsafe { GetIconInfoExW(h.into(), &mut info) }.as_bool() {
        anyhow::bail!("GetIconInfoExW failed");
    }
    let mask = GdiBitmap(info.hbmMask);
    let color = GdiBitmap(info.hbmColor);
    let (w, h_px, rgba) = if !color.0.is_invalid() {
        let (w, h, bgra) = read_bitmap(color.0).ok_or_else(|| anyhow::anyhow!("color bitmap unreadable"))?;
        let has_alpha = bgra.chunks_exact(4).any(|p| p[3] != 0);
        let and_mask = if has_alpha { None } else { read_bitmap(mask.0) };
        let mut out = Vec::with_capacity(bgra.len());
        for (i, p) in bgra.chunks_exact(4).enumerate() {
            let a = match &and_mask {
                // Legacy colour cursor: AND mask bit set (white in the DIB) = transparent.
                Some((_, _, m)) => {
                    if m[i * 4] > 127 {
                        0
                    } else {
                        255
                    }
                }
                None => p[3],
            };
            // Windows colour cursors with alpha are premultiplied; un-premultiply.
            let un = |c: u8| if has_alpha && a > 0 { ((c as u32 * 255 + a as u32 / 2) / a as u32).min(255) as u8 } else { c };
            out.extend_from_slice(&[un(p[2]), un(p[1]), un(p[0]), a]);
        }
        (w, h, out)
    } else {
        // Monochrome: mask is twice as tall, AND on top, XOR at the bottom.
        let (w, h2, m) = read_bitmap(mask.0).ok_or_else(|| anyhow::anyhow!("mask bitmap unreadable"))?;
        let h = h2 / 2;
        let mut out = Vec::with_capacity((w * h * 4) as usize);
        for i in 0..(w * h) as usize {
            let and = m[i * 4] > 127;
            let xor = m[(i + (w * h) as usize) * 4] > 127;
            // AND=1,XOR=0 transparent; AND=0: XOR selects white/black; AND=1,XOR=1
            // (screen inversion) can't be represented and is drawn black with full alpha.
            let px = match (and, xor) {
                (true, false) => [0, 0, 0, 0],
                (false, false) => [0, 0, 0, 255],
                (false, true) => [255, 255, 255, 255],
                (true, true) => [0, 0, 0, 255],
            };
            out.extend_from_slice(&px);
        }
        (w, h, out)
    };

    let rel = format!("cursors/cursor_{index}.png");
    let path = project_dir.join(&rel);
    std::fs::create_dir_all(path.parent().unwrap())?;
    let file = std::io::BufWriter::new(std::fs::File::create(&path)?);
    let mut enc = png::Encoder::new(file, w, h_px);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header()?.write_image_data(&rgba)?;
    Ok(CursorShape {
        file: rel,
        width: w,
        height: h_px,
        hotspot_x: info.xHotspot,
        hotspot_y: info.yHotspot,
        kind: standard_cursor_kind(h).map(str::to_string),
    })
}
