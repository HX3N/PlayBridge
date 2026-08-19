use std::{
    ffi::c_void,
    thread,
    time::{Duration, Instant},
};

use win_screenshot::prelude::*;
use windows::core::PCWSTR;
use windows::Win32::{
    Foundation::{HWND, RECT},
    UI::WindowsAndMessaging::{
        FindWindowExW, GetClassNameW, GetClientRect, GetGUIThreadInfo, GetParent, GetWindowThreadProcessId, IsIconic, IsWindow, ShowWindow,
        GUITHREADINFO, GUI_INMENUMODE, GUI_INMOVESIZE, GUI_POPUPMENUMODE, GUI_SYSTEMMENUMODE, SW_RESTORE,
    },
};

use crate::sys::config::{config, set_client, Client};
use crate::sys::logging::{debug_log, LogLevel, LogMode};
use crate::sys::notification::{display_notification, Notification};

const CROSVM_CLASS: &str = "CROSVM_1";

const WINDOW_RESTORE_DELAY_MS: u64 = 300;

const HOLD_POLL: Duration = Duration::from_millis(5);
const HOLD_WARN_DELAY: Duration = Duration::from_secs(3);

const MODAL_FLAGS: u32 = GUI_INMOVESIZE.0 | GUI_INMENUMODE.0 | GUI_SYSTEMMENUMODE.0 | GUI_POPUPMENUMODE.0;

pub fn parent_or_self(hwnd: HWND) -> HWND {
    unsafe { GetParent(hwnd).ok().filter(|p| !p.0.is_null()).unwrap_or(hwnd) }
}

pub struct GameWindow {
    pub hwnd: HWND,
}

impl GameWindow {
    pub fn find() -> Option<Self> {
        let (hwnd, client) = window_list().ok()?.into_iter().find_map(|win| {
            let client = resolve_client_by_title(&win.window_name)?;
            let child = find_crosvm_child(HWND(win.hwnd as usize as *mut c_void))?;
            Some((child, client))
        })?;

        adopt_running_client(client);
        Some(Self { hwnd })
    }

    pub fn restore(&self) {
        let target_hwnd = parent_or_self(self.hwnd);

        if unsafe { IsIconic(target_hwnd).as_bool() } {
            unsafe { _ = ShowWindow(target_hwnd, SW_RESTORE) };
            thread::sleep(Duration::from_millis(WINDOW_RESTORE_DELAY_MS));
        }
    }

    pub fn get_client_size(&self) -> (i32, i32) {
        let mut rect = RECT::default();
        if unsafe { GetClientRect(self.hwnd, &mut rect) }.is_ok() {
            (rect.right - rect.left, rect.bottom - rect.top)
        } else {
            (0, 0)
        }
    }
}

// The frame's move/size loop drains mouse messages from the whole UI thread queue, so the ones
// posted to the child are swallowed with them. Menu tracking holds the queue the same way.
fn in_modal_loop(top: HWND) -> bool {
    // An unreadable state counts as idle: a gate stuck closed would block MAA for good.
    let thread = unsafe { GetWindowThreadProcessId(top, None) };
    if thread == 0 {
        return false;
    }

    let mut info = GUITHREADINFO { cbSize: std::mem::size_of::<GUITHREADINFO>() as u32, ..Default::default() };
    if unsafe { GetGUIThreadInfo(thread, &mut info) }.is_err() {
        return false;
    }

    info.flags.0 & MODAL_FLAGS != 0
}

pub fn await_modal_end(top: HWND) -> Option<Duration> {
    if !in_modal_loop(top) {
        return Some(Duration::ZERO);
    }

    let started = Instant::now();
    loop {
        if !unsafe { IsWindow(Some(top)).as_bool() } {
            debug_log(LogLevel::Warn, LogMode::Event, "Minitouch: window gone while held");
            return None;
        }

        if !in_modal_loop(top) {
            let held = started.elapsed();
            debug_log(LogLevel::Info, LogMode::Event, &format!("Minitouch: held for user window action, {} ms", held.as_millis()));
            return Some(held);
        }

        // Asked for on every pass by design; the tag's registry cooldown is what spaces the toasts out.
        if started.elapsed() >= HOLD_WARN_DELAY {
            display_notification(Notification::InputHeld);
        }
        thread::sleep(HOLD_POLL);
    }
}

/// Notification language is picked by the stored client, so the update lands before the toast.
fn adopt_running_client(client: Client) {
    let previous = config().client;
    if previous == client {
        return;
    }
    set_client(client);

    if previous != Client::Empty {
        display_notification(Notification::ClientMismatch(previous.package().to_string(), client.package().to_string()));
    }
}

fn resolve_client_by_title(title: &str) -> Option<Client> {
    [Client::KR, Client::JP, Client::EN].into_iter().find(|&client| title.starts_with(client.title()))
}

fn find_crosvm_child(parent_hwnd: HWND) -> Option<HWND> {
    let crosvm_class_wide: Vec<u16> = CROSVM_CLASS.encode_utf16().chain(Some(0)).collect();

    unsafe {
        FindWindowExW(Some(parent_hwnd), None, PCWSTR(crosvm_class_wide.as_ptr()), None)
            .ok()
            .filter(|h| !h.0.is_null())
    }
}

pub(crate) fn get_window_class(hwnd: HWND) -> Option<String> {
    let mut buffer: [u16; 256] = [0; 256];
    unsafe {
        let len = GetClassNameW(hwnd, &mut buffer);
        if len > 0 {
            Some(String::from_utf16_lossy(&buffer[..len as usize]))
        } else {
            None
        }
    }
}

/// Bound-window log summary. client is the child's physical size the WGC frame pool binds to, so a change forces a rebind.
pub(crate) fn describe_window(child: HWND, top: Option<HWND>) -> String {
    let class = get_window_class(child).unwrap_or_default();
    let (w, h) = GameWindow { hwnd: child }.get_client_size();
    match top {
        Some(t) => format!("top=0x{:X} child=0x{:X} class={} client={}x{}", t.0 as usize, child.0 as usize, class, w, h),
        None => format!("child=0x{:X} class={} client={}x{}", child.0 as usize, class, w, h),
    }
}
