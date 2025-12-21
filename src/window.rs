use std::{ffi::c_void, thread, time::Duration};

use regex::Regex;

use crate::config::{config, set_registry_value, Config};
use crate::logging::{debug_log, LogLevel, LogMode};
use crate::notification::{display_notification, Notification};

use win_screenshot::prelude::*;
use windows::core::PCWSTR;
use windows::Win32::{
    Foundation::{HWND, RECT},
    UI::WindowsAndMessaging::{FindWindowExW, GetClassNameW, GetClientRect, GetParent, IsIconic, ShowWindow, SW_RESTORE},
};

const CROSVM_CLASS: &str = "CROSVM_1";

const LOADING_TITLE: &str = "Google Play Games";
const LOADING_CLASS: &str = "HwndWrapper";
const LOADING_TIMEOUT: u64 = 3;

const ARKNIGHTS_TITLES: [&str; 3] = ["Arknights", "명일방주", "アークナイツ"];

const CACHE_PATH: &str = "Google\\Play Games\\image_cache";
const WINDOW_RESTORE_DELAY_MS: u64 = 300;

pub fn launch_arknights(intent: &str) {
    set_package(intent);
    launch_game();
}

pub fn wait_for_game() {
    if config().package.is_empty() {
        debug_log(LogLevel::Info, LogMode::Nested, "Try detecting package from AppData/Local/Google/Play Games");
        if let Some(package) = detect_arknights_package() {
            debug_log(LogLevel::Info, LogMode::Nested, &format!("Package set: {} (file)", package));
            set_registry_value("PACKAGE", &package).unwrap();
            Config::reload();
        } else {
            debug_log(LogLevel::Warn, LogMode::Nested, "Package detection failed");
            return;
        }
    }
    launch_game();
}

fn detect_arknights_package() -> Option<String> {
    let local_app_data = std::env::var("LOCALAPPDATA").ok()?;
    let cache_path = format!("{}\\{}", local_app_data, CACHE_PATH);

    // Ex: com.YoStar__.Arknights.appicon.ico
    let re = Regex::new(r"^com\.YoStar.+\.Arknights").unwrap();

    std::fs::read_dir(&cache_path).ok()?.flatten().find_map(|e| {
        let name = e.file_name().to_string_lossy().to_string();
        re.find(&name).map(|m| m.as_str().to_string())
    })
}

fn set_package(intent: &str) {
    // Ex: com.YoStar__.Arknights/com.u8.sdk.U8UnityContext
    let package = intent.split('/').next().unwrap_or(intent);

    let current = config().package.clone();
    if package != current {
        debug_log(LogLevel::Info, LogMode::Nested, &format!("Package set: {} (command)", package));
        set_registry_value("PACKAGE", package).unwrap();
        Config::reload();
    }
}

fn launch_game() {
    if get_hwnd().is_some() {
        return;
    }

    let package = config().package.clone();

    if !is_gpg_loading() {
        let _ = open::that(format!("googleplaygames://launch/?id={}", package));
        debug_log(LogLevel::Info, LogMode::Nested, &format!("Launching Google Play Games: {}", package));
    }

    for i in 1..=LOADING_TIMEOUT {
        debug_log(LogLevel::Info, LogMode::Nested, &format!("Waiting for Arknights: {}s / {}s", i, LOADING_TIMEOUT));
        thread::sleep(Duration::from_secs(1));
        if get_hwnd().is_some() {
            debug_log(LogLevel::Info, LogMode::Nested, "Arknights ready");
            return;
        }
    }

    if is_gpg_loading() {
        debug_log(LogLevel::Info, LogMode::Nested, "Timeout: Google Play Games loading");
    } else {
        debug_log(LogLevel::Warn, LogMode::Nested, "Timeout: Google Play Games not responding");
    }
}

fn is_gpg_loading() -> bool {
    window_list().unwrap().into_iter().any(|i| {
        if i.window_name == LOADING_TITLE {
            let hwnd = HWND(i.hwnd as usize as *mut c_void);
            if let Some(class_name) = get_window_class(hwnd) {
                return class_name.starts_with(LOADING_CLASS);
            }
        }
        false
    })
}

pub fn get_hwnd() -> Option<HWND> {
    let windows = window_list().ok()?;
    let current_title = config().title.clone();

    if let Some(hwnd) = try_find_window(&windows, &current_title) {
        return Some(hwnd);
    }

    for &title in &ARKNIGHTS_TITLES {
        if title != current_title {
            if let Some(hwnd) = try_find_window(&windows, title) {
                debug_log(LogLevel::Info, LogMode::Nested, &format!("Title set: {}", title));
                set_registry_value("TITLE", title).unwrap();
                Config::reload();
                return Some(hwnd);
            }
        }
    }

    None
}

fn try_find_window(windows: &[HwndName], title: &str) -> Option<HWND> {
    let pattern = format!("^{}( - .+)?$", title);
    let re = Regex::new(&pattern).ok()?;

    windows.iter().filter(|w| re.is_match(&w.window_name)).find_map(|win| {
        let hwnd = HWND(win.hwnd as usize as *mut c_void);
        let class_name = get_window_class(hwnd)?;

        // Google Play Games window structure has been updated
        // Old: CROSVM_1 > subWin
        // New: HwndWrapper > CROSVM_1 > subWin
        if class_name == CROSVM_CLASS {
            Some(hwnd)
        } else if class_name.starts_with(LOADING_CLASS) {
            find_crosvm(hwnd)
        } else {
            None
        }
    })
}

fn find_crosvm(parent_hwnd: HWND) -> Option<HWND> {
    let crosvm_class_wide: Vec<u16> = CROSVM_CLASS.encode_utf16().chain(Some(0)).collect();

    unsafe {
        FindWindowExW(Some(parent_hwnd), None, PCWSTR(crosvm_class_wide.as_ptr()), None)
            .ok()
            .filter(|h| !h.0.is_null())
    }
}

fn get_window_class(hwnd: HWND) -> Option<String> {
    let mut buffer: [u16; 256] = [0; 256];
    unsafe {
        let len = GetClassNameW(hwnd, &mut buffer);
        if len > 0 {
            let class_name = String::from_utf16_lossy(&buffer[..len as usize]);
            Some(class_name)
        } else {
            None
        }
    }
}

pub fn get_info() -> (HWND, i32, i32) {
    let hwnd = get_hwnd().expect("Failed to find window (get_info)");
    let mut rect = RECT::default();

    let (w, h) = if unsafe { GetClientRect(hwnd, &mut rect) }.is_ok() { (rect.right - rect.left, rect.bottom - rect.top) } else { (0, 0) };

    (hwnd, w, h)
}

pub fn restore_if_minimized(hwnd: HWND) {
    let target_hwnd = unsafe { GetParent(hwnd).ok().filter(|parent| !parent.0.is_null()).unwrap_or(hwnd) };

    if unsafe { IsIconic(target_hwnd).as_bool() } {
        display_notification(Notification::WindowMinimized);
        unsafe { _ = ShowWindow(target_hwnd, SW_RESTORE) };
        thread::sleep(Duration::from_millis(WINDOW_RESTORE_DELAY_MS));
    }
}
