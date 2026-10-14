// Records the GPG game window with two WGC sessions and a Desktop Duplication stream at once,
// and flags frames whose content reappears after it was gone.
// Usage: cargo run --release --example wgc_probe -- [total_secs=90] [b_delay_secs=20]

use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::ffi::c_void;
use std::fmt::Write as _;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use win_screenshot::prelude::*;
use windows::core::{w, IInspectable, Interface};
use windows::Foundation::TypedEventHandler;
use windows::Graphics::Capture::{Direct3D11CaptureFramePool, GraphicsCaptureItem, GraphicsCaptureSession};
use windows::Graphics::DirectX::Direct3D11::{IDirect3DDevice, IDirect3DSurface};
use windows::Graphics::DirectX::DirectXPixelFormat;
use windows::Win32::Foundation::{HMODULE, HWND, POINT, RECT};
use windows::Win32::Graphics::Direct3D::{D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_UNKNOWN, D3D_FEATURE_LEVEL};
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Multithread, ID3D11Texture2D, D3D11_CPU_ACCESS_READ,
    D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_MAPPED_SUBRESOURCE, D3D11_MAP_READ, D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC,
    D3D11_USAGE_STAGING,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_SAMPLE_DESC;
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory1, IDXGIAdapter, IDXGIAdapter1, IDXGIDevice, IDXGIFactory1, IDXGIOutput1, IDXGIOutputDuplication, IDXGIResource,
    DXGI_ERROR_ACCESS_LOST, DXGI_ERROR_WAIT_TIMEOUT, DXGI_OUTDUPL_FRAME_INFO,
};
use windows::Win32::Graphics::Gdi::ClientToScreen;
use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};
use windows::Win32::System::Performance::QueryPerformanceFrequency;
use windows::Win32::System::WinRT::Direct3D11::{CreateDirect3D11DeviceFromDXGIDevice, IDirect3DDxgiInterfaceAccess};
use windows::Win32::System::WinRT::Graphics::Capture::IGraphicsCaptureItemInterop;
use windows::Win32::UI::HiDpi::{SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2};
use windows::Win32::UI::WindowsAndMessaging::{FindWindowExW, GetClientRect};

const GAME_TITLES: [&str; 3] = ["명일방주", "アークナイツ", "Arknights"];
const REAPPEAR_GAP_MS: f64 = 200.0;
const PNG_PER_HASH: u32 = 3;
const PNG_PER_CAMERA: usize = 40;

fn sample_hash(bgra: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for px in bgra.as_chunks::<4>().0.iter().step_by(7) {
        h = (h ^ u32::from_ne_bytes(*px) as u64).wrapping_mul(0x100000001b3);
    }
    h
}

struct PngJob {
    name: String,
    w: u32,
    h: u32,
    bgra: Vec<u8>,
}

struct Rec {
    t_ms: f64,
    sys_ms: f64,
    hash: u64,
    w: u32,
    h: u32,
    tex: usize,
    event: bool,
}

#[derive(Default)]
struct HashInfo {
    first_t: f64,
    last_t: f64,
    reappear: u32,
    appearances: Vec<(f64, usize)>,
    first_png: Option<String>,
}

#[derive(Default)]
struct Tracker {
    cam: char,
    start_ms: Option<f64>,
    recs: Vec<Rec>,
    hashes: HashMap<u64, HashInfo>,
    texs: HashSet<usize>,
    prev_hash: Option<u64>,
    prev: Vec<u8>,
    prev_dims: (u32, u32),
    last_sys: Option<f64>,
    sys_stalled: usize,
    events: usize,
    pngs: usize,
    png_tx: Option<Sender<PngJob>>,
    access_lost: usize,
    error: Option<String>,
}

impl Tracker {
    fn new(cam: char, png_tx: Sender<PngJob>) -> Arc<Mutex<Self>> {
        Arc::new(Mutex::new(Self { cam, png_tx: Some(png_tx), ..Default::default() }))
    }

