//! The daemon owns the GPG window's state, not just its pixels: it hides a minimized window off-screen
//! so capture never stops, and clears an activation the window failed to release.

use std::sync::atomic::{AtomicI64, AtomicIsize, Ordering};
use std::time::{Duration, Instant};

use windows::core::PCWSTR;
use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::Graphics::Gdi::{MonitorFromWindow, MONITOR_DEFAULTTONULL};
use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
use windows::Win32::UI::Accessibility::{SetWinEventHook, HWINEVENTHOOK};
use windows::Win32::UI::Input::KeyboardAndMouse::SetActiveWindow;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DispatchMessageW, GetAncestor, GetForegroundWindow, GetGUIThreadInfo, GetMessageW, GetSystemMetrics,
    GetWindowPlacement, GetWindowRect, GetWindowThreadProcessId, IsIconic, SetWindowPlacement, SetWindowPos, ShowWindow, TranslateMessage,
    EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_MINIMIZESTART, GA_ROOT, GUITHREADINFO, MSG, SM_CYVIRTUALSCREEN, SM_YVIRTUALSCREEN,
    SWP_NOACTIVATE, SWP_NOSIZE, SWP_NOZORDER, SW_MINIMIZE, SW_SHOWMINNOACTIVE, SW_SHOWNOACTIVATE, WINDOWPLACEMENT, WINDOW_EX_STYLE,
    WINEVENT_OUTOFCONTEXT, WS_POPUP,
};

use crate::config::{get_registry, set_registry, REG_PATH_STATE};
use crate::logging::{debug_log, LogLevel, LogMode};
use crate::notification::{display_notification, Notification};
use crate::window::GameWindow;

// Written only while parked, so finding it at startup means the last daemon died parked.
const PARK_HOME_KEY: &str = "PARK_HOME";
// Survives daemon restarts, so a daemon that starts on an already-minimized window can park it
// straight away instead of showing it for a pass to read its origin.
const WINDOW_HOME_KEY: &str = "WINDOW_HOME";
const PARK_MARGIN: i32 = 32;
const RESTORE_WAIT: Duration = Duration::from_millis(50);

fn window_origin(hwnd: HWND) -> Option<(i32, i32)> {
    let mut rect = RECT::default();
    unsafe { GetWindowRect(hwnd, &mut rect) }.ok()?;
    Some((rect.left, rect.top))
}

fn park_y() -> i32 {
    let bottom = unsafe { GetSystemMetrics(SM_YVIRTUALSCREEN) + GetSystemMetrics(SM_CYVIRTUALSCREEN) };
    bottom + PARK_MARGIN
}

// SetWindowPlacement pulls an off-screen rcNormalPosition back onto a monitor, and onto the primary
// one at that, so parking can only move the window this way.
fn move_window(hwnd: HWND, x: i32, y: i32) -> bool {
    unsafe { SetWindowPos(hwnd, None, x, y, 0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE) }.is_ok()
}

// ShowWindow returns before the state change lands, and a minimized window ignores SetWindowPos
// while still reporting success.
fn wait_iconic(hwnd: HWND, target: bool) {
    let deadline = Instant::now() + RESTORE_WAIT;
    while unsafe { IsIconic(hwnd).as_bool() } != target && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(1));
    }
}

// Zero disarms the hook: parking without a known origin leaves nowhere to return to.
static HOOK_TARGET: AtomicIsize = AtomicIsize::new(0);

const PARK_RETURN_NONE: i64 = i64::MIN;
static PARK_RETURN: AtomicI64 = AtomicI64::new(PARK_RETURN_NONE);

fn pack_point(x: i32, y: i32) -> i64 {
    (((x as u32 as u64) << 32) | (y as u32 as u64)) as i64
}

fn unpack_point(v: i64) -> (i32, i32) {
    (((v as u64) >> 32) as u32 as i32, (v as u64) as u32 as i32)
}

// The window is still restored here, so this move sticks and becomes the restore rect.
// Once the minimize lands there is no way to move the window off-screen at all.
unsafe extern "system" fn on_minimize_start(_: HWINEVENTHOOK, _: u32, hwnd: HWND, _: i32, _: i32, _: u32, _: u32) {
    if hwnd.0 as isize != HOOK_TARGET.load(Ordering::Relaxed) {
        return;
    }
    if let Some((x, _)) = window_origin(hwnd) {
        move_window(hwnd, x, park_y());
    }
}

