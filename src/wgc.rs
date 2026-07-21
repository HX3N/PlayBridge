// Windows Graphics Capture screencap: a persistent daemon plus thin per-call clients.
// The daemon holds a warm WGC session on the top-level GPG window, caches the latest frame, and crops/resizes on demand.
// WGC captures occluded windows continuously on the GPU, so a frame is always ready (~7ms) — no synchronous PrintWindow readback.

use std::cell::RefCell;
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
use windows::Win32::UI::WindowsAndMessaging::{GetAncestor, GetClientRect, IsWindow, GA_ROOT};

use crate::capture::{black_frame_pixels, resize_to_display, transmit_pixels_nc};
use crate::config::{config, get_registry, set_registry, DISPLAY_HEIGHT, DISPLAY_WIDTH, REG_PATH_STATE};
use crate::logging::{debug_log, LogLevel, LogMode};
use crate::notification::{display_notification, Notification};
use crate::window::{describe_window, ensure_game_ready, GameWindow};

const DAEMON_MUTEX: &str = "Local\\PlayBridgeWgcDaemon";
const DAEMON_PORT_KEY: &str = "WGC_DAEMON_PORT";
const DAEMON_IDLE_SECS: u64 = 120;
// How often the daemon normalizes the GPG window (restore/resize/rebind), independent of request rate.
const MAINTENANCE_INTERVAL: Duration = Duration::from_millis(200);
// A live GPG window composes continuously (Arknights always animates), so a frame older than this means capture froze:
// the window was hidden/closed but its HWND lingers, so IsWindow/find can't see the loss. Serve black and re-verify.
const FRAME_STALE: Duration = Duration::from_secs(1);
// While no window is bound, retry the relaunch no more often than this — start_game_if_needed blocks ~1s per attempt.
const RELAUNCH_COOLDOWN: Duration = Duration::from_secs(10);
const IPC_CONNECT_TIMEOUT_MS: u64 = 500;
// Client must wait out the full frame delivery before the ack (matches `nc -w 3`).
const IPC_ACK_TIMEOUT_MS: u64 = 3000;

// Render resolution last checked, keyed by value so a mid-session change re-fires; daemon-lifetime memory only.
static RES_CHECK: Mutex<Option<(u32, u32)>> = Mutex::new(None);

// Aspect-ratio / resolution check against store.db, run once per value at (re)bind.
// Ratio gates: a non-16:9 render is distorted, so it stops there rather than also flagging the resolution.
fn check_render_resolution() {
    let package = config().client.package();
    if package.is_empty() {
        return;
    }
    let Some((w, h)) = crate::store::render_resolution(package) else {
        return;
    };

    {
        let mut last = RES_CHECK.lock().unwrap();
        if *last == Some((w, h)) {
            return;
        }
        *last = Some((w, h));
    }

    let height_ratio = h as f32 / (w as f32 / 16.0);
    if (height_ratio - 9.0).abs() > 0.1 {
        display_notification(Notification::WindowWrongRatio(height_ratio));
        return;
    }

    if (w, h) != (DISPLAY_WIDTH, DISPLAY_HEIGHT) {
        display_notification(Notification::InternalResolution { w, h });
    }
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

// Reuses the staging texture and refills `out` in place to avoid per-frame allocations.
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

// Crop destination, reused so a multi-megabyte buffer isn't allocated and zeroed per request.
// The daemon serves from one thread, matching capture.rs's RESIZER.
thread_local! {
    static CROP_BUF: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
}

// DWM skips the right/bottom ~3px, so that edge arrives transparent and unrecoverable.
// A buffer clamped to the captured size resizes at the wrong scale and pushes the UI bottom-right, so it stays full cw x ch.
// The alpha walk finds the opaque extent (ox, oy); the padded rest replicates the last opaque pixel,
// which resizing softens anyway.
//
// The padded area is two rectangles, so it is filled as two rectangles rather than by testing every pixel.
//
//   out = cw x ch (full, never clamped):
//   x=0          ox          cw
//   +------------+-----------+ y=0      opaque: x < ox && y < oy -> real content, kept
//   |  opaque    |   right   |          right strip:  each row repeats its own pixel at ox-1
//   |  ox x oy   |   strip   |          bottom strip: every row equals row oy-1, so copy it wholesale
// oy+------------+-----------+
//   |      bottom strip      |          ox,oy sit just inside the copied area (avail_w/avail_h);
//   +------------------------+ ch       the gap between them is DWM's transparent ~3px.
//
// `out` is caller-owned and reused across requests, so it is only resized, never re-zeroed.
// Every byte outside the copied area must be written before returning — the strip fills cover it,
// and the degenerate path zeroes it explicitly — or leftovers from the previous frame would leak.
fn crop_region(frame: &[u8], fw: u32, fh: u32, rect: (i32, i32, i32, i32), out: &mut Vec<u8>) -> Option<(u32, u32)> {
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
    let stride = (cw * 4) as usize;
    out.resize((cw * ch * 4) as usize, 0);
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
        // Degenerate (fully transparent): keep the copied area, but zero the padding the
        // strip fills below would have covered — resize() left previous-frame bytes there.
        for y in 0..avail_h as usize {
            out[y * stride + copy..(y + 1) * stride].fill(0);
        }
        out[avail_h as usize * stride..].fill(0);
        return Some((cw, ch));
    }

    // Right strip: rows above oy repeat their own last opaque pixel out to the edge.
    for y in 0..oy as usize {
        let row = y * stride;
        let s = row + (ox - 1) as usize * 4;
        let p = [out[s], out[s + 1], out[s + 2], out[s + 3]];
        for x in ox as usize..cw as usize {
            let d = row + x * 4;
            out[d..d + 4].copy_from_slice(&p);
        }
    }

    // Bottom strip: row oy-1 is complete by now, and replicating clamps every row below it to that row.
    let (filled, rest) = out.split_at_mut(oy as usize * stride);
    let last_row = &filled[(oy - 1) as usize * stride..];
    for row in rest.chunks_exact_mut(stride) {
        row.copy_from_slice(last_row);
    }

    Some((cw, ch))
}

