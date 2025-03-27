#![allow(non_snake_case)]

mod config;
mod notification;

use crate::notification::*;
use chrono::Local;
use config::{config_mode, CONFIG};
use image::{
    codecs::png::PngEncoder,
    imageops::{crop_imm, FilterType},
    DynamicImage, RgbaImage,
};
use std::{env, fs::File, io::stdout, mem, thread, time::Duration};
use windows::{
    core::*,
    Win32::{Foundation::*, Graphics::Gdi::*, Storage::Xps::*, UI::HiDpi::*, UI::WindowsAndMessaging::*},
};

const CLASS: PCWSTR = w!("CROSVM_1"); // Note: Warning. May cause problems in the future
const POLLING_RATE: i32 = 1000 / 500;

fn main() {
    let args: Vec<String> = env::args().collect();
    if args[0].to_lowercase().contains("-config") {
        config_mode();
        return;
    }

    unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2).unwrap() };
    let command = parse_command(&args);
    if get_hwnd().is_none() {
        start_arknights();
    }

    execute_command(command);
}

enum Command {
    Empty,
    Connect,
    GetPropVersionRelease,
    StartActivity { intent: String },
    DumpsysWindowDisplays,
    GetUUID,
    InputTap { x: i32, y: i32 },
    InputText { text: String },
    InputSwipe { x1: i32, y1: i32, x2: i32, y2: i32, duration: i32 },
    InputKeyEvent { keycode: i32 },
    ExecOutScreencap,
    ForceStop,
    IgnoreCommand,
    Unknown(String),
}

fn parse_command(args: &[String]) -> Command {
    if args.len() <= 1 {
        return Command::Empty;
    }
    let full_command = args.join(" ");
    match full_command.as_str() {
        c if c.contains("connect") => Command::Connect,
        c if c.contains("getprop ro.build.version.release") => Command::GetPropVersionRelease,
        c if c.contains("am start -n") => Command::StartActivity { intent: args[7].clone() },
        c if c.contains("input tap") => Command::InputTap { x: args[6].parse().unwrap(), y: args[7].parse().unwrap() },
        c if c.contains("input text") => Command::InputText { text: args[6..].join(" ") },
        c if c.contains("input swipe") => Command::InputSwipe {
            x1: args[6].parse().unwrap(),
            y1: args[7].parse().unwrap(),
            x2: args[8].parse().unwrap(),
            y2: args[9].parse().unwrap(),
            duration: args[10].parse().unwrap(),
        },
        c if c.contains("input keyevent 111") => Command::InputKeyEvent { keycode: 0x01 },
        c if c.contains("dumpsys window displays") || c.contains("wm size") => Command::DumpsysWindowDisplays,
        c if c.contains("exec-out screencap -p") => Command::ExecOutScreencap,
        c if c.contains("am force-stop") || c.contains("input keyevent HOME") => Command::ForceStop,
        c if c.contains("settings get secure android_id") => Command::GetUUID,
        c if c.contains("cat /proc/net/arp")
            || c.contains("exec-out screencap | nc -w 3")
            || c.contains("exec-out screencap | gzip -1")
            || c.contains("start-server")
            || c.contains("kill-server")
            || c.contains("devices") =>
        {
            Command::IgnoreCommand
        }
        _ => Command::Unknown(full_command),
    }
}

