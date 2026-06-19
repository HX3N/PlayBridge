// Windows Graphics Capture screencap: a persistent daemon plus thin per-call clients.
// The daemon holds a warm WGC session on the top-level GPG window, caches the latest frame, and crops/resizes on demand.
// WGC captures occluded windows continuously on the GPU, so a frame is always ready (~7ms) — no synchronous PrintWindow readback.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::os::windows::process::CommandExt;
use std::process::Command;
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

type FrameLatest = Arc<Mutex<Option<(Vec<u8>, u32, u32, Instant)>>>;

use windows::core::{IInspectable, Interface, PCWSTR};
use windows::Foundation::TypedEventHandler;
use windows::Graphics::Capture::{Direct3D11CaptureFrame, Direct3D11CaptureFramePool, GraphicsCaptureItem, GraphicsCaptureSession};
use windows::Graphics::DirectX::Direct3D11::IDirect3DSurface;
use windows::Graphics::DirectX::DirectXPixelFormat;
use windows::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS, HMODULE, HWND, POINT, RECT};
use windows::Win32::Graphics::Direct3D::{D3D_DRIVER_TYPE_HARDWARE, D3D_FEATURE_LEVEL};
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D, D3D11_CPU_ACCESS_READ, D3D11_CREATE_DEVICE_BGRA_SUPPORT,
    D3D11_MAPPED_SUBRESOURCE, D3D11_MAP_READ, D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC, D3D11_USAGE_STAGING,
};
use windows::Win32::Graphics::Dwm::{
    DwmGetWindowAttribute, DwmSetWindowAttribute, DWMWA_EXTENDED_FRAME_BOUNDS, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DEFAULT,
    DWMWCP_DONOTROUND, DWM_WINDOW_CORNER_PREFERENCE,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_SAMPLE_DESC;
use windows::Win32::Graphics::Dxgi::IDXGIDevice;
use windows::Win32::Graphics::Gdi::ClientToScreen;
use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::System::WinRT::Direct3D11::{CreateDirect3D11DeviceFromDXGIDevice, IDirect3DDxgiInterfaceAccess};
use windows::Win32::System::WinRT::Graphics::Capture::IGraphicsCaptureItemInterop;
use windows::Win32::UI::HiDpi::{SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2};
use windows::Win32::UI::WindowsAndMessaging::{GetAncestor, GetClientRect, IsWindow, GA_ROOT};

use crate::capture::{resize_to_display, transmit_pixels_nc};
use crate::config::{get_registry_dword, set_registry_dword, DISPLAY_HEIGHT, DISPLAY_WIDTH, REG_PATH_STATE};
use crate::logging::{debug_log, LogLevel, LogMode};
use crate::notification::{display_notification, Notification};
use crate::window::GameWindow;

const DAEMON_MUTEX: &str = "Local\\PlayBridgeWgcDaemon";
const DAEMON_PORT_KEY: &str = "WGC_DAEMON_PORT";
const DAEMON_IDLE_SECS: u64 = 120;
// How often the daemon normalizes the GPG window (restore/resize/rebind),
// independent of capture-request frequency.
const MAINTENANCE_INTERVAL: Duration = Duration::from_millis(200);
const IPC_CONNECT_TIMEOUT_MS: u64 = 500;
// Client must wait out the full frame delivery before the ack (matches `nc -w 3`).
const IPC_ACK_TIMEOUT_MS: u64 = 3000;

fn black_frame_rgba() -> Vec<u8> {
    vec![0u8; (DISPLAY_WIDTH * DISPLAY_HEIGHT * 4) as usize]
}

fn write_extras_frame(stream: &mut TcpStream, rgba: &[u8]) {
    let mut header = Vec::with_capacity(8);
    header.extend_from_slice(&DISPLAY_WIDTH.to_le_bytes());
    header.extend_from_slice(&DISPLAY_HEIGHT.to_le_bytes());
    let _ = stream.write_all(&header);
    let _ = stream.write_all(rgba);
}

fn create_device() -> windows::core::Result<(ID3D11Device, ID3D11DeviceContext)> {
    let mut device: Option<ID3D11Device> = None;
    let mut context: Option<ID3D11DeviceContext> = None;
    let mut level = D3D_FEATURE_LEVEL::default();

    unsafe {
        D3D11CreateDevice(
            None,
            D3D_DRIVER_TYPE_HARDWARE,
            HMODULE::default(),
            D3D11_CREATE_DEVICE_BGRA_SUPPORT,
            None,
            D3D11_SDK_VERSION,
            Some(&mut device),
            Some(&mut level),
            Some(&mut context),
        )?;
    }

    Ok((device.unwrap(), context.unwrap()))
}

fn to_winrt_device(d3d: &ID3D11Device) -> windows::core::Result<windows::Graphics::DirectX::Direct3D11::IDirect3DDevice> {
    let dxgi: IDXGIDevice = d3d.cast()?;
    let inspectable: IInspectable = unsafe { CreateDirect3D11DeviceFromDXGIDevice(&dxgi)? };
    inspectable.cast()
}

fn create_item(hwnd: HWND) -> windows::core::Result<GraphicsCaptureItem> {
    let interop: IGraphicsCaptureItemInterop = windows::core::factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>()?;
    unsafe { interop.CreateForWindow(hwnd) }
}

// Reused across FrameArrived callbacks: staging texture + output buffer.
// Avoids a driver allocation and an ~8MB heap alloc on every captured frame.
#[derive(Default)]
struct FrameWork {
    staging: Option<ID3D11Texture2D>,
    scratch: Vec<u8>,
}

fn create_staging(device: &ID3D11Device, desc: &D3D11_TEXTURE2D_DESC) -> windows::core::Result<ID3D11Texture2D> {
    let staging_desc = D3D11_TEXTURE2D_DESC {
        Width: desc.Width,
        Height: desc.Height,
        MipLevels: 1,
        ArraySize: 1,
        Format: desc.Format,
        SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
        Usage: D3D11_USAGE_STAGING,
        BindFlags: 0,
        CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
        MiscFlags: 0,
    };
    let mut staging: Option<ID3D11Texture2D> = None;
    unsafe { device.CreateTexture2D(&staging_desc, None, Some(&mut staging))? };
    Ok(staging.unwrap())
}

// Copy the captured frame into `out` (BGRA, top-down), reusing the staging texture and refilling `out` in place.
// Returns the frame's (width, height).
fn process_frame(
    device: &ID3D11Device,
    context: &ID3D11DeviceContext,
    staging_cache: &mut Option<ID3D11Texture2D>,
    out: &mut Vec<u8>,
    frame: &Direct3D11CaptureFrame,
) -> windows::core::Result<(u32, u32)> {
    let surface: IDirect3DSurface = frame.Surface()?;
    let access: IDirect3DDxgiInterfaceAccess = surface.cast()?;
    let texture: ID3D11Texture2D = unsafe { access.GetInterface()? };

    let mut desc = D3D11_TEXTURE2D_DESC::default();
    unsafe { texture.GetDesc(&mut desc) };

    // Reuse the staging texture unless size/format changed (constant per session).
    let recreate = match staging_cache {
        Some(s) => {
            let mut sd = D3D11_TEXTURE2D_DESC::default();
            unsafe { s.GetDesc(&mut sd) };
            sd.Width != desc.Width || sd.Height != desc.Height || sd.Format != desc.Format
        }
        None => true,
    };
    if recreate {
        *staging_cache = Some(create_staging(device, &desc)?);
    }
    let staging = staging_cache.as_ref().unwrap();

    unsafe { context.CopyResource(staging, &texture) };

    let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
    unsafe { context.Map(staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))? };

    let w = desc.Width as usize;
    let h = desc.Height as usize;
    let pitch = mapped.RowPitch as usize;
    let src = mapped.pData as *const u8;
    let needed = w * h * 4;

    // Refill without zero-init: rows below write all `needed` bytes, so set_len is sound.
    out.clear();
    out.reserve(needed);
    unsafe {
        let dst = out.as_mut_ptr();
        for y in 0..h {
            std::ptr::copy_nonoverlapping(src.add(y * pitch), dst.add(y * w * 4), w * 4);
        }
        out.set_len(needed);
    }

    unsafe { context.Unmap(staging, 0) };

    Ok((desc.Width, desc.Height))
}

