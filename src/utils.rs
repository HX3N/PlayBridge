use chrono::Local;
use open;
use std::net::{Shutdown, TcpStream};
use std::process::Command;
use std::{
    env,
    ffi::c_void,
    fs::File,
    io::{stdout, Read, Write},
};
use std::{thread, time::Duration};

use image::{codecs::png::PngEncoder, imageops::FilterType::CatmullRom, DynamicImage, Rgb, RgbaImage};
use imageproc::drawing::{draw_filled_circle_mut, draw_line_segment_mut};
use regex::Regex;

use crate::config::{get_config, get_registry_dword, set_registry_dword, DISPLAY_HEIGHT, DISPLAY_WIDTH, EXTRAS_PORT};
use crate::notification::show_notification;

use std::fs::OpenOptions;
use std::panic;
use std::path::PathBuf;
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

pub fn start_arknights() {
    if get_hwnd().is_some() {
        return;
    }

    let _ = open::that(format!("googleplaygames://launch/?id={}", get_config().package));

    let found = (0..45).find(|_| {
        thread::sleep(Duration::from_secs(1));
        get_hwnd().is_some()
    });

    if found.is_none() {
        show_notification(LogLevel::WARN, &format!("Failed to start Arknights or detect window\ntarget title: {}", get_config().title), "start_arknights_failed");
        panic!("Failed to start Arknights or detect its window within timeout");
    }
}

