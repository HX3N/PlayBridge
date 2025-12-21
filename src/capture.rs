use std::{env, fs::File, io::stdout};

use chrono::Local;
use image::{codecs::png::PngEncoder, imageops::FilterType::Lanczos3, DynamicImage, Rgb, RgbaImage};
use imageproc::drawing::{draw_filled_circle_mut, draw_line_segment_mut};

use crate::config::*;
use crate::logging::{debug_log, get_debug_folder, LogLevel, LogMode};
use crate::notification::{display_notification, get_value, set_value, Notification};
use crate::window::{get_hwnd, get_info, restore_if_minimized};

use win_screenshot::prelude::{capture_window_ex, Area, Using};
use windows::Win32::UI::HiDpi::{GetWindowDpiAwarenessContext, SetThreadDpiAwarenessContext};

pub fn send_capture() {
    let Some(hwnd) = get_hwnd() else {
        debug_log(LogLevel::Info, LogMode::Nested, "Window not found, sending black image");

        let black_pixels = vec![0u8; (DISPLAY_WIDTH * DISPLAY_HEIGHT * 3) as usize];
        let img = DynamicImage::ImageRgb8(image::RgbImage::from_raw(DISPLAY_WIDTH, DISPLAY_HEIGHT, black_pixels).unwrap());
        img.write_with_encoder(PngEncoder::new(&mut stdout().lock())).unwrap();
        return;
    };

    restore_if_minimized(hwnd);

    let (hwnd, log_w, log_h) = get_info();

    let img = capture_window(hwnd, log_w, log_h, true);
    img.write_with_encoder(PngEncoder::new(&mut stdout().lock())).unwrap();
}

pub fn capture() -> DynamicImage {
    let (hwnd, log_w, log_h) = get_info();
    capture_window(hwnd, log_w, log_h, false)
}

fn capture_window(hwnd: windows::Win32::Foundation::HWND, log_w: i32, log_h: i32, validate_size: bool) -> DynamicImage {
    // Handle mixed DPI settings properly in multiple-monitor setups (ex. main 125%, sub 100%)
    unsafe { _ = SetThreadDpiAwarenessContext(GetWindowDpiAwarenessContext(hwnd)) };

    let buf = capture_window_ex(hwnd.0 as isize, Using::PrintWindow, Area::ClientOnly, None, None).unwrap();

    // physical size (actual captured pixels)
    let phys_w = buf.width;
    let phys_h = buf.height;

    if validate_size {
        validate_window_size(log_w, log_h, phys_w, phys_h);
    }

    let img = DynamicImage::ImageRgba8(RgbaImage::from_raw(phys_w, phys_h, buf.pixels).unwrap());

    img.resize(DISPLAY_WIDTH, DISPLAY_HEIGHT, Lanczos3)
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

    if phys_w > (DISPLAY_WIDTH as f32 * 1.5) as u32 || phys_h > (DISPLAY_HEIGHT as f32 * 1.5) as u32 {
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

        if stored_w == 0 || stored_h == 0 {
            display_notification(Notification::WindowInfo(phys_w, phys_h));
        } else {
            display_notification(Notification::WindowChanged(stored_w, stored_h, phys_w, phys_h));
        }
    }
}

pub fn screenshot() {
    if get_hwnd().is_none() {
        display_notification(Notification::ScreenshotFailed);
        return;
    }

    let img = capture();
    let filename = format!("Screenshot_{}.png", Local::now().format("%Y.%m.%d_%H.%M.%S.%3f"));
    let filepath = format!("{}\\Desktop\\{}", env::var("USERPROFILE").unwrap(), filename);
    img.write_with_encoder(PngEncoder::new(File::create(&filepath).unwrap())).unwrap();

    display_notification(Notification::Screenshot);
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
    let capture_folder = get_debug_folder().join("PlayBridge");
    let _ = std::fs::create_dir_all(&capture_folder);
    let filepath = capture_folder.join(&filename);

    let dynamic_img = DynamicImage::ImageRgb8(img_rgb);
    dynamic_img.write_with_encoder(PngEncoder::new(File::create(&filepath).unwrap())).unwrap();

    debug_log(LogLevel::Info, LogMode::Nested, &filepath.display().to_string());
}