// Crop geometry for the CROSVM child within the top-level's DWM frame bounds.
// Pure Win32 queries (no frame buffer), so it can run outside the frame lock.
fn crop_geometry(top: HWND, child: HWND) -> Option<(i32, i32, i32, i32)> {
    let mut bounds = RECT::default();
    unsafe {
        DwmGetWindowAttribute(
            top,
            DWMWA_EXTENDED_FRAME_BOUNDS,
            &mut bounds as *mut _ as *mut core::ffi::c_void,
            std::mem::size_of::<RECT>() as u32,
        )
        .ok()?;
    }

    let mut client = RECT::default();
    unsafe { GetClientRect(child, &mut client).ok()? };
    let cw = client.right - client.left;
    let ch = client.bottom - client.top;

    let mut tl = POINT { x: 0, y: 0 };
    unsafe {
        let _ = ClientToScreen(child, &mut tl);
    }

    let cx = (tl.x - bounds.left).max(0);
    let cy = (tl.y - bounds.top).max(0);
    Some((cx, cy, cw, ch))
}

// Fill strategy for the padded edge.
#[derive(Clone, Copy)]
#[allow(dead_code)]
enum EdgeFill {
    Black,
    Replicate,
}

// Resizing softens the edge regardless, so replicating the last opaque pixel is the cleanest fill.
const EDGE_FILL: EdgeFill = EdgeFill::Replicate;