// The poll's unpark still owns the park state this callback cannot reach.
unsafe extern "system" fn on_foreground(_: HWINEVENTHOOK, _: u32, hwnd: HWND, _: i32, _: i32, _: u32, _: u32) {
    if hwnd.0 as isize != HOOK_TARGET.load(Ordering::Relaxed) {
        return;
    }
    let packed = PARK_RETURN.load(Ordering::Relaxed);
    if packed == PARK_RETURN_NONE {
        return;
    }
    let (x, y) = unpack_point(packed);
    move_window(hwnd, x, y);
}

pub fn spawn_minimize_watcher() {
    std::thread::spawn(|| {
        let hook = unsafe {
            SetWinEventHook(
                EVENT_SYSTEM_MINIMIZESTART,
                EVENT_SYSTEM_MINIMIZESTART,
                None,
                Some(on_minimize_start),
                0,
                0,
                WINEVENT_OUTOFCONTEXT,
            )
        };
        if hook.is_invalid() {
            debug_log(LogLevel::Warn, LogMode::Event, "Park: minimize hook not installed");
            return;
        }

        let foreground_hook = unsafe {
            SetWinEventHook(EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_FOREGROUND, None, Some(on_foreground), 0, 0, WINEVENT_OUTOFCONTEXT)
        };
        if foreground_hook.is_invalid() {
            debug_log(LogLevel::Warn, LogMode::Event, "Park: foreground hook not installed");
        }

        let mut msg = MSG::default();
        while unsafe { GetMessageW(&mut msg, None, 0, 0) }.as_bool() {
            unsafe {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
    });
}

fn off_all_monitors(hwnd: HWND) -> bool {
    unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTONULL) }.is_invalid()
}

// The origin is negative on any monitor left of the primary, so it cannot be a registry number.
fn store_origin(key: &str, x: i32, y: i32) {
    let _ = set_registry(key, format!("{},{}", x, y), REG_PATH_STATE);
}

fn clear_origin(key: &str) {
    let _ = set_registry(key, String::new(), REG_PATH_STATE);
}

fn read_origin(key: &str) -> Option<(i32, i32)> {
    let raw: String = get_registry(key, String::new(), REG_PATH_STATE);
    let (x, y) = raw.split_once(',')?;
    Some((x.parse().ok()?, y.parse().ok()?))
}

/// Hides a minimized game below the virtual desktop instead, since WGC stops delivering frames
/// once a window is really minimized. Y alone moves, so the taskbar button stays on its monitor.
pub struct ParkState {
    home: Option<(i32, i32)>,
    parked: bool,
    /// Carries the user's minimize intent across the pass that un-minimizes to read the origin.
    pending_park: bool,
}

impl ParkState {
    pub fn new() -> Self {
        Self { home: read_origin(WINDOW_HOME_KEY), parked: false, pending_park: false }
    }

    /// The minimize check must stay ahead of the parked check, or a window minimized while parked
    /// falls into the foreground branch and unparks itself.
    pub fn update(&mut self, top: HWND) {
        if unsafe { IsIconic(top).as_bool() } {
            match self.home {
                Some((x, y)) => self.park(top, x, y),
                None => {
                    // A minimized window reports an off-screen origin, so surface it to read a real one.
                    unsafe { _ = ShowWindow(top, SW_SHOWNOACTIVATE) };
                    self.pending_park = true;
                    debug_log(LogLevel::Info, LogMode::Event, "Park: learning home before parking");
                }
            }
            return;
        }

        if self.pending_park {
            if let Some(origin) = window_origin(top) {
                self.remember_home(origin);
                self.pending_park = false;
                self.park(top, origin.0, origin.1);
            }
            return;
        }

        if self.parked {
            if unsafe { GetForegroundWindow() } == top {
                self.unpark(top);
            } else if !off_all_monitors(top) {
                // Reissued because park()'s move is dropped if the restore had not landed yet.
                if let Some((x, _)) = self.home {
                    move_window(top, x, park_y());
                }
            }
            // home is deliberately not refreshed here: the parked origin would overwrite the real one.
            return;
        }

        // An off-monitor origin is a parked position; storing it as home would strand the window there.
        if !off_all_monitors(top) {
            if let Some(origin) = window_origin(top) {
                self.remember_home(origin);
            }
        }
    }

