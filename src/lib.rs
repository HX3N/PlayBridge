// Fake `external_renderer_ipc.dll` — a nemu-ABI shim.
//
// MAA's MumuExtras path LoadLibrary's a vendor DLL and calls `nemu_*` in-process to grab the emulator framebuffer.
// GPG has no such DLL, so we provide one: it exports the nemu ABI but fetches frames from PlayBridge's WGC daemon.
//
// Deploy: rename the built DLL to
//   <fakemumu>/nx_device/15.0/shell/sdk/external_renderer_ipc.dll
// and point MAA's "MuMu emulator path" at <fakemumu>.
//
// Release uses `panic = "abort"`, so a panic here takes down MAA.
// Every export must be panic-free; turn all errors into nemu error codes (return > 0).

use std::io::{Read, Write};
use std::net::TcpStream;
use std::os::windows::process::CommandExt;
use std::sync::atomic::{AtomicI32, AtomicU32, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use winreg::enums::HKEY_CURRENT_USER;
use winreg::RegKey;

// Guards FreeLibrary from racing live captures.
static INFLIGHT: AtomicI32 = AtomicI32::new(0);

// Last daemon port that connected, so a capture doesn't reopen the registry every frame.
// 0 means unknown; a failed connect clears it, so a restarted daemon on a new port is picked up.
static CACHED_PORT: AtomicU32 = AtomicU32::new(0);

// Frame staging buffer, reused so a capture doesn't allocate and zero ~3.7MB per call.
static FRAME_BUF: Mutex<Vec<u8>> = Mutex::new(Vec::new());

// Must match src/config.rs (REG_PATH_STATE) and src/wgc.rs (DAEMON_PORT_KEY)
// and the EXE_PATH key written by src/main.rs.
const REG_STATE: &str = r"Software\PlayBridge\state";
const KEY_DAEMON_PORT: &str = "WGC_DAEMON_PORT";
const KEY_EXE_PATH: &str = "EXE_PATH";

// Must match src/config.rs DISPLAY_WIDTH / DISPLAY_HEIGHT (the daemon resizes to this,
// and MAA's reported `wm size` matches it so click coords line up).
const WIDTH: u32 = 1280;
const HEIGHT: u32 = 720;

// IPC timeouts. The daemon writes ~3.7MB over localhost; give the read room.
const CONNECT_WRITE_TIMEOUT_MS: u64 = 500;
const READ_TIMEOUT_MS: u64 = 3000;
const FRAME_RETRY_COUNT: usize = 3;
const FRAME_RETRY_DELAY_MS: u64 = 16;

// The DLL writes no logs (kept lightweight); the daemon logs each request.

fn reg_read_dword(key: &str) -> Option<u32> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    hkcu.open_subkey(REG_STATE).ok()?.get_value(key).ok()
}

fn reg_read_string(key: &str) -> Option<String> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    hkcu.open_subkey(REG_STATE).ok()?.get_value(key).ok()
}

/// Connect to the running WGC daemon, if any.
fn connect_daemon() -> Option<TcpStream> {
    let cached = CACHED_PORT.load(Ordering::Relaxed);
    if cached != 0 {
        if let Ok(stream) = TcpStream::connect(("127.0.0.1", cached as u16)) {
            return Some(stream);
        }
        CACHED_PORT.store(0, Ordering::Relaxed);
    }

    let port = reg_read_dword(KEY_DAEMON_PORT)?;
    if port == 0 {
        return None;
    }
    let stream = TcpStream::connect(("127.0.0.1", port as u16)).ok()?;
    CACHED_PORT.store(port, Ordering::Relaxed);
    Some(stream)
}

/// Spawn `PlayBridgeADB.exe --wgc-daemon` detached.
fn spawn_daemon(exe: &str) {
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let _ = std::process::Command::new(exe)
        .arg("--wgc-daemon")
        .creation_flags(DETACHED_PROCESS | CREATE_NO_WINDOW)
        .spawn();
}

/// Make sure the daemon is up and connectable, spawning it if needed.
fn ensure_daemon() {
    if connect_daemon().is_some() {
        return;
    }
    if let Some(exe) = reg_read_string(KEY_EXE_PATH) {
        spawn_daemon(&exe);
    }
}

/// Read the latest frame from the daemon into `buf` as raw RGBA (1280x720, top-down).
/// `buf` is resized rather than reallocated, so a steady-state capture allocates nothing.
fn request_frame(buf: &mut Vec<u8>) -> bool {
    let Some(mut stream) = connect_daemon() else {
        return false;
    };
    let _ = stream.set_write_timeout(Some(Duration::from_millis(CONNECT_WRITE_TIMEOUT_MS)));
    let _ = stream.set_read_timeout(Some(Duration::from_millis(READ_TIMEOUT_MS)));

    // port == 0 selects the daemon's byte-return verb (see wgc.rs handle_client).
    if stream.write_all(&0u16.to_le_bytes()).is_err() {
        return false;
    }

    let mut wh = [0u8; 8];
    if stream.read_exact(&mut wh).is_err() {
        return false;
    }
    let w = u32::from_le_bytes([wh[0], wh[1], wh[2], wh[3]]);
    let h = u32::from_le_bytes([wh[4], wh[5], wh[6], wh[7]]);
    let Some(len) = (w as usize).checked_mul(h as usize).and_then(|n| n.checked_mul(4)) else {
        return false;
    };
    if len == 0 || len > 64 * 1024 * 1024 {
        return false;
    }

    buf.resize(len, 0);
    stream.read_exact(buf).is_ok()
}

