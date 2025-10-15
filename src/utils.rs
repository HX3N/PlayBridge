use chrono::Local;
use open;
use std::{
    env,
    ffi::c_void,
    fs::{File, OpenOptions},
    io::{stdout, Write},
    panic,
    path::PathBuf,
    thread,
    time::Duration,
};

use image::{codecs::png::PngEncoder, imageops::FilterType::CatmullRom, DynamicImage, Rgb, RgbaImage};
use imageproc::drawing::{draw_filled_circle_mut, draw_line_segment_mut};
use regex::Regex;

use crate::config::*;
use crate::notification::display_notification;

use win_screenshot::prelude::*;
use windows::core::PCWSTR;
use windows::Win32::{
    Foundation::{HWND, RECT},
    UI::HiDpi::{GetWindowDpiAwarenessContext, SetThreadDpiAwarenessContext},
    UI::WindowsAndMessaging::*,
};

#[derive(Copy, Clone)]
pub enum LogLevel {
    INFO,
    WARN,
    ERROR,
}

// ============================================================================

const CROSVM_CLASS: &str = "CROSVM_1";
const GPG_LOADING_TIMEOUT: u64 = 3;

pub fn launch_arknights(intent: &str) {
    if get_hwnd().is_some() {
        return;
    }

    let package = get_package(&intent);
    let _ = open::that(format!("googleplaygames://launch/?id={}", package));

    let _ = (0..GPG_LOADING_TIMEOUT).find(|_| {
        thread::sleep(Duration::from_secs(1));
        get_hwnd().is_some()
    });

    if get_hwnd().is_some() {
        return;
    }

    if is_gpg_loading() {
        display_notification(LogLevel::INFO, "gpg_loading", &[]);
        return;
    }

    panic!("Failed to start Google Play Games\nPackage: {}", get_package(&intent));
}

fn is_gpg_loading() -> bool {
    let gpg_pattern = r"^Google Play .+";
    let re = Regex::new(gpg_pattern).unwrap();

    let gpg_window = window_list().unwrap().into_iter().find(|i| re.is_match(&i.window_name));
    gpg_window.is_some()
}

pub fn ensure_gpg_ready() {
    if get_hwnd().is_some() {
        return;
    }

    if config().package == "Unknown" {
        return;
    }

    if !is_gpg_loading() {
        let _ = open::that(format!("googleplaygames://launch/?id={}", &config().package));
        debug_log(LogLevel::INFO, "Google Play Games not started, launching...", None);
    }

    debug_log(LogLevel::INFO, "Waiting for Google Play Games to be ready", None);
    let ready = (0..GPG_LOADING_TIMEOUT)
        .find(|_| {
            thread::sleep(Duration::from_secs(1));
            get_hwnd().is_some()
        })
        .is_some();

    if ready {
        debug_log(LogLevel::INFO, "Google Play Games is ready", None);
    } else {
        debug_log(LogLevel::WARN, "Google Play Games is still loading...", None);
    }
}

fn get_package(intent: &str) -> String {
    // com.YoStar__.Arknights/com.u8.sdk.U8UnityContext
    let package = if let Some(slash_pos) = intent.find('/') { intent[..slash_pos].to_string() } else { intent.to_string() };

    if package != config().package {
        display_notification(LogLevel::INFO, "registry_updated", &[&"PACKAGE", &config().package, &package.to_string()]);
        set_registry_value("PACKAGE", &package).unwrap();
    }

    package
}

pub fn get_hwnd() -> Option<HWND> {
    if let Some(hwnd) = try_find_window(&config().title) {
        return Some(hwnd);
    }

    let fallback_titles = ["Arknights", "명일방주", "アークナイツ"];

    for &title in &fallback_titles {
        if title != config().title {
            if let Some(hwnd) = try_find_window(title) {
                display_notification(LogLevel::INFO, "registry_updated", &[&"TITLE", &config().title, &title.to_string()]);
                set_registry_value("TITLE", title).unwrap();
                return Some(hwnd);
            }
        }
    }

    None
}

fn try_find_window(title: &str) -> Option<HWND> {
    let pattern = format!("^{}( - .+)?$", title);
    let re = Regex::new(&pattern).ok()?;

    let window = window_list().ok()?.into_iter().find(|i| re.is_match(&i.window_name))?;
    let hwnd = HWND(window.hwnd as usize as *mut c_void);

    // Google Play Games window structure has been updated
    // Old: CROSVM_1 > subWin
    // New: HwndWrapper > CROSVM_1 > subWin
    if get_window_class(hwnd).as_deref() == Some(CROSVM_CLASS) {
        return Some(hwnd);
    }
    find_crosvm(hwnd).or(Some(hwnd))
}

