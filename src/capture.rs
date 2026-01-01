use std::{
    env,
    fs::File,
    io::{stdout, Read, Write},
    net::{Shutdown, TcpStream},
    thread,
    time::Duration,
};

use chrono::Local;
use image::{codecs::png::PngEncoder, DynamicImage, ImageBuffer, Rgba, RgbaImage};

use crate::config::*;
use crate::logging::{debug_log, get_debug_folder, LogLevel, LogMode};
use crate::notification::{display_notification, get_value, set_value, Notification};
use crate::window::{find_game_window, get_window_info, restore_if_minimized};

use win_screenshot::prelude::{capture_window_ex, Area, Using};
use windows::Win32::UI::HiDpi::{GetWindowDpiAwarenessContext, SetThreadDpiAwarenessContext};

use fast_image_resize::{images::Image, PixelType, ResizeAlg, ResizeOptions, Resizer};

const MAX_WINDOW_SIZE: (u32, u32) = ((DISPLAY_WIDTH as f32 * 1.5) as u32, (DISPLAY_HEIGHT as f32 * 1.5) as u32);

const LOOPBACK_IP: &str = "127.0.0.1";
const BENCHMARK_DELAY_MS: u64 = 50;
const TCP_TIMEOUT_MS: u64 = 100;

pub fn send_capture() {
    let img = if check_benchmark_mode() {
        // Added delay to ensure RawByNc is selected
        thread::sleep(Duration::from_millis(BENCHMARK_DELAY_MS));
        debug_log(LogLevel::Info, LogMode::Nested, "Benchmark: Sent Black Frame (Encode)");
        DynamicImage::ImageRgba8(ImageBuffer::from_pixel(DISPLAY_WIDTH, DISPLAY_HEIGHT, Rgba([0, 0, 0, 255])))
    } else {
        match capture_resized_pixels() {
            Some(pixels) => DynamicImage::ImageRgba8(RgbaImage::from_raw(DISPLAY_WIDTH, DISPLAY_HEIGHT, pixels).unwrap()),
            None => {
                debug_log(LogLevel::Info, LogMode::Nested, "Window not found: Sent Black Frame (Encode)");
                DynamicImage::ImageRgba8(ImageBuffer::from_pixel(DISPLAY_WIDTH, DISPLAY_HEIGHT, Rgba([0, 0, 0, 255])))
            }
        }
    };

    img.write_with_encoder(PngEncoder::new(&mut stdout().lock())).unwrap();
}

pub fn send_capture_nc(port: u16) {
    let black_frame = || -> Vec<u8> { vec![0u8; (DISPLAY_WIDTH * DISPLAY_HEIGHT * 4) as usize] };

    let pixels = if check_benchmark_mode() {
        debug_log(LogLevel::Info, LogMode::Nested, &format!("RawByNc: Connecting to {}:{}", LOOPBACK_IP, port));
        debug_log(LogLevel::Info, LogMode::Nested, "Benchmark: Sent Black Frame (RawByNc)");
        black_frame()
    } else {
        match capture_resized_pixels() {
            Some(p) => p,
            None => {
                debug_log(LogLevel::Info, LogMode::Nested, "Window not found: Sent Black Frame (RawByNc)");
                black_frame()
            }
        }
    };

    let mut stream = match TcpStream::connect((LOOPBACK_IP, port)) {
        Ok(s) => s,
        Err(e) => {
            debug_log(LogLevel::Error, LogMode::Nested, &format!("Socket Connect Failed: {}", e));
            return;
        }
    };

    // Protocol: [Width:4][Height:4][Format:4][RGBA Data] / Format=1 (RGBA_8888)
    let mut buffer = Vec::with_capacity(12 + pixels.len());
    buffer.extend_from_slice(&DISPLAY_WIDTH.to_le_bytes());
    buffer.extend_from_slice(&DISPLAY_HEIGHT.to_le_bytes());
    buffer.extend_from_slice(&1u32.to_le_bytes());
    buffer.extend_from_slice(&pixels);

    // Ensure last alpha byte is 0xFF for MAA validation
    if let Some(last) = buffer.last_mut() {
        *last = 0xFF;
    }

    if let Err(e) = stream.write_all(&buffer) {
        debug_log(LogLevel::Error, LogMode::Nested, &format!("Socket Send Failed: {}", e));
        return;
    }

    // Send FIN and wait for MAA to close the connection before process exit
    if let Err(e) = stream.shutdown(Shutdown::Write) {
        debug_log(LogLevel::Warn, LogMode::Nested, &format!("Socket Shutdown Failed: {}", e));
    }

    let _ = stream.set_read_timeout(Some(Duration::from_millis(TCP_TIMEOUT_MS)));
    let mut dump = [0; 1];
    if let Err(e) = stream.read(&mut dump) {
        debug_log(LogLevel::Warn, LogMode::Nested, &format!("Socket Wait Failed: {}", e));
    }
}