/// Request a frame, ensuring the daemon exists and tolerating connection failure.
fn request_frame_with_retry(buf: &mut Vec<u8>) -> bool {
    for attempt in 0..FRAME_RETRY_COUNT {
        if request_frame(buf) {
            return true;
        }
        if attempt == 0 {
            ensure_daemon();
        }
        std::thread::sleep(Duration::from_millis(FRAME_RETRY_DELAY_MS));
    }
    false
}

// nemu ABI exports. Return convention: 0 = success, > 0 = failure.

/// Ignores path/index (the daemon finds the GPG window).
/// Always returns a fixed non-zero handle; if the daemon can't start, capture fails and MAA falls back.
#[no_mangle]
pub extern "C" fn nemu_connect(_path: *const u16, _index: i32) -> i32 {
    ensure_daemon();
    1
}

#[no_mangle]
pub extern "C" fn nemu_disconnect(_handle: i32) {
    // Drain in-flight captures before returning so FreeLibrary doesn't race live code.
    while INFLIGHT.load(Ordering::Acquire) > 0 {
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[no_mangle]
pub extern "C" fn nemu_get_display_id(_handle: i32, _pkg: *const u8, _app_index: i32) -> i32 {
    0
}

/// Two-call protocol: buffer_size == 0 returns the dimensions; otherwise fills `pixels` with RGBA.
/// MAA applies `cvtColor(RGBA2BGR)` then `flip(.,0)`, so we write the frame BOTTOM-UP to cancel that vertical flip.
///
/// # Safety
/// `width` and `height` must be null or valid writable pointers.
/// `pixels` must be null or point to a buffer of at least `buffer_size` bytes.
#[no_mangle]
pub unsafe extern "C" fn nemu_capture_display(
    _handle: i32,
    _display_id: u32,
    buffer_size: i32,
    width: *mut i32,
    height: *mut i32,
    pixels: *mut u8,
) -> i32 {
    if !width.is_null() {
        *width = WIDTH as i32;
    }
    if !height.is_null() {
        *height = HEIGHT as i32;
    }

    if buffer_size == 0 {
        return 0;
    }

    let needed = (WIDTH * HEIGHT * 4) as usize;
    if pixels.is_null() || buffer_size < 0 || (buffer_size as usize) < needed {
        return 1;
    }

    INFLIGHT.fetch_add(1, Ordering::AcqRel);
    // A poisoned lock must not panic here, so fall back to a one-off buffer instead of unwrapping.
    match FRAME_BUF.lock() {
        Ok(mut buf) => {
            let ok = request_frame_with_retry(&mut buf);
            write_frame_bottom_up(ok.then_some(buf.as_slice()), pixels);
        }
        Err(_) => {
            let mut buf = Vec::new();
            let ok = request_frame_with_retry(&mut buf);
            write_frame_bottom_up(ok.then_some(buf.as_slice()), pixels);
        }
    }
    INFLIGHT.fetch_sub(1, Ordering::AcqRel);
    // Per-frame success is intentionally silent — the daemon logs the delivery.
    0
}

/// Copy `frame` into `pixels` bottom-up, zero-filling when there is no frame.
///
/// # Safety
/// `pixels` must point to a buffer of at least `WIDTH * HEIGHT * 4` bytes.
unsafe fn write_frame_bottom_up(frame: Option<&[u8]>, pixels: *mut u8) {
    let row = (WIDTH * 4) as usize;
    let h = HEIGHT as usize;
    let frame = frame.filter(|f| f.len() >= row * h);

    for y in 0..h {
        let src = (h - 1 - y) * row;
        let dst = y * row;
        match frame {
            Some(f) => std::ptr::copy_nonoverlapping(f.as_ptr().add(src), pixels.add(dst), row),
            None => std::ptr::write_bytes(pixels.add(dst), 0, row),
        }
    }
}

// Input exports: required symbols for DLL load, but input goes through the minitouch/Win32 path.
// MumuExtras is only used for screencap. Stubbed.
#[no_mangle]
pub extern "C" fn nemu_input_text(_handle: i32, _size: i32, _buf: *const u8) -> i32 {
    0
}

#[no_mangle]
pub extern "C" fn nemu_input_event_touch_down(_handle: i32, _display_id: i32, _x: i32, _y: i32) -> i32 {
    0
}

#[no_mangle]
pub extern "C" fn nemu_input_event_touch_up(_handle: i32, _display_id: i32) -> i32 {
    0
}

#[no_mangle]
pub extern "C" fn nemu_input_event_key_down(_handle: i32, _display_id: i32, _key_code: i32) -> i32 {
    0
}

#[no_mangle]
pub extern "C" fn nemu_input_event_key_up(_handle: i32, _display_id: i32, _key_code: i32) -> i32 {
    0
}