// DWM skips the right/bottom ~3px, so that edge arrives transparent and unrecoverable.
// A buffer clamped to the captured size resizes at the wrong scale and pushes the UI bottom-right, so it stays full cw x ch.
// The alpha walk marks the opaque region and `fill` covers the transparent rest.
fn crop_region(frame: &[u8], fw: u32, fh: u32, rect: (i32, i32, i32, i32), fill: EdgeFill) -> Option<(Vec<u8>, u32, u32)> {
    let (cx, cy, cw, ch) = rect;
    if cw <= 0 || ch <= 0 || cx < 0 || cy < 0 {
        return None;
    }
    let (cw, ch) = (cw as u32, ch as u32);
    let avail_w = (fw as i32 - cx).clamp(0, cw as i32) as u32;
    let avail_h = (fh as i32 - cy).clamp(0, ch as i32) as u32;
    if avail_w == 0 || avail_h == 0 {
        return None;
    }

    let copy = (avail_w * 4) as usize;
    let mut out = vec![0u8; (cw * ch * 4) as usize];
    for y in 0..avail_h {
        let s = (((cy as u32 + y) * fw + cx as u32) * 4) as usize;
        let d = (y * cw * 4) as usize;
        out[d..d + copy].copy_from_slice(&frame[s..s + copy]);
    }

    // Find the opaque right/bottom extent by walking inward while alpha < 255.
    let alpha = |x: u32, y: u32| out[((y * cw + x) * 4 + 3) as usize];
    let mut ox = avail_w;
    while ox > 0 && alpha(ox - 1, avail_h / 2) < 255 {
        ox -= 1;
    }
    let mut oy = avail_h;
    while oy > 0 && alpha(avail_w / 2, oy - 1) < 255 {
        oy -= 1;
    }
    if ox == 0 || oy == 0 {
        return Some((out, cw, ch)); // degenerate; leave as copied
    }

    for y in 0..ch {
        for x in 0..cw {
            if x < ox && y < oy {
                continue; // keep opaque content
            }
            let d = ((y * cw + x) * 4) as usize;
            match fill {
                EdgeFill::Black => out[d..d + 4].copy_from_slice(&[0, 0, 0, 255]),
                EdgeFill::Replicate => {
                    let s = ((y.min(oy - 1) * cw + x.min(ox - 1)) * 4) as usize;
                    let p = [out[s], out[s + 1], out[s + 2], out[s + 3]];
                    out[d..d + 4].copy_from_slice(&p);
                }
            }
        }
    }
    Some((out, cw, ch))
}

// Win11 rounded corners can appear as transparent pixels in WGC near the crop edge.
// Square the window while capturing; square=false restores the default.
fn set_window_corners(top: HWND, square: bool) {
    let pref: DWM_WINDOW_CORNER_PREFERENCE = if square { DWMWCP_DONOTROUND } else { DWMWCP_DEFAULT };
    unsafe {
        let _ = DwmSetWindowAttribute(
            top,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            &pref as *const _ as *const core::ffi::c_void,
            std::mem::size_of::<DWM_WINDOW_CORNER_PREFERENCE>() as u32,
        );
    }
}

