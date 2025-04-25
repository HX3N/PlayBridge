use chrono::Local;
use open;
use std::time::Instant;
use std::{env, ffi::c_void, fs::File, io::stdout};
use std::{thread, time::Duration};

use image::{codecs::png::PngEncoder, imageops::FilterType::Nearest, DynamicImage, RgbaImage};
use regex::Regex;

use crate::config::CONFIG;
use crate::notification::{get_registry_dword, set_registry_dword, show_notification};

use std::fs::OpenOptions;
use std::io::Write;
use std::panic;
use win_screenshot::prelude::*;
use windows::Win32::{
    Foundation::{HWND, RECT},
    UI::WindowsAndMessaging::{GetClientRect, IsIconic, ShowWindow, SW_RESTORE},
};

#[derive(Copy, Clone)]
pub enum LogLevel {
    INFO,
    WARN,
    ERROR,
}

pub fn debug_log(level: LogLevel, message: &str, elapsed_ms: Option<u128>) {
    if !CONFIG.debug {
        return;
    }

    let Ok(mut file) = OpenOptions::new().append(true).create(true).open("PlayBridgeADB.log") else {
        return;
    };

    let now = Local::now().format("%Y-%m-%d %H:%M:%S%.3f");
    let prefix = match level {
        LogLevel::INFO => "INF",
        LogLevel::WARN => "WRN",
        LogLevel::ERROR => "ERR",
    };

    let log = match elapsed_ms {
        Some(ms) => format!("[{}][{}] {} , cost {} ms", now, prefix, message, ms),
        None => format!("[{}][{}] {}", now, prefix, message),
    };

    let _ = writeln!(file, "{}", log);
}

pub fn debug_panic() {
    if !CONFIG.debug {
        return;
    }

    panic::set_hook(Box::new(|info| {
        let msg = info.payload().downcast_ref::<&str>().map(|s| *s).or_else(|| info.payload().downcast_ref::<String>().map(|s| s.as_str())).unwrap_or("Unknown panic message");

        let location = info.location().map(|l| format!("{}:{}", l.file(), l.line())).unwrap_or_else(|| "unknown location".into());

        debug_log(LogLevel::ERROR, &format!("PANIC at {}: {}", location, msg), None);
    }));
}

pub fn get_now() -> Option<Instant> {
    CONFIG.debug.then(Instant::now)
}

pub fn run_arknights() {
    if get_hwnd().is_some() {
        return;
    }

    let _ = open::that(format!("googleplaygames://launch/?id={}", CONFIG.package));

    let found = (0..30).find(|_| {
        thread::sleep(Duration::from_secs(1));
        get_hwnd().is_some()
    });

    if found.is_none() {
        show_notification(LogLevel::WARN, "Failed to launch Arknights or detect window", "start_arknights_failed");
        panic!("Failed to launch Arknights or detect its window within timeout");
    }
}

pub fn get_hwnd() -> Option<HWND> {
    let pattern = format!("^{}( - .+)?$", CONFIG.title);
    let re = Regex::new(&pattern).unwrap();

    let window = window_list().expect("Failed to window_list").into_iter().find(|i| re.is_match(&i.window_name));

    window.map(|w| HWND(w.hwnd as usize as *mut c_void))
}

pub fn get_info() -> (HWND, i32, i32) {
    let hwnd = get_hwnd().unwrap();
    let mut rect = RECT::default();

    let (w, h) = if unsafe { GetClientRect(hwnd, &mut rect) }.is_ok() { (rect.right - rect.left, rect.bottom - rect.top) } else { (0, 0) };

    (hwnd, w, h)
}

fn capture() -> DynamicImage {
    let hwnd = get_hwnd().unwrap();

    if unsafe { IsIconic(hwnd).as_bool() } {
        show_notification(LogLevel::WARN, "Minimized window is not supported", "minimized_not_supported");
        unsafe { _ = ShowWindow(hwnd, SW_RESTORE) };
        thread::sleep(Duration::from_millis(300));
    }

    let buf = capture_window_ex(hwnd.0 as isize, Using::PrintWindow, Area::ClientOnly, None, None).expect("Failed to capture_window_ex");

    let width = buf.width;
    let height = buf.height;

    check_resolution(width, height);

    let img = DynamicImage::ImageRgba8(RgbaImage::from_raw(width, height, buf.pixels).expect("Failed to create RgbaImage"));
    img.resize(CONFIG.width, CONFIG.height, Nearest)
}

pub fn capture_maa() {
    let img = capture();
    img.write_with_encoder(PngEncoder::new(&mut stdout().lock())).expect("Failed to write image to stdout");
}

pub fn capture_screenshot() {
    let img = capture();
    let filename = format!("Screenshot_{}.png", Local::now().format("%Y.%m.%d_%H.%M.%S.%3f"));
    let filepath = format!("{}\\Desktop\\{}", env::var("USERPROFILE").unwrap(), filename);
    img.write_with_encoder(PngEncoder::new(File::create(&filepath).unwrap())).expect("Failed to write image to screenshot file");

    show_notification(LogLevel::INFO, "Screenshot saved!", "screenshot_saved");
}

fn check_resolution(width: u32, height: u32) {
    let ratio = height as f32 / width as f32;
    let target_ratio = 9.0 / 16.0;
    if (ratio - target_ratio).abs() > 0.001 {
        show_notification(LogLevel::WARN, &format!("Aspect ratio is not 16:9 (16:{:.1})", ratio * 16.0), "not_16_9_ratio");
        return;
    }

    if width < (1280.0 * 0.8) as u32 || height < (720.0 * 0.8) as u32 {
        show_notification(LogLevel::WARN, &format!("Resolution is too low ({}x{})", width, height), "resolution_too_low");
        return;
    }

    let stored_width = get_registry_dword("width").unwrap_or(0);
    let stored_height = get_registry_dword("height").unwrap_or(0);

    if stored_width != width || stored_height != height {
        set_registry_dword("width", width).expect("Failed to write width to registry");
        set_registry_dword("height", height).expect("Failed to write height to registry");

        if stored_width == 0 || stored_height == 0 {
            show_notification(LogLevel::INFO, &format!("Resolution info ({}x{})", width, height), "resolution_init");
        } else {
            show_notification(LogLevel::INFO, &format!("Resolution changed ({}x{})", width, height), "resolution_changed");
        }
    }
}
