//! Windows Graphics Capture (WGC) for monitors and windows.
//!
//! Frames arrive on a free-threaded frame pool, are copied GPU→staging→CPU
//! (BGRA) and published into a single-slot [`FrameSlot`]. Only the *latest*
//! frame is kept; buffers are recycled, so memory is bounded at a few frames.
//! The system cursor is excluded from capture — Glide records cursor metadata
//! separately and renders its own cursor.

use anyhow::{anyhow, Context, Result};
use parking_lot::{Condvar, Mutex};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use windows::core::Interface;
use windows::Foundation::TypedEventHandler;
use windows::Graphics::Capture::{Direct3D11CaptureFramePool, GraphicsCaptureItem, GraphicsCaptureSession};
use windows::Graphics::DirectX::Direct3D11::IDirect3DDevice;
use windows::Graphics::DirectX::DirectXPixelFormat;
use windows::Graphics::SizeInt32;
use windows::Win32::Foundation::{HMODULE, HWND};
use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_HARDWARE;
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D, D3D11_CPU_ACCESS_READ,
    D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_MAPPED_SUBRESOURCE, D3D11_MAP_READ, D3D11_SDK_VERSION,
    D3D11_TEXTURE2D_DESC, D3D11_USAGE_STAGING,
};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC};
use windows::Win32::Graphics::Dxgi::IDXGIDevice;
use windows::Win32::Graphics::Gdi::HMONITOR;
use windows::Win32::System::WinRT::Direct3D11::{CreateDirect3D11DeviceFromDXGIDevice, IDirect3DDxgiInterfaceAccess};
use windows::Win32::System::WinRT::Graphics::Capture::IGraphicsCaptureItemInterop;

#[derive(Debug, Clone, Copy)]
pub enum CaptureTarget {
    Display(HMONITOR),
    Window(HWND),
}

// HMONITOR / HWND are plain handles.
unsafe impl Send for CaptureTarget {}

/// A captured BGRA frame of fixed output size.
pub struct Frame {
    pub data: Vec<u8>,
    /// QPC-based capture time in 100 ns units (`SystemRelativeTime`).
    pub time_100ns: i64,
}

#[derive(Default)]
struct SlotState {
    latest: Option<Frame>,
    spare: Vec<Vec<u8>>,
    closed: bool,
    error: Option<String>,
}

/// Single-slot mailbox between the capture callback and the video pacer.
#[derive(Default)]
pub struct FrameSlot {
    state: Mutex<SlotState>,
    cond: Condvar,
    pub frames_arrived: AtomicU64,
}

impl FrameSlot {
    fn spare(&self, len: usize) -> Vec<u8> {
        let mut st = self.state.lock();
        match st.spare.pop() {
            Some(mut b) if b.len() == len => {
                b.iter_mut().for_each(|x| *x = 0);
                b
            }
            _ => vec![0u8; len],
        }
    }

    fn publish(&self, frame: Frame) {
        let mut st = self.state.lock();
        if let Some(old) = st.latest.replace(frame) {
            if st.spare.len() < 3 {
                st.spare.push(old.data);
            }
        }
        drop(st);
        self.frames_arrived.fetch_add(1, Ordering::Relaxed);
        self.cond.notify_all();
    }

    pub fn recycle(&self, buf: Vec<u8>) {
        let mut st = self.state.lock();
        if st.spare.len() < 3 {
            st.spare.push(buf);
        }
    }

    /// Take the newest frame published since the last call, if any.
    pub fn take_latest(&self) -> Option<Frame> {
        self.state.lock().latest.take()
    }

    /// Wait up to `timeout` for a first/new frame.
    pub fn wait_for_frame(&self, timeout: Duration) -> bool {
        let mut st = self.state.lock();
        if st.latest.is_some() {
            return true;
        }
        let _ = self.cond.wait_for(&mut st, timeout);
        st.latest.is_some()
    }

    pub fn close(&self, error: Option<String>) {
        let mut st = self.state.lock();
        st.closed = true;
        if st.error.is_none() {
            st.error = error;
        }
        drop(st);
        self.cond.notify_all();
    }

    pub fn closed(&self) -> (bool, Option<String>) {
        let st = self.state.lock();
        (st.closed, st.error.clone())
    }
}

/// WinRT device wrapper. The object returned by
/// `CreateDirect3D11DeviceFromDXGIDevice` is free-threaded (agile).
struct AgileDevice(IDirect3DDevice);
unsafe impl Send for AgileDevice {}
unsafe impl Sync for AgileDevice {}

struct CopyState {
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    winrt_device: AgileDevice,
    staging: Option<(ID3D11Texture2D, u32, u32)>,
    pool_size: SizeInt32,
    out_w: u32,
    out_h: u32,
    /// Top-left of the captured region inside the source surface.
    crop_x: u32,
    crop_y: u32,
}

/// A sub-rectangle of the source to record (physical pixels, source-relative).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Crop {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