fn execute_command(command: Command) -> bool {
    match command {
        Command::Empty => {
            let filename = format!("Screenshot_{}.png", Local::now().format("%Y.%m.%d_%H.%M.%S.%3f"));
            let filepath = format!("{}\\Desktop\\{}", env::var("USERPROFILE").unwrap(), filename);
            capture().write_with_encoder(PngEncoder::new(File::create(&filepath).unwrap())).unwrap();

            show_notification(INFO, &format!("Screenshot saved to desktop!\n{}", filename), "screenshot_saved");

            true
        }
        // ========= MAA needs this output =========
        Command::Connect => {
            println!("connected to Google Play Games Beta");
            true
        }
        Command::GetPropVersionRelease => {
            println!("14");
            true
        }
        Command::StartActivity { intent } => {
            println!("Starting: Intent {{ cmp={} }}", intent);
            println!("Warning: Activity not started, intent has been delivered to currently running top-most instance.");
            true
        }
        Command::DumpsysWindowDisplays => {
            println!("{} {}", CONFIG.display_width as i32, CONFIG.display_height as i32);
            true
        }
        Command::GetUUID => {
            println!("GooglePlayGames");
            true
        }
        // =========================================
        Command::InputTap { x, y } => {
            input_tap(x, y);
            true
        }
        Command::InputText { text } => {
            input_text(&text);
            true
        }
        Command::InputSwipe { x1, y1, x2, y2, duration } => {
            input_swipe(x1, y1, x2, y2, duration);
            true
        }
        Command::InputKeyEvent { keycode } => {
            input_keyevent(keycode);
            true
        }
        Command::ExecOutScreencap => {
            let image = capture();
            image.write_with_encoder(PngEncoder::new(&mut stdout().lock())).unwrap();
            true
        }
        Command::ForceStop => {
            terminate();
            show_notification(INFO, "Arknights shutdown", "shutdown_arknights");
            true
        }
        Command::IgnoreCommand => true,
        Command::Unknown(cmd) => {
            println!("[PlayBridge] {} (Unknown command)", cmd);
            show_notification(ERR, &format!("Unknown command!\n{}", cmd), "unknown_command");

            false
        }
    }
}

fn start_arknights() {
    let _ = open::that(format!("googleplaygames://launch/?id={}", CONFIG.package));

    let success = (0..30).any(|_| {
        thread::sleep(Duration::from_secs(1));
        get_hwnd().is_some()
    });

    if success {
        show_notification(INFO, "Arknights launched", "start_arknights");
    } else {
        show_notification(ERR, "Failed to launch Arknights!", "start_arknights_failed");
    }
}

fn post_message(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) {
    unsafe {
        let _ = PostMessageA(Some(hwnd), msg, wparam, lparam);
    };
}

fn get_hwnd() -> Option<HWND> {
    let title: Vec<u16> = CONFIG.title.encode_utf16().chain(Some(0)).collect();
    if let Ok(hwnd) = unsafe { FindWindowW(CLASS, PCWSTR(title.as_ptr())) } {
        return Some(hwnd);
    }

    let mut result: Option<HWND> = None;
    unsafe extern "system" fn enum_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let result_ref = &mut *(lparam.0 as *mut Option<HWND>);

        let mut class_buf = [0u16; 256];
        let class_len = GetClassNameW(hwnd, &mut class_buf);
        if class_len == 0 {
            return BOOL(1);
        }
        let class_name = String::from_utf16_lossy(&class_buf[..class_len as usize]);
        if class_name != CLASS.to_string().unwrap() {
            return BOOL(1);
        }

        let mut title_buf = [0u16; 512];
        let len = GetWindowTextW(hwnd, &mut title_buf);
        if len > 0 {
            let title = String::from_utf16_lossy(&title_buf[..len as usize]);
            if title.starts_with(&CONFIG.title) {
                *result_ref = Some(hwnd);
                return BOOL(0);
            }
        }
        BOOL(1)
    }

    unsafe {
        let _ = EnumWindows(Some(enum_proc), LPARAM(&mut result as *mut _ as isize));
    }
    result
}

fn get_gpg_info() -> (HWND, i32, i32) {
    let hwnd = get_hwnd().unwrap();
    let mut client_rect = RECT::default();
    unsafe {
        let _ = GetClientRect(hwnd, &mut client_rect);
    }
    (hwnd, (client_rect.right - client_rect.left) as i32, (client_rect.bottom - client_rect.top) as i32)
}