    fn record(&mut self, t_ms: f64, sys_ms: f64, tex: usize, w: u32, h: u32, frame: &mut Vec<u8>) {
        let hash = sample_hash(frame);
        let idx = self.recs.len();
        let mut event = false;

        if idx == 0 {
            if let Some(tx) = &self.png_tx {
                let _ = tx.send(PngJob { name: format!("{}_first.png", self.cam), w, h, bgra: frame.clone() });
            }
        }

        match self.hashes.get_mut(&hash) {
            Some(info) => {
                if self.prev_hash != Some(hash) && t_ms - info.last_t > REAPPEAR_GAP_MS {
                    event = true;
                    info.reappear += 1;
                    info.appearances.push((t_ms, idx));
                    if info.reappear <= PNG_PER_HASH && self.pngs + 2 <= PNG_PER_CAMERA {
                        if let Some(tx) = &self.png_tx {
                            let name = format!("{}_{:016x}_{}.png", self.cam, hash, info.reappear);
                            let prev_name = format!("{}_{:016x}_{}_prev.png", self.cam, hash, info.reappear);
                            let _ = tx.send(PngJob { name: name.clone(), w, h, bgra: frame.clone() });
                            let _ = tx.send(PngJob { name: prev_name, w: self.prev_dims.0, h: self.prev_dims.1, bgra: self.prev.clone() });
                            self.pngs += 2;
                            info.first_png.get_or_insert(name);
                        }
                    }
                }
                info.last_t = t_ms;
            }
            None => {
                self.hashes
                    .insert(hash, HashInfo { first_t: t_ms, last_t: t_ms, appearances: vec![(t_ms, idx)], ..Default::default() });
            }
        }

        if self.last_sys.is_some_and(|last| sys_ms <= last) {
            self.sys_stalled += 1;
        }
        self.last_sys = Some(sys_ms);
        if tex != 0 {
            self.texs.insert(tex);
        }
        self.events += event as usize;
        self.prev_hash = Some(hash);
        self.recs.push(Rec { t_ms, sys_ms, hash, w, h, tex, event });
        std::mem::swap(&mut self.prev, frame);
        self.prev_dims = (w, h);
    }
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

    let context = context.unwrap();
    let multithread: ID3D11Multithread = context.cast()?;
    unsafe {
        let _ = multithread.SetMultithreadProtected(true);
    }

    Ok((device.unwrap(), context))
}

fn to_winrt_device(d3d: &ID3D11Device) -> windows::core::Result<IDirect3DDevice> {
    let dxgi: IDXGIDevice = d3d.cast()?;
    let inspectable: IInspectable = unsafe { CreateDirect3D11DeviceFromDXGIDevice(&dxgi)? };
    inspectable.cast()
}

fn create_item(hwnd: HWND) -> windows::core::Result<GraphicsCaptureItem> {
    let interop: IGraphicsCaptureItemInterop = windows::core::factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>()?;
    unsafe { interop.CreateForWindow(hwnd) }
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

fn ensure_staging(
    device: &ID3D11Device,
    cache: &mut Option<ID3D11Texture2D>,
    desc: &D3D11_TEXTURE2D_DESC,
) -> windows::core::Result<ID3D11Texture2D> {
    let recreate = match cache {
        Some(s) => {
            let mut sd = D3D11_TEXTURE2D_DESC::default();
            unsafe { s.GetDesc(&mut sd) };
            sd.Width != desc.Width || sd.Height != desc.Height || sd.Format != desc.Format
        }
        None => true,
    };
    if recreate {
        *cache = Some(create_staging(device, desc)?);
    }
    Ok(cache.clone().unwrap())
}

fn read_texture(
    device: &ID3D11Device,
    context: &ID3D11DeviceContext,
    staging_cache: &mut Option<ID3D11Texture2D>,
    out: &mut Vec<u8>,
    texture: &ID3D11Texture2D,
    crop: Option<(usize, usize, usize, usize)>,
) -> windows::core::Result<(u32, u32)> {
    let mut desc = D3D11_TEXTURE2D_DESC::default();
    unsafe { texture.GetDesc(&mut desc) };
    let staging = ensure_staging(device, staging_cache, &desc)?;

    unsafe { context.CopyResource(&staging, texture) };

    let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
    unsafe { context.Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))? };

    let (tw, th) = (desc.Width as usize, desc.Height as usize);
    let (cx, cy, cw, ch) = match crop {
        Some((x, y, w, h)) => {
            let x = x.min(tw);
            let y = y.min(th);
            (x, y, w.min(tw - x), h.min(th - y))
        }
        None => (0, 0, tw, th),
    };
    let pitch = mapped.RowPitch as usize;
    let src = mapped.pData as *const u8;
    let needed = cw * ch * 4;

    out.clear();
    out.reserve(needed);
    unsafe {
        let dst = out.as_mut_ptr();
        for y in 0..ch {
            std::ptr::copy_nonoverlapping(src.add((cy + y) * pitch + cx * 4), dst.add(y * cw * 4), cw * 4);
        }
        out.set_len(needed);
    }

    unsafe { context.Unmap(&staging, 0) };

    Ok((cw as u32, ch as u32))
}