pub fn screenshot() {
    let Some(img) = capture_window_png() else {
        display_notification(Notification::ScreenshotFailed);
        return;
    };

    let filename = format!("Screenshot_{}.png", Local::now().format("%Y.%m.%d_%H.%M.%S.%3f"));
    let filepath = format!("{}/Desktop/{}", env::var("USERPROFILE").unwrap(), filename);
    img.write_with_encoder(PngEncoder::new(File::create(&filepath).unwrap())).unwrap();

    display_notification(Notification::Screenshot);
}

pub fn debug_capture(x: i32, y: i32, end_point: Option<(i32, i32)>) {
    if !config().debug_capture {
        return;
    }

    let Some(img) = capture_window_png() else {
        return;
    };
    let mut img = img.to_rgba8();

    let draw_point = |img: &mut RgbaImage, (x, y), color| {
        draw_filled_circle(img, x, y, 6, Rgba([255, 255, 255, 255]));
        draw_filled_circle(img, x, y, 5, color);
    };

    draw_point(&mut img, (x, y), Rgba([255, 0, 0, 255]));

    if let Some((x2, y2)) = end_point {
        draw_line(&mut img, x, y, x2, y2, Rgba([0, 255, 0, 255]));
        draw_point(&mut img, (x2, y2), Rgba([0, 0, 255, 255]));
    }

    let filename = format!("Debug_{}.png", Local::now().format("%Y.%m.%d_%H.%M.%S.%3f"));
    let capture_folder = get_debug_folder().join("PlayBridge");
    let _ = std::fs::create_dir_all(&capture_folder);
    cleanup_old_captures(&capture_folder);

    let filepath = capture_folder.join(&filename);

    img.write_with_encoder(PngEncoder::new(File::create(&filepath).unwrap())).unwrap();

    debug_log(LogLevel::Info, LogMode::Nested, &format!("Debug capture: {}", filepath.display()));
}

fn capture_resized_pixels() -> Option<Vec<u8>> {
    let hwnd = find_game_window()?;
    restore_if_minimized(hwnd);
    let (hwnd, log_w, log_h) = get_window_info();

    // Handle mixed DPI settings properly in multiple-monitor setups (ex. main 125%, sub 100%)
    unsafe { _ = SetThreadDpiAwarenessContext(GetWindowDpiAwarenessContext(hwnd)) };

    let buf = capture_window_ex(hwnd.0 as isize, Using::PrintWindow, Area::ClientOnly, None, None).unwrap();

    // physical size (actual captured pixels)
    let phys_w = buf.width;
    let phys_h = buf.height;
    validate_window_size(log_w, log_h, phys_w, phys_h);

    let src_image = Image::from_vec_u8(phys_w, phys_h, buf.pixels, PixelType::U8x4).unwrap();
    let mut dst_image = Image::new(DISPLAY_WIDTH, DISPLAY_HEIGHT, PixelType::U8x4);

    let mut resizer = Resizer::new();
    let options = ResizeOptions::new().resize_alg(ResizeAlg::Convolution(fast_image_resize::FilterType::Lanczos3));
    resizer.resize(&src_image, &mut dst_image, &options).unwrap();

    Some(dst_image.into_vec())
}

