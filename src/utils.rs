use chrono::Local;
use open;
use std::{
    env,
    ffi::c_void,
    fs::{File, OpenOptions},
    io::{stdout, Read, Write},
    net::{Shutdown, TcpStream},
    panic,
    path::PathBuf,
    process::Command,
    thread,
    time::Duration,
};

use image::{codecs::png::PngEncoder, imageops::FilterType::CatmullRom, DynamicImage, Rgb, RgbaImage};
use imageproc::drawing::{draw_filled_circle_mut, draw_line_segment_mut};
use regex::Regex;

use crate::config::{config, get_registry_dword, set_registry_dword, DISPLAY_HEIGHT, DISPLAY_WIDTH, EXTRAS_PORT};
use crate::notification::display_notification;

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

// ============================================================================

fn get_title_pattern() -> String {
    format!("^{}( - .+)?$", config().title)
}

pub fn start_arknights() {
    if get_hwnd().is_some() {
        return;
    }

    let _ = open::that(format!("googleplaygames://launch/?id={}", config().package));

    let found = (0..10).find(|_| {
        thread::sleep(Duration::from_secs(1));
        get_hwnd().is_some()
    });

    if found.is_none() {
        let gpg_pattern = r"^Google Play .+";
        let re = Regex::new(gpg_pattern).unwrap();

        let gpg_window = window_list().unwrap().into_iter().find(|i| re.is_match(&i.window_name));

        if gpg_window.is_some() {
            debug_log(LogLevel::INFO, "Google Play Games is still loading", None);
            return;
        } else {
            let pattern = get_title_pattern();
            display_notification(LogLevel::ERROR, "gpg_start_failed", &[&pattern]);
            panic!("Failed to start Google Play Games or detect\nTarget regex: {}\nPackage: {}", pattern, config().package);
        }
    }
}

pub fn get_hwnd() -> Option<HWND> {
    let pattern = get_title_pattern(); // Title - Player ID
    let re = Regex::new(&pattern).unwrap();

    let window = window_list().unwrap().into_iter().find(|i| re.is_match(&i.window_name));

    window.map(|w| HWND(w.hwnd as usize as *mut c_void))
}

pub fn get_info() -> (HWND, i32, i32) {
    let hwnd = get_hwnd().expect("Failed to find window (get_info)");
    let mut rect = RECT::default();

    let (w, h) = if unsafe { GetClientRect(hwnd, &mut rect) }.is_ok() { (rect.right - rect.left, rect.bottom - rect.top) } else { (0, 0) };

    (hwnd, w, h)
}

// ============================================================================

const EXTRAS_TIMEOUT: u64 = 50;

fn extras_exists() -> bool {
    let mut path: PathBuf = match std::env::current_exe() {
        Ok(p) => p,
        Err(_) => return false,
    };
    path.set_file_name("PlayBridgeExtras.exe");
    path.exists()
}

fn spawn_extras_process() -> std::io::Result<()> {
    if let Ok(_) = TcpStream::connect_timeout(&format!("127.0.0.1:{}", EXTRAS_PORT).parse().unwrap(), Duration::from_millis(EXTRAS_TIMEOUT)) {
        return Ok(());
    }

    let mut path: PathBuf = std::env::current_exe()?;
    path.set_file_name("PlayBridgeExtras.exe");

    Command::new(path).spawn().map(|_| ())
}

fn request_extras_image() -> bool {
    if !extras_exists() {
        return false;
    }

    let mut stream = match TcpStream::connect_timeout(&format!("127.0.0.1:{}", EXTRAS_PORT).parse().unwrap(), Duration::from_millis(EXTRAS_TIMEOUT)) {
        Ok(stream) => stream,
        Err(_) => return false,
    };

    stream.write_all(b"GET").unwrap();
    stream.shutdown(Shutdown::Write).unwrap();

    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).unwrap();
    stdout().lock().write_all(&buf).unwrap();

    !buf.is_empty() // return
}

pub fn invalidate_extras_image() {
    if !extras_exists() {
        return;
    }

    // Input tap - wait for load next frame
    thread::sleep(Duration::from_millis(150));

    if let Ok(mut stream) = TcpStream::connect(format!("127.0.0.1:{}", EXTRAS_PORT)) {
        stream.write_all(b"INV").unwrap();
        stream.shutdown(Shutdown::Write).unwrap();
    }
}

// ============================================================================

pub fn capture() -> DynamicImage {
    let hwnd = get_hwnd().expect("Failed to find window (capture)");

    if unsafe { IsIconic(hwnd).as_bool() } {
        display_notification(LogLevel::WARN, "window_minimized", &[]);
        unsafe { _ = ShowWindow(hwnd, SW_RESTORE) };
        thread::sleep(Duration::from_millis(300));
    }

    let buf = capture_window_ex(hwnd.0 as isize, Using::PrintWindow, Area::ClientOnly, None, None).unwrap();

    let width = buf.width;
    let height = buf.height;

    let img = DynamicImage::ImageRgba8(RgbaImage::from_raw(width, height, buf.pixels).unwrap());

    img.resize(DISPLAY_WIDTH, DISPLAY_HEIGHT, CatmullRom)
}

pub fn send_capture() {
    let (_, w, h) = get_info();
    check_window_size(w as u32, h as u32);

    if extras_exists() {
        spawn_extras_process().unwrap();
    }

    // Extras
    if request_extras_image() {
        return;
    }

    // fallback
    let img = capture();
    img.write_with_encoder(PngEncoder::new(&mut stdout().lock())).unwrap();
}

pub fn screenshot() {
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

    let exe_name = env::current_exe().ok().and_then(|p| p.file_stem().map(|s| s.to_string_lossy().to_string())).unwrap_or_else(|| "UNK".into());

    let prog_tag = match exe_name.as_str() {
        "PlayBridgeADB" => "ADB",
        "PlayBridgeExtras" => "Extras",
        _ => exe_name.as_str(),
    };

    let flat_message = message.replace('\n', " ");

    let log = match elapsed_ms {
        Some(ms) => format!("[{}][{}][{}] {} , cost {} ms", now, level_tag, prog_tag, flat_message, ms),
        None => format!("[{}][{}][{}] {}", now, level_tag, prog_tag, flat_message),
    };

    let _ = writeln!(file, "{}", log);
}

pub fn panic_hook() {
    panic::set_hook(Box::new(|info| {
        let msg = info.payload().downcast_ref::<&str>().map(|s| *s).or_else(|| info.payload().downcast_ref::<String>().map(|s| s.as_str())).unwrap_or("Unknown panic message");

        let location = info.location().map(|l| format!("{}:{}", l.file(), l.line())).unwrap_or_else(|| "unknown location".into());

        display_notification(LogLevel::ERROR, "panic", &[&location, msg]);
    }));
}

fn check_window_size(width: u32, height: u32) {
    let ratio = height as f32 / width as f32;
    let target_ratio = 9.0 / 16.0;
    if (ratio - target_ratio).abs() > 0.001 {
        let display_ratio = ratio * 16.0;
        display_notification(LogLevel::WARN, "wrong_ratio", &[&format!("{:.1}", display_ratio)]);
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
