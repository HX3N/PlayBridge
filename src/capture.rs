use std::{env, fs::File, io::stdout, io::Write, net::TcpStream};

use chrono::Local;
use image::{codecs::png::PngEncoder, DynamicImage, Rgb, RgbaImage};
use imageproc::drawing::{draw_filled_circle_mut, draw_line_segment_mut};

use crate::config::*;
use crate::logging::{debug_log, get_debug_folder, LogLevel, LogMode};
use crate::notification::{display_notification, get_value, set_value, Notification};
use crate::window::{find_game_window, get_window_info, restore_if_minimized};

use win_screenshot::prelude::{capture_window_ex, Area, Using};
use windows::Win32::UI::HiDpi::{GetWindowDpiAwarenessContext, SetThreadDpiAwarenessContext};

use fast_image_resize::{images::Image, PixelType, ResizeAlg, ResizeOptions, Resizer};

const MAX_WINDOW_SIZE: (u32, u32) = ((DISPLAY_WIDTH as f32 * 1.5) as u32, (DISPLAY_HEIGHT as f32 * 1.5) as u32);

const LOOPBACK_IP: &str = "127.0.0.1";

fn try_capture_game_window() -> Option<DynamicImage> {
    let hwnd = find_game_window()?;
    restore_if_minimized(hwnd);
    let (hwnd, log_w, log_h) = get_window_info();
    Some(capture_window(hwnd, log_w, log_h, true))
}

pub fn send_capture() {
    if check_benchmark_mode() {
        send_black_frame_png();
        debug_log(LogLevel::Info, LogMode::Nested, "Benchmark: Sent Black Frame (Encode)");
        return;
    }

    let Some(img) = try_capture_game_window() else {
        send_black_frame_png();
        debug_log(LogLevel::Info, LogMode::Nested, "Window not found: Sent Black Frame (Encode)");
        return;
    };

    img.write_with_encoder(PngEncoder::new(&mut stdout().lock())).unwrap();
}

pub fn send_capture_nc(port: u16) {
    // During MAA's "fastest way to screencap" test, if FORCE_ENCODE is set, delay RawByNc to force Encode selection
    if check_benchmark_mode() {
        debug_log(LogLevel::Info, LogMode::Nested, &format!("RawByNc: Connecting to {}:{}", LOOPBACK_IP, port));

        if config().force_encode {
            std::thread::sleep(std::time::Duration::from_millis(100));
            debug_log(LogLevel::Warn, LogMode::Nested, "FORCE_ENCODE enabled: Recommended to use RawByNc");
        }

        send_black_frame_nc(port);
        debug_log(LogLevel::Info, LogMode::Nested, "Benchmark: Sent Black Frame (RawByNc)");
        return;
    }

    let Some(img) = try_capture_game_window() else {
        send_black_frame_nc(port);
        debug_log(LogLevel::Info, LogMode::Nested, "Window not found: Sent Black Frame (RawByNc)");
        return;
    };

    send_raw_image_nc(img, port);
}

fn send_raw_image_nc(img: DynamicImage, port: u16) {
    let width = img.width();
    let height = img.height();
    let pixels = img.to_rgba8().into_raw();

    let mut stream = match TcpStream::connect((LOOPBACK_IP, port)) {
        Ok(s) => s,
        Err(e) => {
            debug_log(LogLevel::Error, LogMode::Nested, &format!("Socket Connect Failed: {}", e));
            return;
        }
    };

    // Protocol: [Width:4][Height:4][Format:4][RGBA Data] / Format=1 (RGBA_8888)
    let mut buffer = Vec::with_capacity(12 + pixels.len());
    buffer.extend_from_slice(&width.to_le_bytes());
    buffer.extend_from_slice(&height.to_le_bytes());
    buffer.extend_from_slice(&1u32.to_le_bytes());
    buffer.extend_from_slice(&pixels);

    // Ensure last alpha byte is 0xFF for MAA validation
    if let Some(last) = buffer.last_mut() {
        *last = 0xFF;
    }

    if let Err(e) = stream.write_all(&buffer) {
        debug_log(LogLevel::Error, LogMode::Nested, &format!("Socket Send Failed: {}", e));
    }
}

