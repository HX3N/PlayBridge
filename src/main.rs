#![allow(non_snake_case)]

mod notification;

use chrono::Local;
use image::{codecs::png::PngEncoder, imageops::crop_imm, imageops::FilterType, DynamicImage, RgbaImage};
use std::{env, fs::File, io::*, mem, time::Duration};
use windows::{
    core::*,
    Win32::{Foundation::*, Graphics::Gdi::*, Storage::Xps::*, UI::HiDpi::*, UI::WindowsAndMessaging::*},
};

const PACKAGE: &str = "com.YoStarKR.Arknights";
const TITLE: PCWSTR = w!("명일방주"); // Note: Rename the title you want to interact with
const CLASS: PCWSTR = w!("CROSVM_1"); // Note: Warning. May cause problems in the future
const DISPLAY_WIDTH: f32 = 1280.0;
const DISPLAY_HEIGHT: f32 = 720.0;
const POLLING_RATE: i32 = 1000 / 500;

// Constants defining the ratio of each sector
const SWIPE_START_TO_MID_RATIO: f32 = 0.9;
const SWIPE_MID_TO_END_RATIO: f32 = 0.1;

// Constants defining the speed of each sector. Values between 2-12 are recommended
const SWIPE_START_TO_MID_SPEED: f32 = 10.0;
const SWIPE_MID_TO_END_SPEED: f32 = 2.0;

fn get_hwnd() -> Option<HWND> {
    unsafe { FindWindowW(CLASS, TITLE).ok() }
}

enum Command {
    Empty,
    Connect,
    GetPropVersionRelease,
    StartActivity { intent: String },
    InputTap { x: i32, y: i32 },
    InputSwipe { x1: i32, y1: i32, x2: i32, y2: i32, duration: i32 },
    InputKeyEvent { keycode: i32 },
    DumpsysWindowDisplays,
    ExecOutScreencap,
    ForceStop,
    ExceptionCommand(String),
    Unknown(String),
}

fn main() {
    unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2).unwrap() };

    let args = env::args().collect::<Vec<_>>();
    let command = parse_command(&args);

    get_hwnd().is_none().then(|| start_arknights());

    let _ = execute_command(command);
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
        c if c.contains("input swipe") => Command::InputSwipe {
            x1: args[6].parse().unwrap(),
            y1: args[7].parse().unwrap(),
            x2: args[8].parse().unwrap(),
            y2: args[9].parse().unwrap(),
            duration: args[10].parse().unwrap(),
        },
        c if c.contains("input keyevent 111") => Command::InputKeyEvent { keycode: 0x01 },
        c if c.contains("dumpsys window displays") => Command::DumpsysWindowDisplays,
        c if c.contains("exec-out screencap -p") => Command::ExecOutScreencap,
        c if c.contains("am force-stop") || c.contains("input keyevent HOME") => Command::ForceStop,
        c if c.contains("cat /proc/net/arp")
            || c.contains("settings get secure android_id")
            || c.contains("exec-out screencap | nc -w 3")
            || c.contains("exec-out screencap | gzip -1")
            || c.contains("start-server")
            || c.contains("kill-server")
            || c.contains("devices") =>
        {
            Command::ExceptionCommand(full_command)
        }
        _ => Command::Unknown(full_command),
    }
}

fn start_arknights() {
    _ = open::that(format!("googleplaygames://launch/?id={}", PACKAGE));

    while get_hwnd().is_none() {
        std::thread::sleep(Duration::from_millis(500));
    }

    notification::show_notification("start_arknights", None);
}