struct WgcCam {
    _device: ID3D11Device,
    pool: Direct3D11CaptureFramePool,
    session: GraphicsCaptureSession,
}

impl WgcCam {
    fn start(top: HWND, start: Instant, tracker: Arc<Mutex<Tracker>>) -> windows::core::Result<Self> {
        let (device, context) = create_device()?;
        let winrt_device = to_winrt_device(&device)?;
        let item = create_item(top)?;
        let size = item.Size()?;
        let pool = Direct3D11CaptureFramePool::CreateFreeThreaded(&winrt_device, DirectXPixelFormat::B8G8R8A8UIntNormalized, 2, size)?;

        {
            let device = device.clone();
            let tracker = tracker.clone();
            let work = Arc::new(Mutex::new((None::<ID3D11Texture2D>, Vec::<u8>::new())));
            let handler = TypedEventHandler::<Direct3D11CaptureFramePool, IInspectable>::new(move |sender, _| {
                let sender = sender.ok()?;
                if let Ok(frame) = sender.TryGetNextFrame() {
                    let t_ms = start.elapsed().as_secs_f64() * 1000.0;
                    let sys_ms = frame.SystemRelativeTime().map(|d| d.Duration as f64 / 10_000.0).unwrap_or(0.0);
                    let mut work = work.lock().unwrap();
                    let (staging, scratch) = &mut *work;
                    let read = (|| {
                        let surface: IDirect3DSurface = frame.Surface()?;
                        let access: IDirect3DDxgiInterfaceAccess = surface.cast()?;
                        let texture: ID3D11Texture2D = unsafe { access.GetInterface()? };
                        let (w, h) = read_texture(&device, &context, staging, scratch, &texture, None)?;
                        Ok::<_, windows::core::Error>((w, h, texture.as_raw() as usize))
                    })();
                    if let Ok((w, h, tex)) = read {
                        tracker.lock().unwrap().record(t_ms, sys_ms, tex, w, h, scratch);
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
        tracker.lock().unwrap().start_ms = Some(start.elapsed().as_secs_f64() * 1000.0);

        Ok(Self { _device: device, pool, session })
    }

    fn stop(self) {
        let _ = self.session.Close();
        let _ = self.pool.Close();
    }
}

struct Duplication {
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    dup: IDXGIOutputDuplication,
}

fn find_output(crop: RECT) -> Result<(IDXGIAdapter1, IDXGIOutput1, RECT), Box<dyn Error>> {
    let (cx, cy) = ((crop.left + crop.right) / 2, (crop.top + crop.bottom) / 2);
    let factory: IDXGIFactory1 = unsafe { CreateDXGIFactory1()? };

    let mut i = 0;
    while let Ok(adapter) = unsafe { factory.EnumAdapters1(i) } {
        i += 1;
        let mut j = 0;
        while let Ok(output) = unsafe { adapter.EnumOutputs(j) } {
            j += 1;
            let coords = unsafe { output.GetDesc()? }.DesktopCoordinates;
            if cx >= coords.left && cx < coords.right && cy >= coords.top && cy < coords.bottom {
                return Ok((adapter, output.cast()?, coords));
            }
        }
    }
    Err("no output contains the client rect center".into())
}

fn create_duplication(adapter: &IDXGIAdapter1, output: &IDXGIOutput1) -> windows::core::Result<Duplication> {
    let adapter: IDXGIAdapter = adapter.cast()?;
    let mut device: Option<ID3D11Device> = None;
    let mut context: Option<ID3D11DeviceContext> = None;
    unsafe {
        D3D11CreateDevice(
            Some(&adapter),
            D3D_DRIVER_TYPE_UNKNOWN,
            HMODULE::default(),
            D3D11_CREATE_DEVICE_BGRA_SUPPORT,
            None,
            D3D11_SDK_VERSION,
            Some(&mut device),
            None,
            Some(&mut context),
        )?;
    }
    let device = device.unwrap();
    let dup = unsafe { output.DuplicateOutput(&device)? };
    Ok(Duplication { device, context: context.unwrap(), dup })
}

fn run_duplication(crop: RECT, start: Instant, stop: &AtomicBool, tracker: &Mutex<Tracker>) {
    let result = (|| -> Result<(), Box<dyn Error>> {
        let mut freq = 0i64;
        unsafe { QueryPerformanceFrequency(&mut freq)? };

        let (adapter, output, coords) = find_output(crop)?;
        let mut d = create_duplication(&adapter, &output)?;
        let region = (
            (crop.left - coords.left) as usize,
            (crop.top - coords.top) as usize,
            (crop.right - crop.left) as usize,
            (crop.bottom - crop.top) as usize,
        );

        tracker.lock().unwrap().start_ms = Some(start.elapsed().as_secs_f64() * 1000.0);
        let mut staging = None;
        let mut scratch = Vec::new();
        while !stop.load(Ordering::Relaxed) {
            let mut info = DXGI_OUTDUPL_FRAME_INFO::default();
            let mut resource: Option<IDXGIResource> = None;
            match unsafe { d.dup.AcquireNextFrame(100, &mut info, &mut resource) } {
                Ok(()) => {}
                Err(e) if e.code() == DXGI_ERROR_WAIT_TIMEOUT => continue,
                Err(e) if e.code() == DXGI_ERROR_ACCESS_LOST => {
                    tracker.lock().unwrap().access_lost += 1;
                    while !stop.load(Ordering::Relaxed) {
                        std::thread::sleep(Duration::from_millis(100));
                        if let Ok(new) = create_duplication(&adapter, &output) {
                            d = new;
                            staging = None;
                            break;
                        }
                    }
                    continue;
                }
                Err(e) => return Err(e.into()),
            }
            let t_ms = start.elapsed().as_secs_f64() * 1000.0;

            if info.LastPresentTime != 0 {
                let texture: ID3D11Texture2D = resource.as_ref().unwrap().cast()?;
                let (w, h) = read_texture(&d.device, &d.context, &mut staging, &mut scratch, &texture, Some(region))?;
                let sys_ms = info.LastPresentTime as f64 * 1000.0 / freq as f64;
                tracker.lock().unwrap().record(t_ms, sys_ms, 0, w, h, &mut scratch);
            }
            drop(resource);
            unsafe { d.dup.ReleaseFrame()? };
        }
        Ok(())
    })();

    if let Err(e) = result {
        tracker.lock().unwrap().error = Some(e.to_string());
    }
}

fn find_game_window() -> Option<(HWND, HWND)> {
    window_list().ok()?.into_iter().find_map(|win| {
        if !GAME_TITLES.iter().any(|t| win.window_name.starts_with(t)) {
            return None;
        }
        let top = HWND(win.hwnd as usize as *mut c_void);
        let child = unsafe { FindWindowExW(Some(top), None, w!("CROSVM_1"), None) }.ok().filter(|h| !h.0.is_null())?;
        Some((top, child))
    })
}

fn client_rect_on_screen(child: HWND) -> Result<RECT, Box<dyn Error>> {
    let mut client = RECT::default();
    unsafe { GetClientRect(child, &mut client)? };
    let mut origin = POINT { x: 0, y: 0 };
    unsafe { ClientToScreen(child, &mut origin).ok()? };
    Ok(RECT { left: origin.x, top: origin.y, right: origin.x + client.right - client.left, bottom: origin.y + client.bottom - client.top })
}

fn median(mut v: Vec<f64>) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let n = v.len();
    if n % 2 == 1 {
        v[n / 2]
    } else {
        (v[n / 2 - 1] + v[n / 2]) / 2.0
    }
}

fn summarize(trackers: &[Tracker], total_ms: f64) -> String {
    let mut s = String::new();
    for t in trackers {
        let _ = writeln!(s, "=== Camera {} ===", t.cam);
        if let Some(e) = &t.error {
            let _ = writeln!(s, "error: {}", e);
        }
        let Some(start_ms) = t.start_ms else {
            let _ = writeln!(s, "never started\n");
            continue;
        };
        let secs = (total_ms - start_ms) / 1000.0;
        let frames = t.recs.len();
        let _ = writeln!(s, "start: {:.2} s since probe start", start_ms / 1000.0);
        let _ = writeln!(s, "frames: {}", frames);
        let _ = writeln!(s, "average fps: {:.2}", frames as f64 / secs);
        let _ = writeln!(s, "distinct hashes: {}", t.hashes.len());
        if t.cam != 'C' {
            let _ = writeln!(s, "distinct texture pointers: {}", t.texs.len());
        }
        let _ = writeln!(s, "frames whose sys time did not advance: {}", t.sys_stalled);
        let _ = writeln!(s, "reappearance events: {} ({:.4}/s)", t.events, t.events as f64 / secs);
        if t.cam == 'C' {
            let _ = writeln!(s, "ACCESS_LOST: {}", t.access_lost);
        }

        let mut top: Vec<(&u64, &HashInfo)> = t.hashes.iter().filter(|(_, i)| i.reappear > 0).collect();
        top.sort_by(|a, b| b.1.reappear.cmp(&a.1.reappear).then(a.1.first_t.partial_cmp(&b.1.first_t).unwrap()));
        let _ = writeln!(s, "\ntop reappearing hashes:");
        let _ = writeln!(
            s,
            "{:<16}  {:>10}  {:>10}  {:>6}  {:>10}  {:>10}  first png",
            "hash", "first(cam)", "first(all)", "count", "med ms", "med frames"
        );
        for (hash, info) in top.into_iter().take(20) {
            let ms = median(info.appearances.windows(2).map(|w| w[1].0 - w[0].0).collect());
            let fr = median(info.appearances.windows(2).map(|w| (w[1].1 - w[0].1) as f64).collect());
            let _ = writeln!(
                s,
                "{:016x}  {:>9.2}s  {:>9.2}s  {:>6}  {:>10.1}  {:>10.1}  {}",
                hash,
                (info.first_t - start_ms) / 1000.0,
                info.first_t / 1000.0,
                info.reappear,
                ms,
                fr,
                info.first_png.as_deref().unwrap_or("-")
            );
        }
        s.push('\n');
    }
    s
}

fn write_csv(path: &PathBuf, trackers: &[Tracker]) -> std::io::Result<()> {
    let mut s = String::from("cam,t_ms,sys_ms,hash,w,h,tex,event\n");
    for t in trackers {
        for r in &t.recs {
            let _ = writeln!(
                s,
                "{},{:.3},{:.3},{:016x},{},{},{:#x},{}",
                t.cam,
                r.t_ms,
                r.sys_ms,
                r.hash,
                r.w,
                r.h,
                r.tex,
                if r.event { "reappear" } else { "" }
            );
        }
    }
    std::fs::write(path, s)
}

fn progress(trackers: &[Arc<Mutex<Tracker>>], secs: u64) {
    let mut line = format!("[{:>4}s]", secs);
    for t in trackers {
        let t = t.lock().unwrap();
        let _ = write!(line, " {}: frames={} events={} |", t.cam, t.recs.len(), t.events);
    }
    println!("{}", line.trim_end_matches(" |"));
}

fn main() -> Result<(), Box<dyn Error>> {
    unsafe { _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) }
    unsafe { _ = CoInitializeEx(None, COINIT_MULTITHREADED) };

    let mut args = std::env::args().skip(1);
    let total_secs: u64 = args.next().map(|a| a.parse()).transpose()?.unwrap_or(90);
    let b_delay_secs: u64 = args.next().map(|a| a.parse()).transpose()?.unwrap_or(20);

    let Some((top, child)) = find_game_window() else {
        eprintln!("No Arknights window with a CROSVM_1 child was found. Start the game in Google Play Games first.");
        std::process::exit(1);
    };
    let crop = client_rect_on_screen(child)?;

    let out_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("wgc_probe")
        .join(chrono::Local::now().format("%Y%m%d-%H%M%S").to_string());
    std::fs::create_dir_all(&out_dir)?;
    println!("Output: {}", out_dir.display());

    let (png_tx, png_rx) = mpsc::channel::<PngJob>();
    let writer_dir = out_dir.clone();
    let writer = std::thread::spawn(move || {
        for job in png_rx {
            let mut rgba = job.bgra;
            rgba.as_chunks_mut::<4>().0.iter_mut().for_each(|c| c.swap(0, 2));
            if let Err(e) = image::save_buffer(writer_dir.join(&job.name), &rgba, job.w, job.h, image::ColorType::Rgba8) {
                eprintln!("PNG {} failed: {}", job.name, e);
            }
        }
    });

    let trackers = [Tracker::new('A', png_tx.clone()), Tracker::new('B', png_tx.clone()), Tracker::new('C', png_tx.clone())];
    drop(png_tx);

    let start = Instant::now();
    let stop = Arc::new(AtomicBool::new(false));
    let c_thread = {
        let (tracker, stop) = (trackers[2].clone(), stop.clone());
        std::thread::spawn(move || {
            unsafe { _ = CoInitializeEx(None, COINIT_MULTITHREADED) };
            run_duplication(crop, start, &stop, &tracker);
        })
    };

    let cam_a = WgcCam::start(top, start, trackers[0].clone())?;
    let mut cam_b: Option<WgcCam> = None;
    let mut b_started = false;
    let mut next_progress = 10;
    loop {
        let secs = start.elapsed().as_secs();
        if secs >= total_secs {
            break;
        }
        if !b_started && secs >= b_delay_secs {
            b_started = true;
            match WgcCam::start(top, start, trackers[1].clone()) {
                Ok(cam) => cam_b = Some(cam),
                Err(e) => trackers[1].lock().unwrap().error = Some(e.to_string()),
            }
        }
        if secs >= next_progress {
            progress(&trackers, secs);
            next_progress += 10;
        }
        std::thread::sleep(Duration::from_millis(100));
    }

    let total_ms = start.elapsed().as_secs_f64() * 1000.0;
    cam_a.stop();
    if let Some(cam) = cam_b {
        cam.stop();
    }
    stop.store(true, Ordering::Relaxed);
    let _ = c_thread.join();

    let done: Vec<Tracker> = trackers
        .iter()
        .map(|t| {
            let mut guard = t.lock().unwrap();
            let cam = guard.cam;
            let mut t = std::mem::replace(&mut *guard, Tracker { cam, ..Default::default() });
            t.png_tx = None;
            t
        })
        .collect();
    drop(trackers);
    let _ = writer.join();

    write_csv(&out_dir.join("frames.csv"), &done)?;
    let summary = summarize(&done, total_ms);
    std::fs::write(out_dir.join("summary.txt"), &summary)?;
    println!("\n{}", summary);
    println!("Output: {}", out_dir.display());
    Ok(())
}