fn send_black_frame_png() {
    use image::{ImageBuffer, Rgba};
    let img: ImageBuffer<Rgba<u8>, Vec<u8>> = ImageBuffer::from_pixel(DISPLAY_WIDTH, DISPLAY_HEIGHT, Rgba([0, 0, 0, 255]));
    img.write_with_encoder(PngEncoder::new(&mut stdout().lock())).unwrap();
}

fn send_black_frame_nc(port: u16) {
    match TcpStream::connect((LOOPBACK_IP, port)) {
        Ok(mut stream) => {
            let width = DISPLAY_WIDTH;
            let height = DISPLAY_HEIGHT;
            let buffer_size = (width * height * 4) as usize;

            let mut buffer = Vec::with_capacity(12 + buffer_size);
            buffer.extend_from_slice(&width.to_le_bytes());
            buffer.extend_from_slice(&height.to_le_bytes());
            buffer.extend_from_slice(&1u32.to_le_bytes());

            let pixel_count = (width * height) as usize;
            let mut pixels = Vec::with_capacity(pixel_count * 4);
            for _ in 0..pixel_count {
                pixels.extend_from_slice(&[0, 0, 0, 255]);
            }
            buffer.extend_from_slice(&pixels);

            stream.write_all(&buffer).unwrap();
        }
        Err(e) => {
            debug_log(LogLevel::Error, LogMode::Nested, &format!("RawByNc Black Frame Socket Error: {}", e));
        }
    }
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

    let src_image = Image::from_vec_u8(phys_w, phys_h, buf.pixels, PixelType::U8x4).unwrap();
    let mut dst_image = Image::new(DISPLAY_WIDTH, DISPLAY_HEIGHT, PixelType::U8x4);

    let mut resizer = Resizer::new();
    let options = ResizeOptions::new().resize_alg(ResizeAlg::Convolution(fast_image_resize::FilterType::Lanczos3));

    resizer.resize(&src_image, &mut dst_image, &options).unwrap();

    DynamicImage::ImageRgba8(RgbaImage::from_raw(DISPLAY_WIDTH, DISPLAY_HEIGHT, dst_image.into_vec()).unwrap())
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

pub fn screenshot() {
    let Some(img) = try_capture_game_window() else {
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

    let Some(img) = try_capture_game_window() else {
        return;
    };
    let mut img_rgb = img.to_rgb8();

    let draw_point = |img: &mut _, (x, y), color| {
        draw_filled_circle_mut(img, (x, y), 6, Rgb([255, 255, 255]));
        draw_filled_circle_mut(img, (x, y), 5, color);
    };

    draw_point(&mut img_rgb, (x, y), Rgb([255, 0, 0]));

    if let Some((x2, y2)) = end_point {
        draw_line_segment_mut(&mut img_rgb, (x as f32, y as f32), (x2 as f32, y2 as f32), Rgb([0, 255, 0]));
        draw_point(&mut img_rgb, (x2, y2), Rgb([0, 0, 255]));
    }

    let filename = format!("Debug_{}.png", Local::now().format("%Y.%m.%d_%H.%M.%S.%3f"));
    let capture_folder = get_debug_folder().join("PlayBridge");
    let _ = std::fs::create_dir_all(&capture_folder);
    let filepath = capture_folder.join(&filename);

    let dynamic_img = DynamicImage::ImageRgb8(img_rgb);
    dynamic_img.write_with_encoder(PngEncoder::new(File::create(&filepath).unwrap())).unwrap();

    debug_log(LogLevel::Info, LogMode::Nested, &filepath.display().to_string());
}