// Win11 rounded corners surface as transparent pixels in WGC near the crop edge, so square the window while capturing.
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

        let bound_client = GameWindow { hwnd: child }.get_client_size();
        Ok(Self { _device: device, _pool: pool, _session: session, latest, top, child, bound_client })
    }

    pub fn child(&self) -> HWND {
        self.child
    }

    pub fn bound_client(&self) -> (i32, i32) {
        self.bound_client
    }

    /// Age of the cached frame, None if nothing captured yet. Grows once WGC stops delivering (window hidden/gone).
    pub fn latest_frame_age(&self) -> Option<Duration> {
        self.latest.lock().unwrap().as_ref().map(|(_, _, _, captured_at)| captured_at.elapsed())
    }

    /// Latest frame cropped to the CROSVM client area, resized to display, RGBA.
    pub fn latest_display_rgba(&self) -> Option<(Vec<u8>, Duration, Duration)> {
        let t0 = Instant::now();

        // Geometry needs no frame buffer — compute before locking.
        let (cx, cy, cw0, ch0) = crop_geometry(self.top, self.child)?;

        CROP_BUF.with(|buf| {
            let mut cropped = buf.borrow_mut();

            // Hold the lock only to copy the crop region (not the whole frame).
            let (cw, ch, captured_at) = {
                let guard = self.latest.lock().unwrap();
                let (bgra, fw, fh, captured_at) = guard.as_ref()?;
                let (cw, ch) = crop_region(bgra, *fw, *fh, (cx, cy, cw0, ch0), &mut cropped)?;
                (cw, ch, *captured_at)
            };

            let mut rgba = resize_to_display(&cropped, cw, ch)?;
            rgba.chunks_exact_mut(4).for_each(|c| c.swap(0, 2)); // BGRA -> RGBA
            Some((rgba, captured_at.elapsed(), t0.elapsed()))
        })
    }
}

fn daemon_already_running() -> bool {
    let name: Vec<u16> = DAEMON_MUTEX.encode_utf16().chain(Some(0)).collect();
    unsafe {
        let _ = CreateMutexW(None, false, PCWSTR(name.as_ptr()));
        GetLastError() == ERROR_ALREADY_EXISTS
    }
}

// HWNDs and the COM handles inside WgcCapture aren't Send, but the daemon is MTA and the frame pool is free-threaded,
// so building a session on a worker thread and handing it to the serve loop is sound — this wrapper carries it across.
struct AssertSend<T>(T);
unsafe impl<T> Send for AssertSend<T> {}

// Building a capture is the daemon's one slow step, so it runs off the serve thread rather than stalling delivery
// behind it; it reports back even on failure so the caller can clear its in-flight flag and retry.
fn spawn_build(child: HWND, top: HWND, tx: mpsc::Sender<AssertSend<Option<WgcCapture>>>) {
    let (child, top) = (child.0 as isize, top.0 as isize);
    std::thread::spawn(move || {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        }
        let cap = WgcCapture::new(HWND(child as *mut core::ffi::c_void), HWND(top as *mut core::ffi::c_void)).ok();
        let _ = tx.send(AssertSend(cap));
    });
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

    // Fetch the frame and start the timer once; both verbs reuse it.
    let t0 = Instant::now();
    let frame = cap.and_then(|c| c.latest_display_rgba()).filter(|(_, age, _)| *age < FRAME_STALE);

    // port == 0 is the byte-return verb (fake nemu DLL): hand the RGBA frame to the caller instead of MAA's nc port.
    // Response: [w: u32 LE][h: u32 LE][rgba...]; no fresh frame degrades to a black frame in the same format.
    if maa_port == 0 {
        match frame {
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
                write_extras_frame(&mut stream, &black_frame_pixels());
                debug_log(LogLevel::Warn, LogMode::Event, "WgcDaemon: extras no fresh frame, sent black frame");
            }
        }
        return;
    }

    let delivered = match frame {
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
            transmit_pixels_nc(black_frame_pixels(), maa_port);
            debug_log(LogLevel::Warn, LogMode::Event, "WgcDaemon: rawbync no fresh frame, sent black frame");
            1u8
        }
    };
    let _ = stream.write_all(&[delivered]);
}

