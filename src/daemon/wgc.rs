// Windows Graphics Capture screencap: a persistent daemon plus thin per-call clients.
// The daemon holds a warm WGC session on the top-level GPG window, caches the latest frame, and crops/resizes on demand.
// WGC keeps compositing occluded windows on the GPU, so a frame is always ready (~7ms)
// and no request pays for a synchronous PrintWindow readback.

use std::cell::RefCell;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::os::windows::process::CommandExt;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

type FrameLatest = Arc<Mutex<Option<(Vec<u8>, u32, u32, Instant)>>>;

use windows::core::{IInspectable, Interface};
use windows::Foundation::TypedEventHandler;
use windows::Graphics::Capture::{Direct3D11CaptureFrame, Direct3D11CaptureFramePool, GraphicsCaptureItem, GraphicsCaptureSession};
use windows::Graphics::DirectX::Direct3D11::IDirect3DSurface;
use windows::Graphics::DirectX::DirectXPixelFormat;
use windows::Win32::Foundation::{HMODULE, HWND, POINT, RECT};
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
use windows::Win32::System::WinRT::Direct3D11::{CreateDirect3D11DeviceFromDXGIDevice, IDirect3DDxgiInterfaceAccess};
use windows::Win32::System::WinRT::Graphics::Capture::IGraphicsCaptureItemInterop;
use windows::Win32::UI::WindowsAndMessaging::{GetClientRect, IsWindow};

use crate::daemon::launcher::ensure_launcher;
use crate::daemon::window_state::{adopt_stale_park, spawn_minimize_watcher, ActivationWatch, ParkState};
use crate::game::capture::{black_frame_pixels, resize_to_display, transmit_pixels_nc};
use crate::game::window::{describe_window, top_level, GameWindow};
use crate::shared::{CREATE_NO_WINDOW, DETACHED_PROCESS, DISPLAY_HEIGHT, DISPLAY_WIDTH, KEY_DAEMON_PORT, REG_PATH_STATE};
use crate::sys::config::{config, get_registry, set_registry};
use crate::sys::logging::{debug_log, LogLevel, LogMode};
use crate::sys::notification::{display_notification, Notification};
use crate::sys::process::already_running;

pub const DAEMON_MUTEX: &str = "Local\\PlayBridgeWgcDaemon";
// MAA's lifetime ends the daemon; idling only drops capture to low power.
const LOW_POWER_AFTER: Duration = Duration::from_secs(15);
// Stays under FRAME_STALE so the first request after idling still gets a frame that counts as fresh.
const LOW_POWER_INTERVAL: Duration = Duration::from_millis(500);
const MAA_CHECK_INTERVAL: Duration = Duration::from_secs(1);
// How often the daemon normalizes the GPG window (restore/resize/rebind), independent of request rate.
const MAINTENANCE_INTERVAL: Duration = Duration::from_millis(50);
// Arknights animates continuously, so a frame older than this means the window stopped composing.
const FRAME_STALE: Duration = Duration::from_secs(1);
// Upper bound on reusing a stale frame, for when the window is gone but its HWND lingers past IsWindow.
const FRAME_REUSE_LIMIT: Duration = Duration::from_secs(30);
const IPC_CONNECT_TIMEOUT_MS: u64 = 500;
// Client must wait out the full frame delivery before the ack (matches `nc -w 3`).
const IPC_ACK_TIMEOUT_MS: u64 = 3000;

// Render resolution last checked, keyed by value so a mid-session change re-fires; daemon-lifetime memory only.
static RES_CHECK: Mutex<Option<(u32, u32)>> = Mutex::new(None);

// The frame callback fires from WGC and has no other link to request activity.
static LOW_POWER: AtomicBool = AtomicBool::new(false);

