use std::{ffi::c_void, thread, time::Duration};

use win_screenshot::prelude::*;
use windows::core::PCWSTR;
use windows::Win32::{
    Foundation::{HWND, RECT},
    UI::WindowsAndMessaging::{FindWindowExW, GetClassNameW, GetClientRect, GetParent, IsIconic, ShowWindow, SW_RESTORE},
};

use crate::config::{config, set_client, Client};
use crate::logging::{debug_log, LogLevel, LogMode};
use crate::notification::{display_notification, Notification};

const WRAPPER_CLASS: &str = "HwndWrapper";
const CROSVM_CLASS: &str = "CROSVM_1";

const LOADING_TITLE: &str = "Google Play Games";

const CACHE_PATH: &str = "Google/Play Games/image_cache";
const WINDOW_RESTORE_DELAY_MS: u64 = 300;

pub fn parent_or_self(hwnd: HWND) -> HWND {
    unsafe { GetParent(hwnd).ok().filter(|p| !p.0.is_null()).unwrap_or(hwnd) }
}

pub struct GameWindow {
    pub hwnd: HWND,
}

impl GameWindow {
    pub fn find() -> Option<Self> {
        let windows = window_list().ok()?;
        let current_title = config().client.title();

        if !current_title.is_empty() {
            if let Some(hwnd) = match_window_by_title(&windows, current_title) {
                return Some(Self { hwnd });
            }
        }

        for client in [Client::KR, Client::JP, Client::EN] {
            if client.title() != current_title {
                if let Some(hwnd) = match_window_by_title(&windows, client.title()) {
                    set_client(client);
                    return Some(Self { hwnd });
                }
            }
        }

        None
    }

    pub fn restore(&self) {
        let target_hwnd = parent_or_self(self.hwnd);

        if unsafe { IsIconic(target_hwnd).as_bool() } {
            display_notification(Notification::WindowMinimized);
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

pub fn ensure_game_ready() {
    if config().client == Client::Empty {
        debug_log(LogLevel::Info, LogMode::Nested, "Package: detecting from AppData/Local/Google/Play Games");
        if let Some(package) = find_installed_package() {
            if let Some(client) = resolve_client(&package) {
                set_client(client);
            }
        } else {
            debug_log(LogLevel::Warn, LogMode::Nested, "Package: detection failed");
            return;
        }
    }

    start_game_if_needed();
}

pub fn apply_intent_package(intent: &str) {
    // Ex: com.YoStar__.Arknights/com.u8.sdk.U8UnityContext
    let package = intent.split('/').next().unwrap_or(intent);

    if config().client.package() == package {
        return;
    }

    if let Some(client) = resolve_client(package) {
        let current_client = config().client;
        if current_client != Client::Empty && current_client != client {
            display_notification(Notification::ClientMismatch(client.package().to_string(), current_client.package().to_string()));
            return;
        }
        set_client(client);
    } else {
        display_notification(Notification::UnsupportedClient(package.to_string()));
    }
}

pub fn start_game_if_needed() {
    if GameWindow::find().is_some() {
        return;
    }

    if !is_loading_screen_active() {
        let launch_uri = format!("googleplaygames://launch/?id={}&pid=1", config().client.package());

        debug_log(LogLevel::Info, LogMode::Nested, &format!("Launching Google Play Games: {}", launch_uri));
        let _ = open::that(launch_uri);
    }

    debug_log(LogLevel::Info, LogMode::Nested, "Waiting for Arknights");
    thread::sleep(Duration::from_secs(1));

    if GameWindow::find().is_some() {
        debug_log(LogLevel::Info, LogMode::Nested, "Arknights ready");
    } else if is_loading_screen_active() {
        debug_log(LogLevel::Info, LogMode::Nested, "Google Play Games loading");
    } else {
        debug_log(LogLevel::Warn, LogMode::Nested, "Google Play Games not responding");
    }
}

fn find_installed_package() -> Option<String> {
    let local_app_data = std::env::var("LOCALAPPDATA").ok()?;
    let cache_path = format!("{}/{}", local_app_data, CACHE_PATH);

    // Ex: com.YoStar__.Arknights.appicon.ico
    std::fs::read_dir(&cache_path).ok()?.flatten().find_map(|e| {
        let name = e.file_name().to_string_lossy().to_string();
        if name.starts_with("com.") && name.contains(".Arknights") {
            Some(name)
        } else {
            None
        }
    })
}

fn resolve_client(package: &str) -> Option<Client> {
    [Client::KR, Client::JP, Client::EN]
        .into_iter()
        .find(|&client| package.starts_with(client.package()))
}

fn match_window_by_title(windows: &[HwndName], title: &str) -> Option<HWND> {
    windows.iter().filter(|w| w.window_name.starts_with(title)).find_map(|win| {
        let hwnd = HWND(win.hwnd as usize as *mut c_void);
        let class_name = get_window_class(hwnd)?;

        // Old: CROSVM_1 > subWin
        // New: HwndWrapper > CROSVM_1 > subWin
        if class_name == CROSVM_CLASS {
            Some(hwnd)
        } else if class_name.starts_with(WRAPPER_CLASS) {
            find_crosvm_child(hwnd)
        } else {
            None
        }
    })
}

fn find_crosvm_child(parent_hwnd: HWND) -> Option<HWND> {
    let crosvm_class_wide: Vec<u16> = CROSVM_CLASS.encode_utf16().chain(Some(0)).collect();

    unsafe {
        FindWindowExW(Some(parent_hwnd), None, PCWSTR(crosvm_class_wide.as_ptr()), None)
            .ok()
            .filter(|h| !h.0.is_null())
    }
}

fn is_loading_screen_active() -> bool {
    window_list().unwrap_or_default().into_iter().any(|i| {
        if i.window_name == LOADING_TITLE {
            let hwnd = HWND(i.hwnd as usize as *mut c_void);
            if let Some(class_name) = get_window_class(hwnd) {
                return class_name.starts_with(WRAPPER_CLASS);
            }
        }
        false
    })
}

fn get_window_class(hwnd: HWND) -> Option<String> {
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