fn get_relative_point(x: i32, y: i32, w: i32, h: i32) -> isize {
    let nx = (x as f32 / CONFIG.display_width * w as f32).ceil() as isize;
    let ny = (y as f32 / CONFIG.display_height * h as f32).ceil() as isize;
    ny << 16 | nx
}

fn input_tap(x: i32, y: i32) {
    let (hwnd, w, h) = get_gpg_info();
    let pos = get_relative_point(x, y, w, h);
    post_message(hwnd, WM_LBUTTONDOWN, WPARAM(1), LPARAM(pos));
    post_message(hwnd, WM_LBUTTONUP, WPARAM(1), LPARAM(pos));
}

fn input_text(text: &str) {
    let hwnd = get_hwnd().unwrap();
    for ch in text.chars() {
        post_message(hwnd, WM_CHAR, WPARAM(ch as usize), LPARAM(0));
        thread::sleep(Duration::from_millis(50));
    }
}

fn ease_out(t: f32) -> f32 {
    if t < 0.8 {
        t / 0.8
    } else {
        let start = 1.0;
        let v0 = 1.0 / 0.8;
        start + v0 * (t - 0.8) - v0 * (t - 0.8).powi(2) / (2.0 * 0.2)
    }
}

fn input_swipe(x1: i32, y1: i32, x2: i32, y2: i32, duration: i32) {
    let (hwnd, w, h) = get_gpg_info();

    let dx = (x2 - x1) as f32;
    let dy = (y2 - y1) as f32;
    let steps = ((duration as f32 / CONFIG.swipe_speed) / POLLING_RATE as f32).ceil() as i32;

    let pos_down = get_relative_point(x1, y1, w, h);
    post_message(hwnd, WM_LBUTTONDOWN, WPARAM(1), LPARAM(pos_down));

    for cnt in 0..=steps {
        let progress = cnt as f32 / steps as f32;
        let eased = ease_out(progress).min(1.0);

        let (nx, ny) = if cnt == steps { (x2, y2) } else { (x1 + (dx * eased).round() as i32, y1 + (dy * eased).round() as i32) };

        let pos = get_relative_point(nx, ny, w, h);
        post_message(hwnd, WM_MOUSEMOVE, WPARAM(1), LPARAM(pos));
        spin_sleep::sleep(Duration::new(0, POLLING_RATE as u32 * 1_000_000));
    }

    let pos_up = get_relative_point(x2, y2, w, h);
    spin_sleep::sleep(Duration::new(0, (POLLING_RATE * 5) as u32 * 1_000_000));
    post_message(hwnd, WM_LBUTTONDOWN, WPARAM(1), LPARAM(pos_up));
    post_message(hwnd, WM_LBUTTONUP, WPARAM(1), LPARAM(pos_up));
}

fn input_keyevent(keycode: i32) {
    let hwnd = get_hwnd().unwrap();
    let wparam = WPARAM(keycode as usize);
    let down = LPARAM((keycode << 16) as isize);
    let up = LPARAM((keycode << 16 | 1 << 30 | 1 << 31) as isize);
    post_message(hwnd, WM_KEYDOWN, wparam, down);
    post_message(hwnd, WM_KEYUP, wparam, up);
}