// Ratio gates: a non-16:9 render is distorted, so it stops there rather than also flagging the resolution.
fn check_render_resolution() {
    let package = config().client.package();
    if package.is_empty() {
        return;
    }
    let Some((w, h)) = crate::sys::store::render_resolution(package) else {
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
        debug_log(LogLevel::Warn, LogMode::Plain, &format!("Window: height ratio {:.1}, expected 9", height_ratio));
        display_notification(Notification::WindowWrongRatio(height_ratio));
        return;
    }

    if (w, h) != (DISPLAY_WIDTH, DISPLAY_HEIGHT) {
        debug_log(
            LogLevel::Warn,
            LogMode::Plain,
            &format!("Window: internal resolution {}x{}, expected {}x{}", w, h, DISPLAY_WIDTH, DISPLAY_HEIGHT),
        );
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

// Arknights animates nonstop, so frames keep arriving with nobody asking for them. Dropping them
// here is what makes idling cheap: the GPU->CPU copy is the whole cost of a frame.
fn skip_frame(last_processed: &Mutex<Instant>) -> bool {
    if !LOW_POWER.load(Ordering::Relaxed) {
        return false;
    }
    let mut last = last_processed.lock().unwrap();
    if last.elapsed() < LOW_POWER_INTERVAL {
        return true;
    }
    *last = Instant::now();
    false
}

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

    for y in 0..oy as usize {
        let row = y * stride;
        let s = row + (ox - 1) as usize * 4;
        let p = [out[s], out[s + 1], out[s + 2], out[s + 3]];
        for x in ox as usize..cw as usize {
            let d = row + x * 4;
            out[d..d + 4].copy_from_slice(&p);
        }
    }

    // Row oy-1 is only complete once the right strip above has run.
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
/// Cropping and resize happen lazily per request, not per captured frame.
pub struct WgcCapture {
    _device: ID3D11Device,
    _pool: Direct3D11CaptureFramePool,
    _session: GraphicsCaptureSession,
    latest: FrameLatest,
    top: HWND,
    child: HWND,
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
            let last_processed = Arc::new(Mutex::new(Instant::now()));
            let handler = TypedEventHandler::<Direct3D11CaptureFramePool, IInspectable>::new(move |sender, _| {
                let sender = sender.ok()?;
                if let Ok(frame) = sender.TryGetNextFrame() {
                    if !skip_frame(&last_processed) {
                        let mut work = work.lock().unwrap();
                        let FrameWork { staging, scratch } = &mut *work;
                        if let Ok((w, h)) = process_frame(&device, &context, staging, scratch, &frame) {
                            let filled = std::mem::take(scratch);
                            let prev = latest.lock().unwrap().replace((filled, w, h, Instant::now()));
                            *scratch = prev.map(|(buf, ..)| buf).unwrap_or_default();
                        }
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

        set_window_corners(top, true);

        let bound_client = GameWindow { hwnd: child }.get_client_size();
        Ok(Self { _device: device, _pool: pool, _session: session, latest, top, child, bound_client })
    }

    /// Grows once WGC stops delivering, which is the daemon's liveness signal for the window.
    pub fn latest_frame_age(&self) -> Option<Duration> {
        self.latest.lock().unwrap().as_ref().map(|(_, _, _, captured_at)| captured_at.elapsed())
    }

    /// Returns the RGBA frame, its age, and the crop+resize cost.
    pub fn latest_display_rgba(&self) -> Option<(Vec<u8>, Duration, Duration)> {
        let t0 = Instant::now();

        let (cx, cy, cw0, ch0) = crop_geometry(self.top, self.child)?;

        CROP_BUF.with(|buf| {
            let mut cropped = buf.borrow_mut();

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
        debug_log(LogLevel::Info, LogMode::Plain, "Wgc: port access (probe, no request)");
        return;
    }
    let maa_port = u16::from_le_bytes(portbuf);

    // Stale doesn't mean dead: a static overlay (the GPG user center webview AccountManager drives)
    // stops composing too, so the last frame is still the current screen while the window lives.
    let t0 = Instant::now();
    let frame = cap.and_then(|c| c.latest_display_rgba()).filter(|(_, age, _)| {
        *age < FRAME_STALE || (*age < FRAME_REUSE_LIMIT && cap.is_some_and(|c| unsafe { IsWindow(Some(c.child)).as_bool() }))
    });
    let reused = frame.as_ref().is_some_and(|(_, age, _)| *age >= FRAME_STALE);

    // port == 0 is the byte-return verb (fake nemu DLL): hand the RGBA frame to the caller instead of MAA's nc port.
    // Response: [w: u32 LE][h: u32 LE][rgba...]; no fresh frame degrades to a black frame in the same format.
    let extras = maa_port == 0;
    let (verb, cost) = if extras { ("extras", "ipc_write") } else { ("rawbync", "transmit") };

    let (rgba, timings) = match frame {
        Some((rgba, frame_age, crop_resize)) => (rgba, Some((frame_age, crop_resize))),
        None => (black_frame_pixels(), None),
    };

    let t1 = Instant::now();
    if extras {
        write_extras_frame(&mut stream, &rgba);
    } else {
        transmit_pixels_nc(rgba, maa_port);
    }

    match timings {
        Some((frame_age, crop_resize)) => debug_log(
            if reused { LogLevel::Warn } else { LogLevel::Info },
            LogMode::Plain,
            &format!(
                "Wgc: {} {} in {} ms (age {} ms, crop+resize {} ms, {} {} ms)",
                verb,
                if reused { "stale frame reused" } else { "delivered" },
                t0.elapsed().as_millis(),
                frame_age.as_millis(),
                crop_resize.as_millis(),
                cost,
                t1.elapsed().as_millis()
            ),
        ),
        None => {
            debug_log(LogLevel::Warn, LogMode::Plain, &format!("Wgc: {} no fresh frame, sent black frame", verb));
            // MAA asked for the screen and there is none, which is the one moment worth starting the game.
            ensure_launcher();
        }
    }

    // Only RawByNc's client waits on an ack; the byte-return verb already has its answer in the frame.
    if !extras {
        let _ = stream.write_all(&[1u8]);
    }
}
/// `building` guards against launching more than one worker build, and the old capture keeps serving until
/// the new one lands.
fn maintain(
    cap: &mut Option<WgcCapture>,
    park: &mut ParkState,
    activation: &mut ActivationWatch,
    build_tx: &mpsc::Sender<AssertSend<Option<WgcCapture>>>,
    building: &mut bool,
) {
    // IsWindow stays true for a hidden-but-alive GPG tree; fresh frames are the real liveness signal.
    let w = match cap.as_ref() {
        Some(c) if c.latest_frame_age().is_some_and(|age| age < FRAME_STALE) => Some(GameWindow { hwnd: c.child }),
        _ => GameWindow::find(),
    };

    match w {
        None => {
            activation.discard();
            // Starting the game belongs to the launcher, so this only waits for the window to return.
            if cap.is_some() {
                debug_log(LogLevel::Warn, LogMode::Plain, "Wgc: window gone, waiting");
                *cap = None;
            }
        }
        Some(w) => {
            let top = top_level(w.hwnd);
            park.update(top);
            park.arm_hook(top);
            activation.update(top);

            let needs_rebind = cap.as_ref().map(|c| w.hwnd != c.child || w.get_client_size() != c.bound_client).unwrap_or(true);
            if needs_rebind && !*building {
                *building = true;
                spawn_build(w.hwnd, top, build_tx.clone());
            }
        }
    }
}

/// Long-lived daemon process (`--wgc-daemon`).
/// Single instance; serves the latest frame to thin clients and lives as long as the MAA that spawned it.
pub fn run_daemon() {
    if already_running(DAEMON_MUTEX) {
        debug_log(LogLevel::Info, LogMode::Plain, "Wgc: already running");
        return;
    }

    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }

    let listener = match TcpListener::bind(("127.0.0.1", 0)) {
        Ok(l) => l,
        Err(e) => {
            debug_log(LogLevel::Error, LogMode::Plain, &format!("Wgc: bind failed: {}", e));
            return;
        }
    };
    let port = match listener.local_addr() {
        Ok(a) => a.port(),
        Err(_) => {
            debug_log(LogLevel::Error, LogMode::Plain, "Wgc: local addr unknown");
            return;
        }
    };
    let _ = set_registry(KEY_DAEMON_PORT, port as u32, REG_PATH_STATE);
    debug_log(LogLevel::Info, LogMode::Start, &format!("Wgc: started / awaiting handshake (127.0.0.1:{})", port));

    adopt_stale_park();
    spawn_minimize_watcher();

    let (build_tx, build_rx) = mpsc::channel::<AssertSend<Option<WgcCapture>>>();
    let mut cap: Option<WgcCapture> = None;
    let mut park = ParkState::new();
    let mut activation = ActivationWatch::new();
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
    let mut last_maa_check = Instant::now();
    let mut maa_gone = false;
    loop {
        // Clear the flag on either outcome so a failed build is retried on the next tick, not left stuck.
        if let Ok(AssertSend(built)) = build_rx.try_recv() {
            building = false;
            if let Some(c) = built {
                debug_log(LogLevel::Info, LogMode::Plain, &format!("Wgc: bound {}", describe_window(c.child, Some(c.top))));
                check_render_resolution();
                cap = Some(c);
            }
        }

        match rx.recv_timeout(MAINTENANCE_INTERVAL) {
            Ok(stream) => {
                if LOW_POWER.swap(false, Ordering::Relaxed) {
                    debug_log(LogLevel::Info, LogMode::Plain, "Wgc: request in, back to full rate");
                }
                handle_client(stream, cap.as_ref());
                last_active = Instant::now();
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if last_active.elapsed() > LOW_POWER_AFTER && !LOW_POWER.swap(true, Ordering::Relaxed) {
                    debug_log(LogLevel::Info, LogMode::Plain, "Wgc: idle, dropping to low power");
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }

        if last_maa_check.elapsed() >= MAA_CHECK_INTERVAL {
            last_maa_check = Instant::now();
            if !crate::sys::maa::is_alive() {
                maa_gone = true;
                break;
            }
        }

        // Runs outside the recv branch so maintenance still ticks while a busy MAA keeps recv_timeout returning Ok.
        if last_maint.elapsed() >= MAINTENANCE_INTERVAL {
            last_maint = Instant::now();
            maintain(&mut cap, &mut park, &mut activation, &build_tx, &mut building);
        }
    }

    match cap.as_ref().filter(|c| unsafe { IsWindow(Some(c.top)).as_bool() }) {
        Some(c) => {
            set_window_corners(c.top, false);
            park.restore_on_exit(c.top);
        }
        None => park.discard(),
    }

    activation.discard();

    let _ = set_registry(KEY_DAEMON_PORT, 0u32, REG_PATH_STATE);
    let farewell = if maa_gone { "Wgc: MAA gone, stopped" } else { "Wgc: stopped" };
    debug_log(LogLevel::Info, LogMode::End, farewell);
}

pub fn ensure_daemon() {
    if let Ok(exe) = std::env::current_exe() {
        let _ = Command::new(exe).arg("--wgc-daemon").creation_flags(DETACHED_PROCESS | CREATE_NO_WINDOW).spawn();
    }
}

/// Ask the daemon to deliver the latest frame to MAA's nc `port`.
pub fn deliver_via_daemon(maa_port: u16) -> bool {
    let dport = get_registry(KEY_DAEMON_PORT, 0u32, REG_PATH_STATE);
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
