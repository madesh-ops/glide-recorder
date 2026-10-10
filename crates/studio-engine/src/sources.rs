//! Enumerate capturable displays and windows (physical pixel coordinates).

use serde::Serialize;
use windows::core::{BOOL, PWSTR};
use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM, RECT};
use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED, DWMWA_EXTENDED_FRAME_BOUNDS};
use windows::Win32::Graphics::Gdi::{EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFOEXW};
use windows::Win32::System::Threading::{
    GetCurrentProcessId, OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::HiDpi::{
    GetDpiForMonitor, SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, MDT_EFFECTIVE_DPI,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetWindowLongW, GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId, IsIconic,
    IsWindow, IsWindowVisible, GWL_EXSTYLE, WS_EX_TOOLWINDOW,
};

/// Make coordinates physical pixels. Harmless if awareness was already set
/// (e.g. by the application manifest or the windowing library).
pub fn ensure_dpi_awareness() {
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DisplayInfo {
    /// Stable-ish identifier: the GDI device name (e.g. `\\.\DISPLAY1`).
    pub id: String,
    pub name: String,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub scale: f64,
    pub primary: bool,
    #[serde(skip)]
    pub handle: isize,
}

impl DisplayInfo {
    pub fn hmonitor(&self) -> HMONITOR {
        HMONITOR(self.handle as *mut _)
    }
}

pub fn list_displays() -> Vec<DisplayInfo> {
    unsafe extern "system" fn cb(hm: HMONITOR, _: HDC, _: *mut RECT, data: LPARAM) -> BOOL {
        let out = unsafe { &mut *(data.0 as *mut Vec<HMONITOR>) };
        out.push(hm);
        BOOL(1)
    }
    let mut handles: Vec<HMONITOR> = Vec::new();
    unsafe {
        let _ = EnumDisplayMonitors(None, None, Some(cb), LPARAM(&mut handles as *mut _ as isize));
    }
    let mut out = Vec::new();
    for (i, hm) in handles.into_iter().enumerate() {
        let mut mi = MONITORINFOEXW::default();
        mi.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
        if !unsafe { GetMonitorInfoW(hm, &mut mi as *mut _ as *mut _) }.as_bool() {
            continue;
        }
        let r = mi.monitorInfo.rcMonitor;
        let (mut dx, mut dy) = (96u32, 96u32);
        let scale = if unsafe { GetDpiForMonitor(hm, MDT_EFFECTIVE_DPI, &mut dx, &mut dy) }.is_ok() {
            dx as f64 / 96.0
        } else {
            1.0
        };
        let dev = String::from_utf16_lossy(&mi.szDevice).trim_end_matches('\0').to_string();
        let primary = mi.monitorInfo.dwFlags & 1 != 0;
        out.push(DisplayInfo {
            id: dev,
            name: format!("Display {}", i + 1),
            x: r.left,
            y: r.top,
            width: (r.right - r.left).max(0) as u32,
            height: (r.bottom - r.top).max(0) as u32,
            scale,
            primary,
            handle: hm.0 as isize,
        });
    }
    // Primary first, then left-to-right.
    out.sort_by_key(|d| (!d.primary, d.x, d.y));
    for (i, d) in out.iter_mut().enumerate() {
        d.name = if d.primary { "Built-in / Primary Display".into() } else { format!("Display {}", i + 1) };
    }
    out
}

pub fn find_display(id: &str) -> Option<DisplayInfo> {
    list_displays().into_iter().find(|d| d.id == id)
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowInfo {
    pub id: u64,
    pub title: String,
    pub app: String,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl WindowInfo {
    pub fn hwnd(&self) -> HWND {
        HWND(self.id as usize as *mut _)
    }
}

/// Visible frame bounds of a window (excludes the invisible resize border).
pub fn window_rect(hwnd: HWND) -> Option<RECT> {
    let mut r = RECT::default();
    unsafe {
        DwmGetWindowAttribute(hwnd, DWMWA_EXTENDED_FRAME_BOUNDS, &mut r as *mut _ as *mut _, std::mem::size_of::<RECT>() as u32)
            .ok()?;
    }
    (r.right > r.left && r.bottom > r.top).then_some(r)
}

pub fn is_window_alive(hwnd: HWND) -> bool {
    unsafe { IsWindow(Some(hwnd)).as_bool() }
}

fn process_name(pid: u32) -> String {
    unsafe {
        let Ok(h) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else { return String::new() };
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len).is_ok();
        let _ = CloseHandle(h);
        if !ok {
            return String::new();
        }
        let path = String::from_utf16_lossy(&buf[..len as usize]);
        std::path::Path::new(&path).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
    }
}

pub fn list_windows() -> Vec<WindowInfo> {
    unsafe extern "system" fn cb(hwnd: HWND, data: LPARAM) -> BOOL {
        let out = unsafe { &mut *(data.0 as *mut Vec<HWND>) };
        out.push(hwnd);
        BOOL(1)
    }
    let mut hwnds: Vec<HWND> = Vec::new();
    unsafe {
        let _ = EnumWindows(Some(cb), LPARAM(&mut hwnds as *mut _ as isize));
    }
    let own_pid = unsafe { GetCurrentProcessId() };
    let mut out = Vec::new();
    for hwnd in hwnds {
        unsafe {
            if !IsWindowVisible(hwnd).as_bool() || IsIconic(hwnd).as_bool() {
                continue;
            }
            if (GetWindowLongW(hwnd, GWL_EXSTYLE) as u32) & WS_EX_TOOLWINDOW.0 != 0 {
                continue;
            }
            let mut cloaked = 0u32;
            if DwmGetWindowAttribute(hwnd, DWMWA_CLOAKED, &mut cloaked as *mut _ as *mut _, 4).is_ok() && cloaked != 0 {
                continue;
            }
            let len = GetWindowTextLengthW(hwnd);
            if len <= 0 {
                continue;
            }
            let mut buf = vec![0u16; len as usize + 1];
            let n = GetWindowTextW(hwnd, &mut buf);
            let title = String::from_utf16_lossy(&buf[..n.max(0) as usize]);
            let mut pid = 0u32;
            GetWindowThreadProcessId(hwnd, Some(&mut pid));
            if pid == own_pid {
                continue;
            }
            let Some(r) = window_rect(hwnd) else { continue };
            let (w, h) = ((r.right - r.left) as u32, (r.bottom - r.top) as u32);
            if w < 64 || h < 64 {
                continue;
            }
            let app = process_name(pid);
            // Shell surfaces that are technically windows but not useful to record.
            if app.eq_ignore_ascii_case("TextInputHost") || title == "Program Manager" {
                continue;
            }
            out.push(WindowInfo { id: hwnd.0 as usize as u64, title, app, x: r.left, y: r.top, width: w, height: h });
        }
    }
    out
}