fn capture() -> DynamicImage {
    let hwnd = get_hwnd().unwrap();
    let swnd = unsafe { FindWindowExA(Some(hwnd), None, s!("subWin"), PCSTR::null()).unwrap() };
    let mut rect = RECT::default();
    unsafe { GetWindowRect(swnd, &mut rect).unwrap() };
    let width = rect.right - rect.left;
    let height = rect.bottom - rect.top;
    let mut buffer = vec![0u8; (width * height) as usize * 4];
    let mut info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width,
            biHeight: height,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: 0,
            biSizeImage: 0,
            biXPelsPerMeter: 0,
            biYPelsPerMeter: 0,
            biClrUsed: 0,
            biClrImportant: 0,
        },
        bmiColors: [RGBQUAD::default(); 1],
    };
    unsafe {
        let dc = GetDC(Some(hwnd));
        let cdc = CreateCompatibleDC(Some(dc));
        let cbmp = CreateCompatibleBitmap(dc, width, height);
        SelectObject(cdc, cbmp.into());
        let _ = PrintWindow(hwnd, cdc, PRINT_WINDOW_FLAGS(PW_CLIENTONLY.0 | PW_RENDERFULLCONTENT));
        let _ = GetDIBits(cdc, cbmp, 0, height as u32, Some(buffer.as_mut_ptr() as *mut _), &mut info, DIB_RGB_COLORS);
        let _ = DeleteObject(cbmp.into());
        ReleaseDC(Some(hwnd), dc);
        let _ = DeleteDC(dc);
        let _ = DeleteDC(cdc);
    }
    let mut chunks: Vec<Vec<u8>> = buffer.chunks(width as usize * 4).map(|x| x.to_vec()).collect();
    chunks.reverse();
    let rgba = chunks.concat().chunks_exact(4).take((width * height) as usize).flat_map(|bgra| [bgra[2], bgra[1], bgra[0], bgra[3]]).collect();
    let image = RgbaImage::from_vec(width as u32, height as u32, rgba).unwrap();
    let cropped_dynamic_image = process_image(&image, width, height);
    cropped_dynamic_image.resize_exact(CONFIG.display_width as u32, CONFIG.display_height as u32, FilterType::Lanczos3)
}

fn terminate() {
    let hwnd = get_hwnd().unwrap();
    post_message(hwnd, WM_CLOSE, WPARAM(0), LPARAM(0));
}

fn process_image(image: &RgbaImage, width: i32, height: i32) -> DynamicImage {
    let has_transparency = image.pixels().any(|pixel| pixel[3] == 0);
    let cropped_dynamic_image = if has_transparency {
        let (min_x, min_y, max_x, max_y) = find_non_transparent_bounds(image);
        let cropped_image = crop_imm(image, min_x, min_y, max_x - min_x, max_y - min_y);
        DynamicImage::ImageRgba8(cropped_image.to_image())
    } else {
        DynamicImage::ImageRgba8(image.clone())
    };
    let cropped_width = cropped_dynamic_image.width();
    let cropped_height = cropped_dynamic_image.height();
    let ratio = cropped_height as f32 / cropped_width as f32;
    let target_ratio = 9.0 / 16.0; // 16:9 ratio
    let error_margin = 0.001;

    let min_width = 1280.0 * 0.825;
    let min_height = 720.0 * 0.825;

    if (ratio - target_ratio).abs() > error_margin {
        show_notification(WARN, &format!("Aspect ratio is not 16:9 (16:{:.0})", ratio * 16.0), "not_16_9_ratio");
    } else if width < 2 && height < 2 {
        unsafe { ShowWindow(get_hwnd().unwrap(), SW_RESTORE).unwrap() };
        show_notification(WARN, "Minimized window is not supported", "minimized_not_supported");
    } else if (width as f32) < min_width || (height as f32) < min_height {
        show_notification(WARN, &format!("Resolution is too low ({}x{})", width, height), "resolution_too_low");
    } else {
        check_and_update_resolution(width as u32, height as u32);
    }

    cropped_dynamic_image
}

fn find_non_transparent_bounds(image: &RgbaImage) -> (u32, u32, u32, u32) {
    let (width, height) = image.dimensions();
    let mut min_x = width;
    let mut min_y = height;
    let mut max_x = 0;
    let mut max_y = 0;
    for (x, y, pixel) in (0..width).flat_map(|x| (0..height).map(move |y| (x, y, image.get_pixel(x, y)))) {
        if pixel[3] != 0 {
            min_x = min_x.min(x);
            min_y = min_y.min(y);
            max_x = max_x.max(x);
            max_y = max_y.max(y);
        }
    }
    (min_x, min_y, max_x + 1, max_y + 1)
}