    // Writes only on a real move, so the poll does not hammer the registry.
    fn remember_home(&mut self, origin: (i32, i32)) {
        if self.home == Some(origin) {
            return;
        }
        self.home = Some(origin);
        store_origin(WINDOW_HOME_KEY, origin.0, origin.1);
    }

    pub fn arm_hook(&self, top: HWND) {
        HOOK_TARGET.store(if self.home.is_some() { top.0 as isize } else { 0 }, Ordering::Relaxed);
    }

    // The hook has already pushed the window off-screen by now, so this restore lands out of sight.
    // Hiding it first does not help: the SetWindowPos block comes from the minimized state, not from visibility.
    fn park(&mut self, top: HWND, x: i32, y: i32) {
        unsafe { _ = ShowWindow(top, SW_SHOWNOACTIVATE) };
        wait_iconic(top, false);

        let target_y = park_y();
        let moved = move_window(top, x, target_y);
        if !moved {
            debug_log(LogLevel::Warn, LogMode::Event, "Park: move off-screen failed");
            return;
        }

        PARK_RETURN.store(pack_point(x, y), Ordering::Relaxed);

        // A re-entry means the previous pass could not complete the move, so it stays quiet.
        if self.parked {
            return;
        }

        self.parked = true;
        store_origin(PARK_HOME_KEY, x, y);
        display_notification(Notification::WindowParked);
        debug_log(LogLevel::Info, LogMode::Event, &format!("Park: minimized, parked at ({},{}) home ({},{})", x, target_y, x, y));
    }

    fn unpark(&mut self, top: HWND) {
        let Some((x, y)) = self.home else {
            return;
        };

        move_window(top, x, y);
        self.parked = false;
        PARK_RETURN.store(PARK_RETURN_NONE, Ordering::Relaxed);
        clear_origin(PARK_HOME_KEY);
        debug_log(LogLevel::Info, LogMode::Event, &format!("Park: foreground, restored to ({},{})", x, y));
    }

    // Disarms the hooks first, or the exit's own minimize re-parks the window it is restoring.
    pub fn restore_on_exit(&mut self, top: HWND) {
        HOOK_TARGET.store(0, Ordering::Relaxed);
        PARK_RETURN.store(PARK_RETURN_NONE, Ordering::Relaxed);

        let Some((x, y)) = self.home.filter(|_| self.parked) else {
            clear_origin(PARK_HOME_KEY);
            return;
        };

        if !place_minimized_at(top, x, y) {
            move_window(top, x, y);
            unsafe { _ = ShowWindow(top, SW_MINIMIZE) };
        }
        self.parked = false;
        clear_origin(PARK_HOME_KEY);
        debug_log(LogLevel::Info, LogMode::Event, &format!("Park: daemon exit, restored to ({},{}) and re-minimized", x, y));
    }

    // With no window left to move back, a surviving record would only strand the next daemon.
    pub fn discard(&self) {
        if self.parked {
            debug_log(LogLevel::Warn, LogMode::Event, "Park: window gone, state discarded");
        }
        clear_origin(PARK_HOME_KEY);
    }
}

// Minimizing first keeps the animation off-screen, and the rect fix that follows paints nothing
// because the window is already iconic. Doing both in one call animates at the new spot instead.
// rcNormalPosition is not in screen coordinates, so the target is applied as a delta.
fn place_minimized_at(hwnd: HWND, x: i32, y: i32) -> bool {
    // An iconic window reports a sentinel rect, and a failure past the minimize leaves the caller's
    // fallback nothing to work with, so both reads happen first.
    let Some((cur_x, cur_y)) = window_origin(hwnd) else {
        return false;
    };
    let mut wp = WINDOWPLACEMENT { length: std::mem::size_of::<WINDOWPLACEMENT>() as u32, ..Default::default() };
    if unsafe { GetWindowPlacement(hwnd, &mut wp) }.is_err() {
        return false;
    }

    unsafe { _ = ShowWindow(hwnd, SW_MINIMIZE) };
    wait_iconic(hwnd, true);

    let (dx, dy) = (x - cur_x, y - cur_y);
    wp.rcNormalPosition.left += dx;
    wp.rcNormalPosition.right += dx;
    wp.rcNormalPosition.top += dy;
    wp.rcNormalPosition.bottom += dy;
    wp.showCmd = SW_SHOWMINNOACTIVE.0 as u32;
    unsafe { SetWindowPlacement(hwnd, &wp) }.is_ok()
}

