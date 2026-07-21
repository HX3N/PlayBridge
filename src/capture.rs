use std::{
    cell::RefCell,
    env,
    fs::File,
    io::{Read, Write},
    net::{Shutdown, TcpStream},
    time::Duration,
};

use chrono::Local;
use fast_image_resize::{
    images::{Image, ImageRef},
    PixelType, ResizeAlg, ResizeOptions, Resizer,
};
use image::{codecs::png::PngEncoder, Rgba, RgbaImage};
use win_screenshot::prelude::{capture_window_ex, Area, Using};

use crate::config::*;
use crate::logging::{debug_log, get_debug_folder, LogLevel, LogMode};
use crate::notification::{display_notification, Notification};
use crate::window::GameWindow;

const LOOPBACK_IP: &str = "127.0.0.1";
const TCP_TIMEOUT_MS: u64 = 100;

pub fn send_black_frame_nc(port: u16) {
    transmit_pixels_nc(black_frame_pixels(), port);
}

pub fn screenshot(window: &GameWindow) {
    let Some(pixels) = capture_resized_pixels(window) else {
        display_notification(Notification::ScreenshotFailed);
        return;
    };

    let img = RgbaImage::from_raw(DISPLAY_WIDTH, DISPLAY_HEIGHT, pixels).expect("Failed to create RgbaImage");
    let filename = format!("Screenshot_{}.png", Local::now().format("%Y.%m.%d_%H.%M.%S.%3f"));
    let filepath = format!("{}/Desktop/{}", env::var("USERPROFILE").unwrap(), filename);
    img.write_with_encoder(PngEncoder::new(File::create(&filepath).unwrap())).unwrap();

    display_notification(Notification::Screenshot);
}

/// Verifies that taps/swipes committed by MAA land at the intended screen coordinates.
pub fn capture_touch_overlay(window: &GameWindow, path: &[(i32, i32)]) {
    Config::reload();
    if !config().touch_overlay || path.is_empty() {
        return;
    }

    let Some(pixels) = capture_resized_pixels(window) else {
        return;
    };
    let mut img = RgbaImage::from_raw(DISPLAY_WIDTH, DISPLAY_HEIGHT, pixels).expect("Failed to create RgbaImage");

    let last = (path.len() - 1).max(1) as f32;

    for (i, &(x, y)) in path.iter().enumerate() {
        let t = i as f32 / last;
        let r = (255.0 * (1.0 - t)) as u8;
        let b = (255.0 * t) as u8;
        if i == 0 || i == path.len() - 1 {
            draw_x(&mut img, x, y, Rgba([r, 0, b, 255]));
        } else {
            draw_filled_circle(&mut img, x, y, Rgba([r, 0, b, 255]));
        }
    }

    let filename = format!("TouchOverlay_{}.png", Local::now().format("%Y.%m.%d_%H.%M.%S.%3f"));
    let capture_folder = get_debug_folder().join("PlayBridge");
    let _ = std::fs::create_dir_all(&capture_folder);
    cleanup_old_captures(&capture_folder);

    let filepath = capture_folder.join(&filename);
    img.write_with_encoder(PngEncoder::new(File::create(&filepath).unwrap())).unwrap();

    debug_log(LogLevel::Info, LogMode::Nested, &format!("TouchOverlay: {}", filepath.display()));
}

fn capture_resized_pixels(window: &GameWindow) -> Option<Vec<u8>> {
    window.restore();

    let hwnd = window.hwnd;

    // DWM skips the same right/bottom ~3px as WGC (see crop_region in wgc.rs).
    // PrintWindow writes that edge as black in a full-size buffer, so the scale holds and only a thin black edge remains.
    let buf = capture_window_ex(hwnd.0 as isize, Using::PrintWindow, Area::ClientOnly, None, None).ok()?;

    resize_to_display(&buf.pixels, buf.width, buf.height)
}