/// One maintenance tick: re-verify the window and kick off a rebind/relaunch as needed; returns true when it's gone (exit).
/// The WgcCapture build runs on a worker (spawn_build); `building` guards against launching more than one at a time,
/// and the old capture keeps serving until the new one lands.
fn maintain(
    cap: &mut Option<WgcCapture>,
    last_relaunch: &mut Option<Instant>,
    build_tx: &mpsc::Sender<AssertSend<Option<WgcCapture>>>,
    building: &mut bool,
) -> bool {
    // IsWindow stays true for a hidden-but-alive GPG tree; fresh frames are the real liveness signal.
    // While they flow reuse the cached child; once they stall past FRAME_STALE, re-verify via window.rs's title search.
    let w = match cap.as_ref() {
        Some(c) if c.latest_frame_age().is_some_and(|age| age < FRAME_STALE) => Some(GameWindow { hwnd: c.child() }),
        _ => GameWindow::find(),
    };

    match w {
        None => {
            // find() miss with a bound capture means the window is really gone, so exit and let a fresh daemon rebind after relaunch.
            // cap == None is the startup/post-exit wait — relaunch instead of exiting.
            if cap.is_some() {
                debug_log(LogLevel::Warn, LogMode::Event, "WgcDaemon: window gone, exiting");
                return true;
            }
            // No window yet: relaunch the game, throttled.
            // This runs only while the window is down, so the ~1s launch wait never lands inside a benchmark capture.
            if last_relaunch.is_none_or(|t| t.elapsed() >= RELAUNCH_COOLDOWN) {
                *last_relaunch = Some(Instant::now());
                ensure_game_ready();
            }
        }
        Some(w) => {
            w.restore();
            let needs_rebind = cap.as_ref().map(|c| w.hwnd != c.child() || w.get_client_size() != c.bound_client()).unwrap_or(true);
            if needs_rebind && !*building {
                *building = true;
                let top = unsafe { GetAncestor(w.hwnd, GA_ROOT) };
                spawn_build(w.hwnd, top, build_tx.clone());
            }
        }
    }
    false
}

/// Long-lived daemon process (`--wgc-daemon`).
/// Single instance; serves the latest frame to thin clients and self-exits when idle or the window is gone.
pub fn run_daemon() {
    if daemon_already_running() {
        return;
    }

    unsafe {
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
    let _ = set_registry(DAEMON_PORT_KEY, port as u32, REG_PATH_STATE);
    debug_log(LogLevel::Info, LogMode::End, &format!("WgcDaemon: started / awaiting handshake (127.0.0.1:{})", port));

    // A WgcCapture build is the serve loop's only blocking step, so it runs on a worker and returns over this channel.
    let (build_tx, build_rx) = mpsc::channel::<AssertSend<Option<WgcCapture>>>();
    let mut cap: Option<WgcCapture> = None;
    let mut building = false;

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
    let mut last_relaunch: Option<Instant> = None;
    loop {
        // Clear the flag on either outcome so a failed build is retried on the next tick, not left stuck.
        if let Ok(AssertSend(built)) = build_rx.try_recv() {
            building = false;
            if let Some(c) = built {
                debug_log(LogLevel::Info, LogMode::Event, &format!("WgcDaemon: bound {}", describe_window(c.child(), Some(c.top))));
                check_render_resolution();
                cap = Some(c);
            }
        }

        match rx.recv_timeout(Duration::from_millis(200)) {
            Ok(stream) => {
                handle_client(stream, cap.as_ref());
                last_active = Instant::now();
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if last_active.elapsed() > Duration::from_secs(DAEMON_IDLE_SECS) {
                    debug_log(LogLevel::Info, LogMode::Event, "WgcDaemon: idle timeout, exiting");
                    break;
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }

        // Runs outside the recv branch so maintenance still ticks while a busy MAA keeps recv_timeout returning Ok.
        if last_maint.elapsed() >= MAINTENANCE_INTERVAL {
            last_maint = Instant::now();
            if maintain(&mut cap, &mut last_relaunch, &build_tx, &mut building) {
                break;
            }
        }
    }

    // Restore the window's default corner rounding we squared off while capturing.
    if let Some(c) = cap.as_ref() {
        if unsafe { IsWindow(Some(c.top)).as_bool() } {
            set_window_corners(c.top, false);
        }
    }

    let _ = set_registry(DAEMON_PORT_KEY, 0u32, REG_PATH_STATE);
    display_notification(Notification::WgcDaemonStopped);
    debug_log(LogLevel::Info, LogMode::End, "WgcDaemon: stopped");
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
    let dport = get_registry(DAEMON_PORT_KEY, 0u32, REG_PATH_STATE);
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
