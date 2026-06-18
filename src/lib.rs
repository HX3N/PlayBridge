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
use std::time::Duration;

use winreg::enums::HKEY_CURRENT_USER;
use winreg::RegKey;

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
    let port = reg_read_dword(KEY_DAEMON_PORT)?;
    if port == 0 {
        return None;
    }
    TcpStream::connect(("127.0.0.1", port as u16)).ok()
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

/// Ask the daemon for the latest frame as raw RGBA (1280x720, top-down).
fn request_frame() -> Option<Vec<u8>> {
    let mut stream = connect_daemon()?;
    let _ = stream.set_write_timeout(Some(Duration::from_millis(CONNECT_WRITE_TIMEOUT_MS)));
    let _ = stream.set_read_timeout(Some(Duration::from_millis(READ_TIMEOUT_MS)));

    // port == 0 selects the daemon's byte-return verb (see wgc.rs handle_client).
    stream.write_all(&0u16.to_le_bytes()).ok()?;

    let mut status = [0u8; 1];
    stream.read_exact(&mut status).ok()?;
    if status[0] != 1 {
        return None; // no frame cached yet (cold start)
    }

    let mut wh = [0u8; 8];
    stream.read_exact(&mut wh).ok()?;
    let w = u32::from_le_bytes(wh[0..4].try_into().ok()?);
    let h = u32::from_le_bytes(wh[4..8].try_into().ok()?);
    let len = (w as usize).checked_mul(h as usize)?.checked_mul(4)?;
    if len == 0 || len > 64 * 1024 * 1024 {
        return None;
    }

    let mut buf = vec![0u8; len];
    stream.read_exact(&mut buf).ok()?;
    Some(buf)
}

/// Request a frame, ensuring the daemon exists and tolerating cold start.
fn request_frame_with_retry() -> Option<Vec<u8>> {
    for attempt in 0..FRAME_RETRY_COUNT {
        if let Some(frame) = request_frame() {
            return Some(frame);
        }
        if attempt == 0 {
            ensure_daemon();
        }
        std::thread::sleep(Duration::from_millis(FRAME_RETRY_DELAY_MS));
    }
    None
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
    // The daemon manages its own lifecycle (self-exits after idle); nothing to do.
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
        unsafe { *width = WIDTH as i32 };
    }
    if !height.is_null() {
        unsafe { *height = HEIGHT as i32 };
    }

    // Size query.
    if buffer_size == 0 {
        return 0;
    }

    let needed = (WIDTH * HEIGHT * 4) as usize;
    if pixels.is_null() || buffer_size < 0 || (buffer_size as usize) < needed {
        return 1;
    }

    let frame = request_frame_with_retry();
    let frame = frame.as_deref();

    let row = (WIDTH * 4) as usize;
    let h = HEIGHT as usize;
    unsafe {
        for y in 0..h {
            let src = (h - 1 - y) * row; // bottom-up
            let dst = y * row;
            if let Some(frame) = frame.filter(|f| f.len() >= needed) {
                std::ptr::copy_nonoverlapping(frame.as_ptr().add(src), pixels.add(dst), row);
            } else {
                std::ptr::write_bytes(pixels.add(dst), 0, row);
            }
        }
    }
    // Per-frame success is intentionally silent — the daemon logs the delivery.
    0
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
