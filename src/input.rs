use std::{
    thread,
    time::{Duration, Instant},
};

use spin_sleep;

use crate::{
    config::{config, DISPLAY_HEIGHT, DISPLAY_WIDTH},
    utils::{debug_capture, get_hwnd, get_info, invalidate_extras_image},
};

use windows::Win32::{Foundation::*, UI::WindowsAndMessaging::*};

pub fn post_message(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) {
    unsafe { _ = PostMessageA(Some(hwnd), msg, wparam, lparam) };
}

fn get_relative_point(x: i32, y: i32, w: i32, h: i32) -> isize {
    let nx = (x as f32 / DISPLAY_WIDTH as f32 * w as f32).round() as isize;
    let ny = (y as f32 / DISPLAY_HEIGHT as f32 * h as f32).round() as isize;
    ny << 16 | nx
}

pub fn input_tap(x: i32, y: i32) {
    debug_capture(x, y, None);

    let (hwnd, w, h) = get_info();
    let pos = get_relative_point(x, y, w, h);

    post_message(hwnd, WM_CANCELMODE, WPARAM(0), LPARAM(0));
    post_message(hwnd, WM_LBUTTONDOWN, WPARAM(1), LPARAM(pos));
    thread::sleep(Duration::from_millis(10));
    post_message(hwnd, WM_LBUTTONUP, WPARAM(1), LPARAM(pos));

    invalidate_extras_image();
}

pub fn input_text(text: &str) {
    let hwnd = get_hwnd().expect("Failed to find window (input_text)");
    for ch in text.chars() {
        post_message(hwnd, WM_CHAR, WPARAM(ch as usize), LPARAM(0));
        thread::sleep(Duration::from_millis(50));
    }
}

fn ease_in_out(t: f32) -> f32 {
    if t < 0.5 {
        4.0 * t * t * t
    } else {
        1.0 - (-2.0 * t + 2.0).powi(3) / 2.0
    }
}

pub fn input_swipe(x1: i32, y1: i32, x2: i32, y2: i32, duration: i32) {
    let (hwnd, w, h) = get_info();

    let effective_duration = Duration::from_millis((duration as f32 / config().swipe_speed as f32).max(1.0) as u64);
    let start_time = Instant::now();

    let dx = (x2 - x1) as f32;
    let dy = (y2 - y1) as f32;

    let pos_down = get_relative_point(x1, y1, w, h);
    post_message(hwnd, WM_CANCELMODE, WPARAM(0), LPARAM(0));
    post_message(hwnd, WM_LBUTTONDOWN, WPARAM(1), LPARAM(pos_down));
    thread::sleep(Duration::from_millis(10));

    let mut last_pos = pos_down;
    let mut debug_captured = false;

    loop {
        let elapsed = start_time.elapsed();
        if elapsed >= effective_duration {
            break;
        }

        let progress = elapsed.as_secs_f32() / effective_duration.as_secs_f32();
        let eased_progress = ease_in_out(progress);

        let nx = x1 + (dx * eased_progress).round() as i32;
        let ny = y1 + (dy * eased_progress).round() as i32;

        let pos = get_relative_point(nx, ny, w, h);
        if pos != last_pos {
            post_message(hwnd, WM_MOUSEMOVE, WPARAM(1), LPARAM(pos));
            last_pos = pos;
        }

        if !debug_captured && progress >= 0.9 {
            debug_capture(x1, y1, Some((x2, y2)));
            debug_captured = true;
        }

        spin_sleep::sleep(Duration::from_millis(1));
    }

    let pos_up = get_relative_point(x2, y2, w, h);
    post_message(hwnd, WM_MOUSEMOVE, WPARAM(1), LPARAM(pos_up));
    thread::sleep(Duration::from_millis(10));
    post_message(hwnd, WM_LBUTTONUP, WPARAM(1), LPARAM(pos_up));
}

pub fn input_keyevent(keycode: i32) {
    let hwnd = get_hwnd().expect("Failed to find window (input_keyevent)");
    let wparam = WPARAM(keycode as usize);
    let down = LPARAM((keycode << 16) as isize);
    let up = LPARAM((keycode << 16 | 1 << 30 | 1 << 31) as isize);
    post_message(hwnd, WM_KEYDOWN, wparam, down);
    post_message(hwnd, WM_KEYUP, wparam, up);
}

pub fn terminate() {
    let hwnd = get_hwnd().expect("Failed to find window (terminate)");
    post_message(hwnd, WM_CLOSE, WPARAM(0), LPARAM(0));
}