pub struct Capture {
    session: GraphicsCaptureSession,
    pool: Direct3D11CaptureFramePool,
    _item: GraphicsCaptureItem,
    token: i64,
    pub width: u32,
    pub height: u32,
    stopped: Arc<AtomicBool>,
}

fn create_d3d_device() -> Result<(ID3D11Device, ID3D11DeviceContext)> {
    let mut device = None;
    let mut context = None;
    unsafe {
        D3D11CreateDevice(
            None,
            D3D_DRIVER_TYPE_HARDWARE,
            HMODULE::default(),
            D3D11_CREATE_DEVICE_BGRA_SUPPORT,
            None,
            D3D11_SDK_VERSION,
            Some(&mut device),
            None,
            Some(&mut context),
        )
        .context("D3D11CreateDevice failed (is a GPU driver installed?)")?;
    }
    Ok((device.unwrap(), context.unwrap()))
}

pub fn is_supported() -> bool {
    GraphicsCaptureSession::IsSupported().unwrap_or(false)
}

impl Capture {
    /// Start capturing `target`. Frames are cropped/padded to the initial size
    /// (rounded down to even dimensions for H.264).
    pub fn start(target: CaptureTarget, slot: Arc<FrameSlot>) -> Result<Capture> {
        Self::start_cropped(target, slot, None)
    }

    /// Like [`Capture::start`], recording only `crop` of the source.
    pub fn start_cropped(target: CaptureTarget, slot: Arc<FrameSlot>, crop: Option<Crop>) -> Result<Capture> {
        if !is_supported() {
            return Err(anyhow!("Windows Graphics Capture is not supported on this system (requires Windows 10 1903+)"));
        }
        let interop = windows::core::factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>()?;
        let item: GraphicsCaptureItem = unsafe {
            match target {
                CaptureTarget::Display(hm) => interop.CreateForMonitor(hm),
                CaptureTarget::Window(hwnd) => interop.CreateForWindow(hwnd),
            }
        }
        .context("could not create a capture item for the selected source")?;
        let size = item.Size()?;
        if size.Width < 2 || size.Height < 2 {
            return Err(anyhow!("the selected source has no visible area"));
        }
        let (sw, sh) = (size.Width as u32, size.Height as u32);
        let crop = match crop {
            Some(c) => {
                if c.w < 16 || c.h < 16 || c.x.saturating_add(c.w) > sw || c.y.saturating_add(c.h) > sh {
                    return Err(anyhow!("the selected area ({}×{} at {},{}) is outside the {sw}×{sh} display", c.w, c.h, c.x, c.y));
                }
                c
            }
            None => Crop { x: 0, y: 0, w: sw, h: sh },
        };
        let out_w = crop.w & !1;
        let out_h = crop.h & !1;

        let (device, context) = create_d3d_device()?;
        let dxgi: IDXGIDevice = device.cast()?;
        let winrt_device: IDirect3DDevice = unsafe { CreateDirect3D11DeviceFromDXGIDevice(&dxgi)? }.cast()?;
        let pool = Direct3D11CaptureFramePool::CreateFreeThreaded(
            &winrt_device,
            DirectXPixelFormat::B8G8R8A8UIntNormalized,
            2,
            size,
        )?;
        let session = pool.CreateCaptureSession(&item)?;
        // Cursor is recorded as metadata and re-rendered.
        let _ = session.SetIsCursorCaptureEnabled(false);
        // Windows 11: hide the yellow capture border where permitted.
        let _ = session.SetIsBorderRequired(false);

        let state = Arc::new(Mutex::new(CopyState {
            device,
            context,
            winrt_device: AgileDevice(winrt_device),
            staging: None,
            pool_size: size,
            out_w,
            out_h,
            crop_x: crop.x,
            crop_y: crop.y,
        }));
        let stopped = Arc::new(AtomicBool::new(false));
        let handler_slot = slot.clone();
        let handler_stopped = stopped.clone();
        let handler = TypedEventHandler::<Direct3D11CaptureFramePool, windows::core::IInspectable>::new(
            move |pool, _| {
                if handler_stopped.load(Ordering::Relaxed) {
                    return Ok(());
                }
                let Some(pool) = pool.as_ref() else { return Ok(()) };
                if let Err(e) = on_frame(pool, &state, &handler_slot) {
                    log::warn!("capture frame error: {e:?}");
                }
                Ok(())
            },
        );
        let token = pool.FrameArrived(&handler)?;
        let closed_slot = slot.clone();
        item.Closed(&TypedEventHandler::<GraphicsCaptureItem, windows::core::IInspectable>::new(move |_, _| {
            closed_slot.close(Some("the captured window or display was closed".into()));
            Ok(())
        }))?;
        session.StartCapture()?;
        Ok(Capture { session, pool, _item: item, token, width: out_w, height: out_h, stopped })
    }

    pub fn stop(self) {
        self.stopped.store(true, Ordering::Relaxed);
        let _ = self.pool.RemoveFrameArrived(self.token);
        let _ = self.session.Close();
        let _ = self.pool.Close();
    }
}