/// Warm WGC session bound to the top-level GPG window.
/// FrameArrived caches the latest full top-level BGRA frame; cropping/resize happen lazily per request.
pub struct WgcCapture {
    _device: ID3D11Device,
    _pool: Direct3D11CaptureFramePool,
    _session: GraphicsCaptureSession,
    latest: FrameLatest,
    top: HWND,
    child: HWND,
    /// Child client size at session creation.
    /// The frame pool is fixed to the window size, so a later resize (same hwnd) needs a rebind to match.
    bound_client: (i32, i32),
}

impl WgcCapture {
    pub fn new(child: HWND, top: HWND) -> windows::core::Result<Self> {
        let (device, context) = create_device()?;
        let winrt_device = to_winrt_device(&device)?;
        let item = create_item(top)?;
        let size = item.Size()?;
        let pool = Direct3D11CaptureFramePool::CreateFreeThreaded(&winrt_device, DirectXPixelFormat::B8G8R8A8UIntNormalized, 2, size)?;

        let latest: FrameLatest = Arc::new(Mutex::new(None));
        {
            let latest = latest.clone();
            let device = device.clone();
            let context = context.clone();
            let work = Arc::new(Mutex::new(FrameWork::default()));
            let handler = TypedEventHandler::<Direct3D11CaptureFramePool, IInspectable>::new(move |sender, _| {
                let sender = sender.ok()?;
                if let Ok(frame) = sender.TryGetNextFrame() {
                    let mut work = work.lock().unwrap();
                    let FrameWork { staging, scratch } = &mut *work;
                    if let Ok((w, h)) = process_frame(&device, &context, staging, scratch, &frame) {
                        // Swap the filled buffer into `latest`, reclaim the old one to reuse.
                        let filled = std::mem::take(scratch);
                        let prev = latest.lock().unwrap().replace((filled, w, h, Instant::now()));
                        *scratch = prev.map(|(buf, ..)| buf).unwrap_or_default();
                    }
                    let _ = frame.Close();
                }
                Ok(())
            });
            pool.FrameArrived(&handler)?;
        }

        let session = pool.CreateCaptureSession(&item)?;
        let _ = session.SetIsCursorCaptureEnabled(false);
        let _ = session.SetIsBorderRequired(false);
        session.StartCapture()?;

        // Square the window so the crop's bottom-right isn't eaten by the rounded-corner transparency.
        set_window_corners(top, true);

        let bound_client = child_client_size(child);
        Ok(Self { _device: device, _pool: pool, _session: session, latest, top, child, bound_client })
    }

    pub fn child(&self) -> HWND {
        self.child
    }

    pub fn bound_client(&self) -> (i32, i32) {
        self.bound_client
    }

    /// Latest frame cropped to the CROSVM client area, resized to display, RGBA.
    pub fn latest_display_rgba(&self) -> Option<(Vec<u8>, Duration, Duration)> {
        let t0 = Instant::now();

        // Geometry needs no frame buffer — compute before locking.
        let (cx, cy, cw0, ch0) = crop_geometry(self.top, self.child)?;

        // Hold the lock only to copy the crop region (not the whole frame).
        let (cropped, cw, ch, captured_at) = {
            let guard = self.latest.lock().unwrap();
            let (bgra, fw, fh, captured_at) = guard.as_ref()?;
            let (cropped, cw, ch) = crop_region(bgra, *fw, *fh, (cx, cy, cw0, ch0), EDGE_FILL)?;
            (cropped, cw, ch, *captured_at)
        };

        let mut rgba = resize_to_display(cropped, cw, ch)?;
        rgba.chunks_exact_mut(4).for_each(|c| c.swap(0, 2)); // BGRA -> RGBA
        Some((rgba, captured_at.elapsed(), t0.elapsed()))
    }
}

/// Physical client size of the CROSVM child (daemon is per-monitor DPI aware).
fn child_client_size(child: HWND) -> (i32, i32) {
    let mut rect = RECT::default();
    if unsafe { GetClientRect(child, &mut rect) }.is_ok() {
        (rect.right - rect.left, rect.bottom - rect.top)
    } else {
        (0, 0)
    }
}