// Reused so fast_image_resize keeps its scratch buffers instead of reallocating per resize.
// resize_to_display only runs on one thread per process.
thread_local! {
    static RESIZER: RefCell<Resizer> = RefCell::new(Resizer::new());
}

pub fn resize_to_display(pixels: &[u8], w: u32, h: u32) -> Option<Vec<u8>> {
    let src_image = ImageRef::new(w, h, pixels, PixelType::U8x4).ok()?;
    let mut dst_image = Image::new(DISPLAY_WIDTH, DISPLAY_HEIGHT, PixelType::U8x4);

    let options = ResizeOptions::new().resize_alg(ResizeAlg::Convolution(fast_image_resize::FilterType::Lanczos3));
    RESIZER.with(|r| r.borrow_mut().resize(&src_image, &mut dst_image, &options)).ok()?;

    Some(dst_image.into_vec())
}

pub fn black_frame_pixels() -> Vec<u8> {
    vec![0u8; (DISPLAY_WIDTH * DISPLAY_HEIGHT * 4) as usize]
}

pub fn transmit_pixels_nc(mut pixels: Vec<u8>, port: u16) {
    let mut stream = match TcpStream::connect((LOOPBACK_IP, port)) {
        Ok(s) => s,
        Err(e) => {
            debug_log(LogLevel::Error, LogMode::Nested, &format!("Socket: connect failed: {}", e));
            return;
        }
    };

    // MAA's frame validation requires the last alpha byte to be 0xFF.
    if let Some(last) = pixels.last_mut() {
        *last = 0xFF;
    }

    // Protocol: [Width:4][Height:4][Format:4][RGBA Data] / Format=1 (RGBA_8888)
    let mut header = [0u8; 12];
    header[0..4].copy_from_slice(&DISPLAY_WIDTH.to_le_bytes());
    header[4..8].copy_from_slice(&DISPLAY_HEIGHT.to_le_bytes());
    header[8..12].copy_from_slice(&1u32.to_le_bytes());

    if let Err(e) = stream.write_all(&header).and_then(|()| stream.write_all(&pixels)) {
        debug_log(LogLevel::Error, LogMode::Nested, &format!("Socket: send failed: {}", e));
        return;
    }

    // Half-close, then linger on a short read so MAA drains the frame before process exit resets the socket.
    let _ = stream.shutdown(Shutdown::Write);
    let _ = stream.set_read_timeout(Some(Duration::from_millis(TCP_TIMEOUT_MS)));
    let mut dump = [0; 1];
    let _ = stream.read(&mut dump);
}

fn draw_filled_circle(img: &mut RgbaImage, cx: i32, cy: i32, color: Rgba<u8>) {
    let radius = 1;
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

fn draw_x(img: &mut RgbaImage, cx: i32, cy: i32, color: Rgba<u8>) {
    let size = 4;
    let (w, h) = (img.width() as i32, img.height() as i32);
    for i in -size..=size {
        for (dx, dy) in [(i, i), (i, -i)] {
            for ox in -1..=1 {
                for oy in -1..=1 {
                    let nx = cx + dx + ox;
                    let ny = cy + dy + oy;
                    if nx >= 0 && nx < w && ny >= 0 && ny < h {
                        img.put_pixel(nx as u32, ny as u32, color);
                    }
                }
            }
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
            name.starts_with("TouchOverlay_") && name.ends_with(".png")
        })
        .collect();

    if files.len() < MAX_TOUCH_OVERLAY_FILES {
        return;
    }

    files.sort_by_key(|e| e.metadata().and_then(|m| m.modified()).unwrap_or(std::time::SystemTime::UNIX_EPOCH));

    let num_to_remove = files.len().saturating_sub(MAX_TOUCH_OVERLAY_FILES - 1);

    for entry in files.iter().take(num_to_remove) {
        let _ = std::fs::remove_file(entry.path());
    }
}