/// A leftover record means the previous daemon died parked, leaving the window off-screen.
pub fn adopt_stale_park() {
    let Some((x, y)) = read_origin(PARK_HOME_KEY) else {
        return;
    };

    let Some(win) = GameWindow::find() else {
        debug_log(LogLevel::Warn, LogMode::Event, "Park: stale record found but window is gone");
        return;
    };

    let top = unsafe { GetAncestor(win.hwnd, GA_ROOT) };
    if !off_all_monitors(top) {
        clear_origin(PARK_HOME_KEY);
        debug_log(LogLevel::Warn, LogMode::Event, "Park: stale record dropped, window is already on screen");
        return;
    }

    move_window(top, x, y);
    clear_origin(PARK_HOME_KEY);
    debug_log(LogLevel::Warn, LogMode::Event, &format!("Park: adopted stale record, restored to ({},{})", x, y));
}

// A window still holding its queue's active window while the foreground sits elsewhere never receives
// another activation, so it never rebuilds the path that hands clicks to the guest.
const ACTIVATION_REPAIR_DELAY: Duration = Duration::from_millis(500);
const ACTIVATION_REPAIR_ATTEMPTS: u32 = 3;

fn activation_is_stale(top: HWND) -> bool {
    let thread = unsafe { GetWindowThreadProcessId(top, None) };
    if thread == 0 {
        return false;
    }

    let mut info = GUITHREADINFO { cbSize: std::mem::size_of::<GUITHREADINFO>() as u32, ..Default::default() };
    if unsafe { GetGUIThreadInfo(thread, &mut info) }.is_err() {
        return false;
    }

    let foreground = unsafe { GetForegroundWindow() };
    let owns_foreground = !foreground.0.is_null() && unsafe { GetAncestor(foreground, GA_ROOT) }.0 == top.0;

    !info.hwndActive.0.is_null() && !owns_foreground
}

// SetActiveWindow needs a window owned by the calling thread to move activation onto.
fn create_activation_helper() -> Option<HWND> {
    let class: Vec<u16> = "STATIC".encode_utf16().chain(Some(0)).collect();
    unsafe {
        CreateWindowExW(WINDOW_EX_STYLE::default(), PCWSTR(class.as_ptr()), PCWSTR::null(), WS_POPUP, 0, 0, 0, 0, None, None, None, None)
    }
    .ok()
}

// Sharing the queue makes the daemon a legal SetActiveWindow caller there, so the kernel delivers the
// deactivation itself. The foreground is never touched.
fn deactivate_via_queue(top: HWND, helper: HWND) -> bool {
    let gpg_thread = unsafe { GetWindowThreadProcessId(top, None) };
    if gpg_thread == 0 {
        return false;
    }
    let ours = unsafe { GetCurrentThreadId() };
    if !unsafe { AttachThreadInput(ours, gpg_thread, true) }.as_bool() {
        return false;
    }
    let moved = unsafe { SetActiveWindow(helper) }.is_ok();
    let _ = unsafe { AttachThreadInput(ours, gpg_thread, false) };
    moved
}

pub struct ActivationWatch {
    stale_since: Option<Instant>,
    repairs: u32,
    helper: Option<HWND>,
}

impl ActivationWatch {
    pub fn new() -> Self {
        Self { stale_since: None, repairs: 0, helper: None }
    }

    pub fn update(&mut self, top: HWND) {
        if !activation_is_stale(top) {
            if self.repairs > 0 {
                debug_log(LogLevel::Info, LogMode::Event, "Activation: cleared");
            }
            self.stale_since = None;
            self.repairs = 0;
            return;
        }

        // A handover to another window passes through this state, so only one that outlives it is repaired.
        let stale_since = *self.stale_since.get_or_insert_with(Instant::now);
        if stale_since.elapsed() < ACTIVATION_REPAIR_DELAY || self.repairs >= ACTIVATION_REPAIR_ATTEMPTS {
            return;
        }

        if self.helper.is_none() {
            self.helper = create_activation_helper();
        }
        let moved = self.helper.is_some_and(|helper| deactivate_via_queue(top, helper));

        // Restarted so the next attempt waits out the delay again instead of firing on the following tick.
        self.stale_since = Some(Instant::now());
        self.repairs += 1;
        debug_log(
            LogLevel::Warn,
            LogMode::Event,
            &format!(
                "Activation: stale active window, queue deactivation ({}/{}, moved {})",
                self.repairs, ACTIVATION_REPAIR_ATTEMPTS, moved
            ),
        );
    }
}