fn execute_command(command: Command) {
    match command {
        Command::Empty => {
            let filename = format!("Screenshot_{}.png", Local::now().format("%Y.%m.%d_%H.%M.%S.%3f"));
            let filepath = format!("{}\\Desktop\\{}", env::var("USERPROFILE").unwrap(), filename);
            capture().write_with_encoder(PngEncoder::new(File::create(&filepath).unwrap())).unwrap();
            notification::show_notification("screenshot_saved", Some(&filename));
        }
        Command::Connect => {
            // maa needs this output
            println!("connected to Google Play Games Beta");
        }
        Command::GetPropVersionRelease => {
            // maa needs this output
            println!("14");
        }
        Command::StartActivity { intent } => {
            // maa needs this output
            println!("Starting: Intent {{ cmp={} }}", intent);
            // Note: As PlayBridge does not call the intent directly, it needs to print warning
            println!("Warning: Activity not started, intent has been delivered to currently running top-most instance.");
        }
        Command::InputTap { x, y } => {
            input_tap(x, y);
        }
        Command::InputSwipe { x1, y1, x2, y2, duration } => {
            input_swipe(x1, y1, x2, y2, duration);
        }
        Command::InputKeyEvent { keycode } => {
            input_keyevent(keycode);
        }
        Command::DumpsysWindowDisplays => {
            // maa needs this output
            println!("{} {}", DISPLAY_WIDTH as i32, DISPLAY_HEIGHT as i32);
        }
        Command::ExecOutScreencap => {
            let image = capture();
            image.write_with_encoder(PngEncoder::new(&mut stdout().lock())).unwrap();
        }
        Command::ForceStop => {
            terminate();
            notification::show_notification("shutdown_arknights", None);
        }
        Command::ExceptionCommand(cmd) => println!("PlayBridge: {} (Exception)", cmd),
        Command::Unknown(cmd) => {
            println!("PlayBridge: {} (Unknown command)", cmd);
            notification::show_notification("unknown_command", Some(&format!("{}", cmd)));
        }
    }
}

fn get_gpg_info() -> (HWND, i32, i32) {
    let hwnd = get_hwnd().unwrap();

    let mut client_rect = RECT::default();
    unsafe { _ = GetClientRect(hwnd, &mut client_rect) };

    (hwnd, (client_rect.right - client_rect.left) as i32, (client_rect.bottom - client_rect.top) as i32)
}

fn get_relative_point(x: i32, y: i32, w: i32, h: i32) -> isize {
    let nx = (x as f32 / DISPLAY_WIDTH * w as f32).ceil() as isize;
    let ny = (y as f32 / DISPLAY_HEIGHT * h as f32).ceil() as isize;

    ny << 16 | nx
}

fn input_tap(x: i32, y: i32) {
    let (hwnd, w, h) = get_gpg_info();
    let pos = get_relative_point(x, y, w, h);

    unsafe {
        _ = PostMessageA(hwnd, WM_LBUTTONDOWN, WPARAM(1), LPARAM(pos));
        _ = PostMessageA(hwnd, WM_LBUTTONUP, WPARAM(1), LPARAM(pos));
    }
}

fn input_swipe(x1: i32, y1: i32, x2: i32, y2: i32, duration: i32) {
    let (hwnd, w, h) = get_gpg_info();

    let dx = (x2 - x1) as f32;
    let dy = (y2 - y1) as f32;

    let duration_start_to_mid = (duration as f32 * SWIPE_START_TO_MID_RATIO) / SWIPE_START_TO_MID_SPEED;
    let duration_mid_to_end = (duration as f32 * SWIPE_MID_TO_END_RATIO) / SWIPE_MID_TO_END_SPEED;

    let steps_start_to_mid = (duration_start_to_mid / POLLING_RATE as f32).ceil() as i32;
    let steps_mid_to_end = (duration_mid_to_end / POLLING_RATE as f32).ceil() as i32;

    for cnt in 0..steps_start_to_mid {
        let progress = cnt as f32 / steps_start_to_mid as f32;
        let nx = x1 + (dx * progress * SWIPE_START_TO_MID_RATIO) as i32;
        let ny = y1 + (dy * progress * SWIPE_START_TO_MID_RATIO) as i32;
        let pos = get_relative_point(nx, ny, w, h);
        unsafe { _ = PostMessageA(hwnd, WM_LBUTTONDOWN, WPARAM(1), LPARAM(pos)) };
        spin_sleep::sleep(Duration::new(0, POLLING_RATE as u32 * 1_000_000));
    }

    for cnt in 0..steps_mid_to_end {
        let progress = cnt as f32 / steps_mid_to_end as f32;
        let nx = x1 + (dx * (SWIPE_START_TO_MID_RATIO + progress * SWIPE_MID_TO_END_RATIO)) as i32;
        let ny = y1 + (dy * (SWIPE_START_TO_MID_RATIO + progress * SWIPE_MID_TO_END_RATIO)) as i32;
        let pos = get_relative_point(nx, ny, w, h);
        unsafe { _ = PostMessageA(hwnd, WM_LBUTTONDOWN, WPARAM(1), LPARAM(pos)) };
        spin_sleep::sleep(Duration::new(0, POLLING_RATE as u32 * 1_000_000));
    }

    let pos = get_relative_point(x2, y2, w, h);
    unsafe { _ = PostMessageA(hwnd, WM_LBUTTONUP, WPARAM(1), LPARAM(pos)) };
}