fn on_frame(pool: &Direct3D11CaptureFramePool, state: &Mutex<CopyState>, slot: &FrameSlot) -> Result<()> {
    let frame = pool.TryGetNextFrame()?;
    let time = frame.SystemRelativeTime()?.Duration;
    let content = frame.ContentSize()?;
    let mut st = state.lock();
    let surface = frame.Surface()?;
    let access: IDirect3DDxgiInterfaceAccess = surface.cast()?;
    let tex: ID3D11Texture2D = unsafe { access.GetInterface()? };
    let mut desc = D3D11_TEXTURE2D_DESC::default();
    unsafe { tex.GetDesc(&mut desc) };

    // (Re)create the staging texture when the surface size changes.
    let need_new = !matches!(&st.staging, Some((_, w, h)) if *w == desc.Width && *h == desc.Height);
    if need_new {
        let sd = D3D11_TEXTURE2D_DESC {
            Width: desc.Width,
            Height: desc.Height,
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
            Usage: D3D11_USAGE_STAGING,
            BindFlags: 0,
            CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
            MiscFlags: 0,
        };
        let mut staging = None;
        unsafe { st.device.CreateTexture2D(&sd, None, Some(&mut staging))? };
        st.staging = Some((staging.unwrap(), desc.Width, desc.Height));
    }
    let staging = st.staging.as_ref().unwrap().0.clone();
    let (out_w, out_h) = (st.out_w as usize, st.out_h as usize);
    let mut buf = slot.spare(out_w * out_h * 4);
    unsafe {
        st.context.CopyResource(&staging, &tex);
        let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
        st.context.Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))?;
        let (cx, cy) = (st.crop_x as usize, st.crop_y as usize);
        let avail_w = (content.Width.max(0) as usize).min(desc.Width as usize);
        let avail_h = (content.Height.max(0) as usize).min(desc.Height as usize);
        let copy_w = avail_w.saturating_sub(cx).min(out_w);
        let copy_h = avail_h.saturating_sub(cy).min(out_h);
        let src = mapped.pData as *const u8;
        let pitch = mapped.RowPitch as usize;
        for y in 0..copy_h {
            let row = std::slice::from_raw_parts(src.add((y + cy) * pitch + cx * 4), copy_w * 4);
            buf[y * out_w * 4..y * out_w * 4 + copy_w * 4].copy_from_slice(row);
        }
        st.context.Unmap(&staging, 0);
    }
    // Window resized: grow/shrink the pool so future frames are not cropped by
    // the pool size (output stays at the initial size).
    let resized = content.Width != st.pool_size.Width || content.Height != st.pool_size.Height;
    if resized && content.Width > 0 && content.Height > 0 {
        pool.Recreate(&st.winrt_device.0, DirectXPixelFormat::B8G8R8A8UIntNormalized, 2, content)?;
        st.pool_size = content;
    }
    drop(st);
    slot.publish(Frame { data: buf, time_100ns: time });
    Ok(())
}

/// Grab a single frame of `target`, downscaled to at most `max_width`, as PNG bytes.
pub fn thumbnail_png(target: CaptureTarget, max_width: u32) -> Result<Vec<u8>> {
    let slot = Arc::new(FrameSlot::default());
    let cap = Capture::start(target, slot.clone())?;
    let (w, h) = (cap.width, cap.height);
    let got = slot.wait_for_frame(Duration::from_millis(1500));
    let frame = slot.take_latest();
    cap.stop();
    let frame = match (got, frame) {
        (true, Some(f)) => f,
        _ => return Err(anyhow!("no frame received")),
    };
    // Integer box-filter downscale (BGRA → RGBA).
    let factor = w.div_ceil(max_width.max(1)).max(1);
    let (tw, th) = ((w / factor).max(1), (h / factor).max(1));
    let mut out = vec![0u8; (tw * th * 4) as usize];
    let n = factor * factor;
    for ty in 0..th {
        for tx in 0..tw {
            let mut acc = [0u32; 3];
            for dy in 0..factor {
                let row = ((ty * factor + dy) * w) as usize;
                for dx in 0..factor {
                    let i = (row + (tx * factor + dx) as usize) * 4;
                    acc[0] += frame.data[i + 2] as u32;
                    acc[1] += frame.data[i + 1] as u32;
                    acc[2] += frame.data[i] as u32;
                }
            }
            let o = ((ty * tw + tx) * 4) as usize;
            out[o..o + 4].copy_from_slice(&[(acc[0] / n) as u8, (acc[1] / n) as u8, (acc[2] / n) as u8, 255]);
        }
    }
    let mut png_bytes = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut png_bytes, tw, th);
        enc.set_color(png::ColorType::Rgba);
        enc.set_compression(png::Compression::Fast);
        enc.write_header()?.write_image_data(&out)?;
    }
    Ok(png_bytes)
}
