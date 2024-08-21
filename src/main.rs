#![allow(non_snake_case)]

use image::{codecs::png::PngEncoder, imageops::FilterType, DynamicImage, RgbaImage};
use std::{env, io::*, mem, time::Duration};
use windows::{
    core::*,
    Win32::{Foundation::*, Graphics::Gdi::*, Storage::Xps::*, UI::HiDpi::*, UI::WindowsAndMessaging::*},
};

type StdResult<T> = std::result::Result<T, &'static str>;

const PACKAGE: &str = "com.YoStarKR.Arknights";
const TITLE: PCWSTR = w!("명일방주"); // Note: Rename the title you want to interact with.
const CLASS: PCWSTR = w!("CROSVM_1"); // Note: Warning. May cause problems in the future.
const DISPLAY_WIDTH: f32 = 1280.0;
const DISPLAY_HEIGHT: f32 = 720.0;
const POLLING_RATE: i32 = 1000 / 500;

const SWIPE_START_TO_MID_RATIO: f32 = 0.9;
const SWIPE_START_TO_MID_SPEED: f32 = 10.0;
const SWIPE_MID_TO_END_RATIO: f32 = 0.1;
const SWIPE_MID_TO_END_SPEED: f32 = 2.0;

enum Command {
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

    if unsafe { FindWindowW(CLASS, TITLE).unwrap_or_default().0 }.is_null() {
        start_arknights();
    }

    let _ = execute_command(command);
}

fn parse_command(args: &[String]) -> Command {
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
        c if c.contains("am force-stop") => Command::ForceStop,
        c if c.contains("cat /proc/net/arp")
            || c.contains("settings get secure android_id")
            || c.contains("exec-out screencap | nc -w 3")
            || c.contains("exec-out screencap | gzip -1")
            || c.contains("start-server")
            || c.contains("kill-server") =>
        {
            Command::ExceptionCommand(full_command)
        }
        _ => Command::Unknown(full_command),
    }
}

fn start_arknights() {
    let _ = open::that(format!("googleplaygames://launch/?id={}", PACKAGE));
    println!("start Arknights");

    loop {
        let hwnd = unsafe { FindWindowW(CLASS, TITLE) };
        if hwnd.is_ok() && hwnd.unwrap() != HWND(std::ptr::null_mut()) {
            println!("> start Arknights success!");
            break;
        }
        println!("Waiting for Arknights to start...");
        std::thread::sleep(Duration::from_millis(500));
    }
}

fn execute_command(command: Command) -> StdResult<()> {
    match command {
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
            println!("Warning: Activity not started, intent has been delivered to currently running top-most instance.");
        }
        Command::InputTap { x, y } => {
            input_tap(x, y);
            println!("PlayBridge: Tapped at ({}, {})", x, y);
        }
        Command::InputSwipe { x1, y1, x2, y2, duration } => {
            input_swipe(x1, y1, x2, y2, duration);
            println!("PlayBridge: Swiped from ({}, {}) to ({}, {}) over {} ms", x1, y1, x2, y2, duration);
        }
        Command::InputKeyEvent { keycode } => {
            input_keyevent(keycode);
            println!("PlayBridge: Key event {}", keycode);
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
            println!("PlayBridge: am force-stop");
        }
        Command::ExceptionCommand(cmd) => println!("Exception: {}", cmd),
        Command::Unknown(cmd) => println!("PlayBridge: {} (Unknown command)", cmd),
    }
    Ok(())
}

fn get_gpg_info() -> (HWND, i32, i32) {
    let hwnd = unsafe { FindWindowW(CLASS, TITLE).unwrap() };
    let mut client_rect = RECT::default();
    unsafe { GetClientRect(hwnd, &mut client_rect).unwrap() };
    (hwnd, client_rect.right - client_rect.left, client_rect.bottom - client_rect.top)
}

fn get_relative_point(x: i32, y: i32, w: i32, h: i32) -> isize {
    ((y as f32 / DISPLAY_HEIGHT * h as f32).ceil() as isize) << 16 | (x as f32 / DISPLAY_WIDTH * w as f32).ceil() as isize
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
    let hwnd = unsafe { FindWindowW(CLASS, TITLE).unwrap() };
    unsafe {
        _ = PostMessageA(hwnd, WM_KEYDOWN, WPARAM(keycode as usize), LPARAM((keycode << 16) as isize));
        _ = PostMessageA(hwnd, WM_KEYUP, WPARAM(keycode as usize), LPARAM((keycode << 16 | 1 << 30 | 1 << 31) as isize));
    }
}

fn capture() -> DynamicImage {
    let hwnd = unsafe { FindWindowW(CLASS, TITLE).unwrap() };
    let swnd = unsafe { FindWindowExA(hwnd, HWND(std::ptr::null_mut()), s!("subWin"), PCSTR::null()).unwrap() };
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

    let rgba =
        chunks.concat().chunks_exact(4).take((width * height) as usize).flat_map(|bgra| [bgra[2], bgra[1], bgra[0], bgra[3]]).collect();
    let image = RgbaImage::from_vec(width as u32, height as u32, rgba).unwrap();
    image::DynamicImage::ImageRgba8(image).resize_exact(DISPLAY_WIDTH as u32, DISPLAY_HEIGHT as u32, FilterType::Lanczos3)
}

fn terminate() {
    let hwnd = unsafe { FindWindowW(CLASS, TITLE).unwrap() };
    unsafe { _ = PostMessageA(hwnd, WM_CLOSE, WPARAM(0), LPARAM(0)) };
}