fn capture_window_png() -> Option<DynamicImage> {
    let pixels = capture_resized_pixels()?;
    Some(DynamicImage::ImageRgba8(RgbaImage::from_raw(DISPLAY_WIDTH, DISPLAY_HEIGHT, pixels).unwrap()))
}

fn validate_window_size(log_w: i32, log_h: i32, phys_w: u32, phys_h: u32) {
    let height_ratio = log_h as f32 / (log_w as f32 / 16.0);

    if (height_ratio - 9.0).abs() > 0.1 {
        display_notification(Notification::WindowWrongRatio(height_ratio));
        return;
    }

    if phys_w < DISPLAY_WIDTH || phys_h < DISPLAY_HEIGHT {
        display_notification(Notification::WindowTooSmall(phys_w, phys_h));
        return;
    }

    if phys_w > MAX_WINDOW_SIZE.0 || phys_h > MAX_WINDOW_SIZE.1 {
        display_notification(Notification::WindowTooLarge(phys_w, phys_h));
        return;
    }

    let [stored_w, stored_h] = [get_value("width"), get_value("height")];

    if stored_w != phys_w || stored_h != phys_h {
        set_value("width", phys_w);
        set_value("height", phys_h);

        let scale = if log_w > 0 { (phys_w as f32 / log_w as f32 * 100.0).round() as u32 } else { 100 };
        debug_log(
            LogLevel::Info,
            LogMode::Nested,
            &format!("Physical: {}x{} / Logical: {}x{} / Scale: {}%", phys_w, phys_h, log_w, log_h, scale),
        );

        display_notification(Notification::WindowChanged(phys_w, phys_h));
    }
}

fn draw_filled_circle(img: &mut RgbaImage, cx: i32, cy: i32, radius: i32, color: Rgba<u8>) {
    let (w, h) = (img.width() as i32, img.height() as i32);
    for dy in -radius..=radius {
        for dx in -radius..=radius {
            if dx * dx + dy * dy <= radius * radius {
                let nx = cx + dx;
                let ny = cy + dy;
                if nx >= 0 && nx < w && ny >= 0 && ny < h {
                    img.put_pixel(nx as u32, ny as u32, color);
                }
            }
        }
    }
}

fn draw_line(img: &mut RgbaImage, x0: i32, y0: i32, x1: i32, y1: i32, color: Rgba<u8>) {
    let (w, h) = (img.width() as i32, img.height() as i32);
    let mut x = x0;
    let mut y = y0;
    let dx = (x1 - x0).abs();
    let dy = -(y1 - y0).abs();
    let sx = if x0 < x1 { 1 } else { -1 };
    let sy = if y0 < y1 { 1 } else { -1 };
    let mut err = dx + dy;

    loop {
        if x >= 0 && x < w && y >= 0 && y < h {
            img.put_pixel(x as u32, y as u32, color);
        }
        if x == x1 && y == y1 {
            break;
        }
        let e2 = 2 * err;
        if e2 >= dy {
            if x == x1 {
                break;
            }
            err += dy;
            x += sx;
        }
        if e2 <= dx {
            if y == y1 {
                break;
            }
            err += dx;
            y += sy;
        }
    }
}

fn cleanup_old_captures(folder: &std::path::Path) {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return;
    };

    let mut files: Vec<_> = entries
        .filter_map(|e| e.ok())
        .filter(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            name.starts_with("Debug_") && name.ends_with(".png")
        })
        .collect();

    if files.len() < MAX_DEBUG_CAPTURE_FILES {
        return;
    }

    files.sort_by_key(|e| e.metadata().and_then(|m| m.modified()).unwrap_or(std::time::SystemTime::UNIX_EPOCH));

    let num_to_remove = files.len().saturating_sub(MAX_DEBUG_CAPTURE_FILES - 1);

    if num_to_remove == 0 {
        return;
    }

    for entry in files.iter().take(num_to_remove) {
        std::fs::remove_file(entry.path()).ok();
    }
}