pub fn get_hwnd() -> Option<HWND> {
    let pattern = format!("^{}( - .+)?$", get_config().title); // Player ID
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

// ============================================================================

fn extras_app_exists() -> bool {
    let mut path: PathBuf = match std::env::current_exe() {
        Ok(p) => p,
        Err(_) => return false,
    };
    path.set_file_name("PlayBridgeExtras.exe");
    path.exists()
}

fn try_use_extras_image() -> bool {
    if !extras_app_exists() {
        return false;
    }

    let image_data_result = 'block: {
        for _ in 0..50 {
            if let Ok(data) = request_extras_image() {
                break 'block Ok(data);
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }

        request_extras_image()
    };

    if let Ok(data) = image_data_result {
        if stdout().lock().write_all(&data).is_ok() {
            return true;
        }
    }

    false
}

fn request_extras_image() -> std::io::Result<Vec<u8>> {
    let mut stream = TcpStream::connect(format!("127.0.0.1:{}", EXTRAS_PORT))?;
    stream.write_all(b"GET")?;
    stream.shutdown(Shutdown::Write)?;

    let mut buf = Vec::new();
    stream.read_to_end(&mut buf)?;
    Ok(buf)
}

pub fn invalidate_extras_image() {
    if !extras_app_exists() {
        return;
    }

    // Input tap - wait for load next frame
    thread::sleep(Duration::from_millis(100));

    if let Ok(mut stream) = TcpStream::connect(format!("127.0.0.1:{}", EXTRAS_PORT)) {
        let _ = stream.write_all(b"INV");
        let _ = stream.shutdown(Shutdown::Write);
    }
}

pub fn spawn_extras_process() -> std::io::Result<()> {
    if !extras_app_exists() {
        return Ok(());
    }

    if let Ok(_) = TcpStream::connect(format!("127.0.0.1:{}", EXTRAS_PORT)) {
        return Ok(());
    }

    let mut path: PathBuf = std::env::current_exe()?;
    path.set_file_name("PlayBridgeExtras.exe");

    Command::new(path).spawn().map(|_| ())
}

// ============================================================================

pub fn capture() -> DynamicImage {
    let hwnd = get_hwnd().unwrap();

    if unsafe { IsIconic(hwnd).as_bool() } {
        show_notification(LogLevel::WARN, "Minimized window is not supported", "minimized_not_supported");
        unsafe { _ = ShowWindow(hwnd, SW_RESTORE) };
        thread::sleep(Duration::from_millis(300));
    }

    let buf = capture_window_ex(hwnd.0 as isize, Using::PrintWindow, Area::ClientOnly, None, None).expect("Failed to capture_window_ex");

    let width = buf.width;
    let height = buf.height;

    let img = DynamicImage::ImageRgba8(RgbaImage::from_raw(width, height, buf.pixels).expect("Failed to create RgbaImage"));
    img.resize(DISPLAY_WIDTH, DISPLAY_HEIGHT, CatmullRom)
}

pub fn capture_maa() {
    let (_, w, h) = get_info();
    check_window_size(w as u32, h as u32);

    let _ = spawn_extras_process();

    // Extras
    if try_use_extras_image() {
        return;
    }

    // fallback
    let img = capture();
    img.write_with_encoder(PngEncoder::new(&mut stdout().lock())).expect("Failed to write image to stdout");
}

pub fn capture_screenshot() {
    let img = capture();
    let filename = format!("Screenshot_{}.png", Local::now().format("%Y.%m.%d_%H.%M.%S.%3f"));
    let filepath = format!("{}\\Desktop\\{}", env::var("USERPROFILE").unwrap(), filename);
    img.write_with_encoder(PngEncoder::new(File::create(&filepath).unwrap())).expect("Failed to write image to screenshot");

    show_notification(LogLevel::INFO, "Screenshot saved!", "screenshot_saved");
}

pub fn capture_debug(x: i32, y: i32, end_point: Option<(i32, i32)>) {
    if !get_config().debug_capture {
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
    dynamic_img.write_with_encoder(PngEncoder::new(File::create(&filepath).unwrap())).expect("Failed to write debug image (capture_debug)");
}

// ============================================================================

fn get_debug_folder() -> PathBuf {
    let exe_dir = env::current_exe().unwrap().parent().unwrap().to_path_buf();
    let folder_path = exe_dir.join("PlayBridge");
    let _ = std::fs::create_dir_all(&folder_path);
    folder_path
}

pub fn debug_log(level: LogLevel, message: &str, elapsed_ms: Option<u128>) {
    if !get_config().debug && !matches!(level, LogLevel::ERROR) {
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

pub fn debug_panic() {
    panic::set_hook(Box::new(|info| {
        let msg = info.payload().downcast_ref::<&str>().map(|s| *s).or_else(|| info.payload().downcast_ref::<String>().map(|s| s.as_str())).unwrap_or("Unknown panic message");

        let location = info.location().map(|l| format!("{}:{}", l.file(), l.line())).unwrap_or_else(|| "unknown location".into());

        debug_log(LogLevel::ERROR, &format!("PANIC at {}: {}", location, msg), None);
    }));
}

fn check_window_size(width: u32, height: u32) {
    let ratio = height as f32 / width as f32;
    let target_ratio = 9.0 / 16.0;
    if (ratio - target_ratio).abs() > 0.001 {
        show_notification(LogLevel::WARN, &format!("Aspect ratio is not 16:9 (16:{:.1})", ratio * 16.0), "not_16_9_ratio");
        return;
    }

    if width < (DISPLAY_WIDTH as f32 * 0.8) as u32 || height < (DISPLAY_HEIGHT as f32 * 0.8) as u32 {
        show_notification(LogLevel::WARN, &format!("Window size is too low ({}x{})", width, height), "window_size_too_low");
        return;
    }

    let stored_width = get_registry_dword("width").unwrap_or(0);
    let stored_height = get_registry_dword("height").unwrap_or(0);

    if stored_width != width || stored_height != height {
        set_registry_dword("width", width).expect("Failed to write width to registry");
        set_registry_dword("height", height).expect("Failed to write height to registry");

        if stored_width == 0 || stored_height == 0 {
            show_notification(LogLevel::INFO, &format!("Window size info ({}x{})", width, height), "window_size_init");
        } else {
            show_notification(LogLevel::INFO, &format!("Window size changed ({}x{})", width, height), "window_size_changed");
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

pub fn check_debug_folder_size() {
    if !get_config().debug && !get_config().debug_capture {
        return;
    }

    const WARNING_INTERVAL: u64 = 100; // 100MB

    let debug_folder = get_debug_folder();
    let current = get_folder_size(&debug_folder);

    if current < WARNING_INTERVAL {
        if get_registry_dword("debug_folder_last_warned_size").unwrap_or(0) != 0 {
            let _ = set_registry_dword("debug_folder_last_warned_size", 0);
        }
        return;
    }

    let last_warned = get_registry_dword("debug_folder_last_warned_size").unwrap_or(0) as u64;

    let current_level = current / WARNING_INTERVAL;
    let last_warned_level = last_warned / WARNING_INTERVAL;

    if current_level > last_warned_level {
        show_notification(LogLevel::WARN, &format!("PlayBridge folder size is {}MB!\nPlease be careful of high storage usage", current), "high_storage_usage");

        let _ = set_registry_dword("debug_folder_last_warned_size", current as u32);
    }
}