fn find_crosvm(parent_hwnd: HWND) -> Option<HWND> {
    let crosvm_class_wide: Vec<u16> = CROSVM_CLASS.encode_utf16().chain(Some(0)).collect();

    unsafe {
        let crosvm_hwnd =
            FindWindowExW(Some(parent_hwnd), None, PCWSTR(crosvm_class_wide.as_ptr()), None).expect("Failed to find crosvm window");

        if crosvm_hwnd.0.is_null() {
            None
        } else {
            Some(crosvm_hwnd)
        }
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

// ============================================================================

fn restore_if_minimized(hwnd: HWND) {
    let target_hwnd = unsafe { GetParent(hwnd).ok().filter(|parent| !parent.0.is_null()).unwrap_or(hwnd) };

    if unsafe { IsIconic(target_hwnd).as_bool() } {
        display_notification(LogLevel::WARN, "window_minimized", &[]);
        unsafe { _ = ShowWindow(target_hwnd, SW_RESTORE) };
        thread::sleep(Duration::from_millis(300));
    }
}

pub fn capture() -> DynamicImage {
    let (hwnd, _, _) = get_info();

    restore_if_minimized(hwnd);

    // Handle mixed DPI settings properly in multiple-monitor setups (ex. main 125%, sub 100%)
    unsafe { _ = SetThreadDpiAwarenessContext(GetWindowDpiAwarenessContext(hwnd)) };

    let buf = capture_window_ex(hwnd.0 as isize, Using::PrintWindow, Area::ClientOnly, None, None).unwrap();

    let width = buf.width;
    let height = buf.height;

    let img = DynamicImage::ImageRgba8(RgbaImage::from_raw(width, height, buf.pixels).unwrap());

    img.resize(DISPLAY_WIDTH, DISPLAY_HEIGHT, CatmullRom)
}

pub fn send_capture() {
    if get_hwnd().is_none() {
        debug_log(LogLevel::INFO, "Capture requested but window not found, sending black image instead", None);

        // Send black image for spoofing GPG Loading
        let black_pixels = vec![0u8; (DISPLAY_WIDTH * DISPLAY_HEIGHT * 3) as usize]; // RGB format
        let img = DynamicImage::ImageRgb8(image::RgbImage::from_raw(DISPLAY_WIDTH, DISPLAY_HEIGHT, black_pixels).unwrap());
        img.write_with_encoder(PngEncoder::new(&mut stdout().lock())).unwrap();
        return;
    }

    let (_, w, h) = get_info();
    check_window_size(w as u32, h as u32);

    let img = capture();
    img.write_with_encoder(PngEncoder::new(&mut stdout().lock())).unwrap();
}

pub fn screenshot() {
    if get_hwnd().is_none() {
        display_notification(LogLevel::INFO, "screenshot_failed", &[]);
        return;
    }

    let img = capture();
    let filename = format!("Screenshot_{}.png", Local::now().format("%Y.%m.%d_%H.%M.%S.%3f"));
    let filepath = format!("{}\\Desktop\\{}", env::var("USERPROFILE").unwrap(), filename);
    img.write_with_encoder(PngEncoder::new(File::create(&filepath).unwrap())).unwrap();

    display_notification(LogLevel::INFO, "screenshot", &[]);
}

pub fn debug_capture(x: i32, y: i32, end_point: Option<(i32, i32)>) {
    if !config().debug_capture {
        return;
    }

    let img = capture();
    let mut img_rgb = img.to_rgb8();

    match end_point {
        Some((x2, y2)) => {
            // start to end line
            draw_line_segment_mut(&mut img_rgb, (x as f32, y as f32), (x2 as f32, y2 as f32), Rgb([0, 255, 0]));

            // swipe start point
            draw_filled_circle_mut(&mut img_rgb, (x, y), 6, Rgb([255, 255, 255]));
            draw_filled_circle_mut(&mut img_rgb, (x, y), 5, Rgb([255, 0, 0]));

            // swipe end point
            draw_filled_circle_mut(&mut img_rgb, (x2, y2), 6, Rgb([255, 255, 255]));
            draw_filled_circle_mut(&mut img_rgb, (x2, y2), 5, Rgb([0, 0, 255]));
        }
        None => {
            // tap point
            draw_filled_circle_mut(&mut img_rgb, (x, y), 6, Rgb([255, 255, 255]));
            draw_filled_circle_mut(&mut img_rgb, (x, y), 5, Rgb([255, 0, 0]));
        }
    }

    let filename = format!("Debug_{}.png", Local::now().format("%Y.%m.%d_%H.%M.%S.%3f"));
    let filepath = get_debug_folder().join(filename);

    let dynamic_img = DynamicImage::ImageRgb8(img_rgb);
    dynamic_img.write_with_encoder(PngEncoder::new(File::create(&filepath).unwrap())).unwrap();
}

// ============================================================================

fn get_debug_folder() -> PathBuf {
    let exe_dir = env::current_exe().unwrap().parent().unwrap().to_path_buf();
    let folder_path = exe_dir.join("PlayBridge");
    let _ = std::fs::create_dir_all(&folder_path);
    folder_path
}

pub fn debug_log(level: LogLevel, message: &str, elapsed_ms: Option<u128>) {
    if !config().debug && !matches!(level, LogLevel::ERROR) {
        return;
    }

    let log_path = get_debug_folder().join("debug.log");
    let Ok(mut file) = OpenOptions::new().append(true).create(true).open(log_path) else {
        return;
    };

    let now = Local::now().format("%Y-%m-%d %H:%M:%S%.3f");
    let level_tag = match level {
        LogLevel::INFO => "INF",
        LogLevel::WARN => "WRN",
        LogLevel::ERROR => "ERR",
    };

    let flat_message = message.replace('\n', " ");

    let log = match elapsed_ms {
        Some(ms) => format!("[{}][{}] {} , cost {} ms", now, level_tag, flat_message, ms),
        None => format!("[{}][{}] {}", now, level_tag, flat_message),
    };

    let _ = writeln!(file, "{}", log);
}

pub fn panic_hook() {
    panic::set_hook(Box::new(|info| {
        let msg = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| *s)
            .or_else(|| info.payload().downcast_ref::<String>().map(|s| s.as_str()))
            .unwrap_or("Unknown panic message");

        let location = info
            .location()
            .map(|l| format!("{}:{}", l.file(), l.line()))
            .unwrap_or_else(|| "unknown location".into());

        display_notification(LogLevel::ERROR, "panic", &[&location, msg]);
    }));
}

fn check_window_size(width: u32, height: u32) {
    let height_ratio = height as f32 / (width as f32 / 16.0);

    if (height_ratio - 9.0).abs() > 0.1 {
        display_notification(LogLevel::WARN, "window_wrong_ratio", &[&format!("{:.2}", height_ratio)]);
        return;
    }

    if width < (DISPLAY_WIDTH as f32 * 0.8) as u32 || height < (DISPLAY_HEIGHT as f32 * 0.8) as u32 {
        display_notification(LogLevel::WARN, "window_too_small", &[&width.to_string(), &height.to_string()]);
        return;
    }

    let stored_width = get_registry_dword("width", &config().notification_path).unwrap_or(0);
    let stored_height = get_registry_dword("height", &config().notification_path).unwrap_or(0);

    if stored_width != width || stored_height != height {
        set_registry_dword("width", width, &config().notification_path).unwrap();
        set_registry_dword("height", height, &config().notification_path).unwrap();

        if stored_width == 0 || stored_height == 0 {
            display_notification(LogLevel::INFO, "window_info", &[&width.to_string(), &height.to_string()]);
        } else {
            display_notification(LogLevel::INFO, "window_changed", &[&width.to_string(), &height.to_string()]);
        }
    }
}

fn get_folder_size(folder_path: &PathBuf) -> u64 {
    let mut total_size = 0u64;

    if let Ok(entries) = std::fs::read_dir(folder_path) {
        for entry in entries.flatten() {
            if let Ok(metadata) = entry.metadata() {
                total_size += metadata.len();
            }
        }
    }

    total_size / (1024 * 1024) // MB
}

pub fn check_folder_size() {
    if !config().debug && !config().debug_capture {
        return;
    }

    const WARNING_INTERVAL: u64 = 250; // 250MB

    let debug_folder = get_debug_folder();
    let current = get_folder_size(&debug_folder);

    if current < WARNING_INTERVAL {
        if get_registry_dword("debug_folder_last_warned_size", &config().notification_path).unwrap_or(0) != 0 {
            set_registry_dword("debug_folder_last_warned_size", 0, &config().notification_path).unwrap();
        }
        return;
    }

    let last_warned = get_registry_dword("debug_folder_last_warned_size", &config().notification_path).unwrap_or(0) as u64;

    let current_level = current / WARNING_INTERVAL;
    let last_warned_level = last_warned / WARNING_INTERVAL;

    if current_level > last_warned_level {
        display_notification(LogLevel::WARN, "storage_warning", &[&current.to_string()]);
        set_registry_dword("debug_folder_last_warned_size", current as u32, &config().notification_path).unwrap();
    }
}