fn daemon_already_running() -> bool {
    let name: Vec<u16> = DAEMON_MUTEX.encode_utf16().chain(Some(0)).collect();
    unsafe {
        let _ = CreateMutexW(None, false, PCWSTR(name.as_ptr()));
        GetLastError() == ERROR_ALREADY_EXISTS
    }
}

fn build_capture_once() -> Option<WgcCapture> {
    let w = GameWindow::find()?;
    w.normalize();
    let top = unsafe { GetAncestor(w.hwnd, GA_ROOT) };
    WgcCapture::new(w.hwnd, top).ok()
}

fn handle_client(mut stream: TcpStream, cap: Option<&WgcCapture>) {
    let _ = stream.set_read_timeout(Some(Duration::from_millis(IPC_CONNECT_TIMEOUT_MS)));
    let _ = stream.set_write_timeout(Some(Duration::from_millis(IPC_ACK_TIMEOUT_MS)));

    // No port bytes = a reachability probe (the nemu DLL's ensure_daemon opens and drops a socket).
    // The DLL logs nothing, so record the access here.
    let mut portbuf = [0u8; 2];
    if stream.read_exact(&mut portbuf).is_err() {
        debug_log(LogLevel::Info, LogMode::Event, "WgcDaemon: port access (probe, no request)");
        return;
    }
    let maa_port = u16::from_le_bytes(portbuf);

    // port == 0 is the byte-return verb (fake nemu DLL):
    // hand the raw RGBA frame back to the caller instead of streaming to MAA's nc port.
    // Response: [status: u8] then, if status==1, [w: u32 LE][h: u32 LE][rgba...].
    if maa_port == 0 {
        let t0 = Instant::now();
        match cap.and_then(|c| c.latest_display_rgba()) {
            Some((rgba, frame_age, crop_resize)) => {
                let t1 = Instant::now();
                write_extras_frame(&mut stream, &rgba);
                debug_log(
                    LogLevel::Info,
                    LogMode::Event,
                    &format!(
                        "WgcDaemon: extras delivered in {} ms (age {} ms, crop+resize {} ms, ipc_write {} ms)",
                        t0.elapsed().as_millis(),
                        frame_age.as_millis(),
                        crop_resize.as_millis(),
                        t1.elapsed().as_millis()
                    ),
                );
            }
            None => {
                let rgba = black_frame_rgba();
                write_extras_frame(&mut stream, &rgba);
                debug_log(LogLevel::Warn, LogMode::Event, "WgcDaemon: extras no frame cached, sent black frame");
            }
        }
        return;
    }

    let t0 = Instant::now();
    let delivered = match cap.and_then(|c| c.latest_display_rgba()) {
        Some((rgba, frame_age, crop_resize)) => {
            let t1 = Instant::now();
            transmit_pixels_nc(rgba, maa_port);
            debug_log(
                LogLevel::Info,
                LogMode::Nested,
                &format!(
                    "WgcDaemon: rawbync delivered in {} ms (age {} ms, crop+resize {} ms, transmit {} ms)",
                    t0.elapsed().as_millis(),
                    frame_age.as_millis(),
                    crop_resize.as_millis(),
                    t1.elapsed().as_millis()
                ),
            );
            1u8
        }
        None => {
            transmit_pixels_nc(black_frame_rgba(), maa_port);
            debug_log(LogLevel::Warn, LogMode::Event, "WgcDaemon: rawbync no frame cached, sent black frame");
            1u8
        }
    };
    let _ = stream.write_all(&[delivered]);
}

