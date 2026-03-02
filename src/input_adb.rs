use std::{
    thread,
    time::{Duration, Instant},
};

use windows::Win32::{
    Foundation::{LPARAM, WPARAM},
    UI::WindowsAndMessaging::{WM_CHAR, WM_KEYDOWN, WM_KEYUP, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE},
};

use crate::{
    capture::debug_capture,
    input::{get_relative_point, post_message, send_cancel_mode},
    notification::{display_notification, Notification},
    window::GameWindow,
};

const INPUT_DELAY_MS: u64 = 10;
const TEXT_INPUT_DELAY_MS: u64 = 50;
const SWIPE_SPEED: u32 = 10;

pub fn input_tap(window: &GameWindow, x: i32, y: i32) {
    display_notification(Notification::AdbInputDeprecation);
    debug_capture(window, x, y, None);

    let (w, h) = window.get_info();
    let pos = get_relative_point(x, y, w, h);

    send_cancel_mode(window.hwnd);
    post_message(window.hwnd, WM_MOUSEMOVE, WPARAM(1), LPARAM(pos));
    thread::sleep(Duration::from_millis(INPUT_DELAY_MS));
    post_message(window.hwnd, WM_LBUTTONDOWN, WPARAM(1), LPARAM(pos));
    thread::sleep(Duration::from_millis(INPUT_DELAY_MS));
    post_message(window.hwnd, WM_LBUTTONUP, WPARAM(1), LPARAM(pos));
}

pub fn input_swipe(window: &GameWindow, x1: i32, y1: i32, x2: i32, y2: i32, duration: i32) {
    display_notification(Notification::AdbInputDeprecation);

    let (w, h) = window.get_info();
    let effective_duration = Duration::from_millis((duration as f32 / SWIPE_SPEED as f32).max(1.0) as u64);
    let start_time = Instant::now();

    let dx = (x2 - x1) as f32;
    let dy = (y2 - y1) as f32;

    let pos_down = get_relative_point(x1, y1, w, h);
    send_cancel_mode(window.hwnd);
    post_message(window.hwnd, WM_LBUTTONDOWN, WPARAM(1), LPARAM(pos_down));
    thread::sleep(Duration::from_millis(INPUT_DELAY_MS));

    let mut last_pos = pos_down;

    loop {
        let elapsed = start_time.elapsed();
        if elapsed >= effective_duration {
            break;
        }

        let eased_progress = ease_in_out(elapsed.as_secs_f32() / effective_duration.as_secs_f32());

        let nx = x1 + (dx * eased_progress).round() as i32;
        let ny = y1 + (dy * eased_progress).round() as i32;

        let pos = get_relative_point(nx, ny, w, h);
        if pos != last_pos {
            post_message(window.hwnd, WM_MOUSEMOVE, WPARAM(1), LPARAM(pos));
            last_pos = pos;
        }

        spin_sleep::sleep(Duration::from_millis(1));
    }

    let pos_up = get_relative_point(x2, y2, w, h);
    post_message(window.hwnd, WM_MOUSEMOVE, WPARAM(1), LPARAM(pos_up));
    thread::sleep(Duration::from_millis(INPUT_DELAY_MS));
    post_message(window.hwnd, WM_LBUTTONUP, WPARAM(1), LPARAM(pos_up));

    debug_capture(window, x1, y1, Some((x2, y2)));
}

pub fn input_keyevent(window: &GameWindow, keycode: i32) {
    let wparam = WPARAM(keycode as usize);
    let down = LPARAM((keycode << 16) as isize);
    let up = LPARAM((keycode << 16 | 1 << 30 | 1 << 31) as isize);
    post_message(window.hwnd, WM_KEYDOWN, wparam, down);
    post_message(window.hwnd, WM_KEYUP, wparam, up);
}

pub fn input_text(window: &GameWindow, text: &str) {
    for ch in text.chars() {
        post_message(window.hwnd, WM_CHAR, WPARAM(ch as usize), LPARAM(0));
        thread::sleep(Duration::from_millis(TEXT_INPUT_DELAY_MS));
    }
}

fn ease_in_out(t: f32) -> f32 {
    if t < 0.5 {
        4.0 * t * t * t
    } else {
        1.0 - (-2.0 * t + 2.0).powi(3) / 2.0
    }
}
