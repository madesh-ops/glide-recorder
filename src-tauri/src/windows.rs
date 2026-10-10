//! Window helpers: the floating recording bar and capture exclusion.

use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};

pub const RECORDER_LABEL: &str = "recorder";
const BAR_W: f64 = 360.0;
const BAR_H: f64 = 64.0;

/// Show the floating recording controls at the bottom centre of the primary
/// monitor. The bar is excluded from screen capture so it never appears in
/// recordings.
pub fn open_recorder_bar(app: &AppHandle) -> tauri::Result<()> {
    if let Some(w) = app.get_webview_window(RECORDER_LABEL) {
        w.show()?;
        w.set_focus()?;
        return Ok(());
    }
    let mut builder = WebviewWindowBuilder::new(app, RECORDER_LABEL, WebviewUrl::App("index.html#/recorder".into()))
        .title("Glide — Recording")
        .inner_size(BAR_W, BAR_H)
        .resizable(false)
        .maximizable(false)
        .minimizable(false)
        .decorations(false)
        .transparent(true)
        .shadow(false)
        .always_on_top(true)
        .skip_taskbar(false)
        .focused(true);
    if let Ok(Some(m)) = app.primary_monitor() {
        let scale = m.scale_factor();
        let size = m.size().to_logical::<f64>(scale);
        let pos = m.position().to_logical::<f64>(scale);
        builder = builder.position(pos.x + (size.width - BAR_W) / 2.0, pos.y + size.height - BAR_H - 72.0);
    } else {
        builder = builder.center();
    }
    let window = builder.build()?;
    exclude_from_capture(&window);
    Ok(())
}

pub fn close_recorder_bar(app: &AppHandle) {
    if let Some(w) = app.get_webview_window(RECORDER_LABEL) {
        let _ = w.close();
    }
}

pub fn show_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
    }
}

pub fn hide_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.hide();
    }
}

#[cfg(windows)]
fn exclude_from_capture(window: &tauri::WebviewWindow) {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{SetWindowDisplayAffinity, WDA_EXCLUDEFROMCAPTURE};
    match window.hwnd() {
        Ok(hwnd) => unsafe {
            // Windows 10 2004+; older versions ignore it (the bar would then be visible).
            if let Err(e) = SetWindowDisplayAffinity(HWND(hwnd.0), WDA_EXCLUDEFROMCAPTURE) {
                log::warn!("could not exclude the recording bar from capture: {e}");
            }
        },
        Err(e) => log::warn!("no HWND for recorder bar: {e}"),
    }
}

#[cfg(not(windows))]
fn exclude_from_capture(_: &tauri::WebviewWindow) {}