fn input_keyevent(keycode: i32) {
    let hwnd = get_hwnd().unwrap();

    let wparam = WPARAM(keycode as usize);
    let down = LPARAM((keycode << 16) as isize);
    let up = LPARAM((keycode << 16 | 1 << 30 | 1 << 31) as isize);

    unsafe {
        _ = PostMessageA(hwnd, WM_KEYDOWN, wparam, down);
        _ = PostMessageA(hwnd, WM_KEYUP, wparam, up);
    }
}

fn capture() -> DynamicImage {
    let hwnd = get_hwnd().unwrap();
    let swnd = unsafe { FindWindowExA(hwnd, None, s!("subWin"), PCSTR::null()).unwrap() };

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
        let dc = GetDC(hwnd);
        let cdc = CreateCompatibleDC(dc);
        let cbmp = CreateCompatibleBitmap(dc, width, height);

        SelectObject(cdc, cbmp);
        _ = PrintWindow(hwnd, cdc, PRINT_WINDOW_FLAGS(PW_CLIENTONLY.0 | PW_RENDERFULLCONTENT));
        GetDIBits(cdc, cbmp, 0, height as u32, Some(buffer.as_mut_ptr() as *mut _), &mut info, DIB_RGB_COLORS);

        _ = DeleteObject(cbmp);
        ReleaseDC(hwnd, dc);
        _ = DeleteDC(dc);
        _ = DeleteDC(cdc);
    }

    let mut chunks: Vec<Vec<u8>> = buffer.chunks(width as usize * 4).map(|x| x.to_vec()).collect();
    chunks.reverse();

    let rgba = chunks.concat().chunks_exact(4).take((width * height) as usize).flat_map(|bgra| [bgra[2], bgra[1], bgra[0], bgra[3]]).collect();
    let image = RgbaImage::from_vec(width as u32, height as u32, rgba).unwrap();
    let cropped_dynamic_image = process_image(&image, width, height);

    cropped_dynamic_image.resize_exact(DISPLAY_WIDTH as u32, DISPLAY_HEIGHT as u32, FilterType::Lanczos3)
}

fn terminate() {
    let hwnd = get_hwnd().unwrap();
    unsafe { _ = PostMessageA(hwnd, WM_CLOSE, WPARAM(0), LPARAM(0)) };
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
    let ratio = (cropped_height as f32) / (cropped_width as f32);

    let target_ratio = 0.5625; // 16:9 ratio ~0.5625
    let error_margin = 0.001;

    if (ratio - target_ratio).abs() > error_margin {
        notification::show_notification("not_16_9_ratio", Some(&format!("16:{:.0}", ratio * 16.0)));
    } else if width < 2 && height < 2 {
        unsafe { ShowWindow(get_hwnd().unwrap(), SW_RESTORE).unwrap() };
        notification::show_notification("minimized_not_supported", None);
    } else if (width as f32) < (DISPLAY_WIDTH * 0.825) || (height as f32) < (DISPLAY_HEIGHT * 0.825) {
        notification::show_notification("resolution_too_low", Some(&format!("{}x{}", width, height)));
    } else {
        notification::check_and_update_resolution(width as u32, height as u32);
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