/// Long-lived daemon process (`--wgc-daemon`).
/// Single instance; serves the latest frame to thin clients and self-exits when idle or the window is gone.
pub fn run_daemon() {
    if daemon_already_running() {
        return;
    }

    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }

    let listener = match TcpListener::bind(("127.0.0.1", 0)) {
        Ok(l) => l,
        Err(e) => {
            debug_log(LogLevel::Error, LogMode::Event, &format!("WgcDaemon: bind failed: {}", e));
            return;
        }
    };
    let port = match listener.local_addr() {
        Ok(a) => a.port(),
        Err(_) => return,
    };
    let _ = set_registry_dword(DAEMON_PORT_KEY, port as u32, REG_PATH_STATE);
    display_notification(Notification::WgcDaemonStarted);
    debug_log(LogLevel::Info, LogMode::End, &format!("WgcDaemon: started / awaiting handshake (127.0.0.1:{})", port));

    let mut cap = build_capture_once();
    if cap.is_none() {
        debug_log(LogLevel::Warn, LogMode::Event, "WgcDaemon: window not found yet, serving black frames");
    }

    // Blocking accept on a dedicated thread feeding a channel: zero accept latency.
    let (tx, rx) = mpsc::channel::<TcpStream>();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            match stream {
                Ok(s) => {
                    if tx.send(s).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });

    let mut last_active = Instant::now();
    let mut last_maint = Instant::now();
    loop {
        match rx.recv_timeout(Duration::from_millis(200)) {
            Ok(stream) => {
                handle_client(stream, cap.as_ref());
                last_active = Instant::now();
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if last_active.elapsed() > Duration::from_secs(DAEMON_IDLE_SECS) {
                    break;
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }

        // Window maintenance — must live outside the recv branch
        // so it still runs while a busy MAA keeps recv_timeout returning Ok.
        if last_maint.elapsed() >= MAINTENANCE_INTERVAL {
            last_maint = Instant::now();

            // Skip the full GameWindow::find() while the bound window is alive;
            // reuse the cached handle (normalize/rebind checks still run below).
            let w = match cap.as_ref() {
                Some(c) if unsafe { IsWindow(Some(c.child())).as_bool() } => Some(GameWindow { hwnd: c.child() }),
                _ => GameWindow::find(),
            };

            match w {
                None => {
                    if cap.is_some() {
                        debug_log(LogLevel::Warn, LogMode::Event, "WgcDaemon: window gone, serving black frames");
                        cap = None;
                    }
                }
                Some(w) => {
                    w.normalize(); // restore from minimized; resize if too small/large/maximized
                    let needs_rebind = cap
                        .as_ref()
                        .map(|c| w.hwnd != c.child() || child_client_size(w.hwnd) != c.bound_client())
                        .unwrap_or(true);
                    if needs_rebind {
                        let top = unsafe { GetAncestor(w.hwnd, GA_ROOT) };
                        if let Ok(c) = WgcCapture::new(w.hwnd, top) {
                            debug_log(LogLevel::Info, LogMode::Event, "WgcDaemon: rebound (window changed)");
                            cap = Some(c);
                        }
                    }
                }
            }
        }
    }

    // Restore the window's default corner rounding we squared off while capturing.
    if let Some(c) = cap.as_ref() {
        if unsafe { IsWindow(Some(c.top)).as_bool() } {
            set_window_corners(c.top, false);
        }
    }

    let _ = set_registry_dword(DAEMON_PORT_KEY, 0, REG_PATH_STATE);
    display_notification(Notification::WgcDaemonStopped);
    debug_log(LogLevel::Info, LogMode::End, "WgcDaemon: exit by idle");
}

/// Spawn the daemon detached if it is not already running.
pub fn ensure_daemon() {
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    if let Ok(exe) = std::env::current_exe() {
        let _ = Command::new(exe).arg("--wgc-daemon").creation_flags(DETACHED_PROCESS | CREATE_NO_WINDOW).spawn();
    }
}

/// Ask the daemon to deliver the latest frame to MAA's nc `port`.
/// Returns true if the daemon confirmed delivery.
pub fn deliver_via_daemon(maa_port: u16) -> bool {
    let dport = get_registry_dword(DAEMON_PORT_KEY, REG_PATH_STATE).unwrap_or(0);
    if dport == 0 {
        return false;
    }

    let Ok(mut stream) = TcpStream::connect(("127.0.0.1", dport as u16)) else {
        return false;
    };
    let _ = stream.set_read_timeout(Some(Duration::from_millis(IPC_ACK_TIMEOUT_MS)));
    let _ = stream.set_write_timeout(Some(Duration::from_millis(IPC_CONNECT_TIMEOUT_MS)));

    if stream.write_all(&maa_port.to_le_bytes()).is_err() {
        return false;
    }

    let mut ack = [0u8; 1];
    if stream.read_exact(&mut ack).is_err() {
        return false;
    }
    ack[0] == 1
}
