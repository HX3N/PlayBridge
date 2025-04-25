use std::{thread, time::Duration};

use spin_sleep;

use crate::{
    config::CONFIG,
    utils::{get_hwnd, get_info},
};

use windows::Win32::{Foundation::*, UI::WindowsAndMessaging::*};

pub fn post_message(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) {
    unsafe { _ = PostMessageA(Some(hwnd), msg, wparam, lparam) };
}

fn get_relative_point(x: i32, y: i32, w: i32, h: i32) -> isize {
    let nx = (x as f32 / CONFIG.width as f32 * w as f32).ceil() as isize;
    let ny = (y as f32 / CONFIG.height as f32 * h as f32).ceil() as isize;
    ny << 16 | nx
}

pub fn input_tap(x: i32, y: i32) {
    let (hwnd, w, h) = get_info();
    let pos = get_relative_point(x, y, w, h);
    post_message(hwnd, WM_LBUTTONDOWN, WPARAM(1), LPARAM(pos));
    post_message(hwnd, WM_LBUTTONUP, WPARAM(1), LPARAM(pos));
}

pub fn input_text(text: &str) {
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

pub fn input_swipe(x1: i32, y1: i32, x2: i32, y2: i32, duration: i32) {
    let (hwnd, w, h) = get_info();

    let effective_duration = duration as f32 / CONFIG.swipe_speed as f32;
    let steps = (effective_duration / 1000.0 * CONFIG.polling_rate as f32).ceil() as u32;
    let sleep_nanos = 1_000_000_000u64 / CONFIG.polling_rate as u64;

    let dx = (x2 - x1) as f32;
    let dy = (y2 - y1) as f32;

    let pos_down = get_relative_point(x1, y1, w, h);
    post_message(hwnd, WM_LBUTTONDOWN, WPARAM(1), LPARAM(pos_down));

    for cnt in 0..=steps {
        let progress = cnt as f32 / steps as f32;
        let eased = ease_out(progress).min(1.0);

        let (nx, ny) = if cnt == steps { (x2, y2) } else { (x1 + (dx * eased).round() as i32, y1 + (dy * eased).round() as i32) };

        let pos = get_relative_point(nx, ny, w, h);
        post_message(hwnd, WM_MOUSEMOVE, WPARAM(1), LPARAM(pos));
        spin_sleep::sleep(Duration::new(0, sleep_nanos as u32));
    }

    let pos_up = get_relative_point(x2, y2, w, h);
    spin_sleep::sleep(Duration::new(0, sleep_nanos as u32 * 5));
    post_message(hwnd, WM_LBUTTONDOWN, WPARAM(1), LPARAM(pos_up));
    post_message(hwnd, WM_LBUTTONUP, WPARAM(1), LPARAM(pos_up));
}

pub fn input_keyevent(keycode: i32) {
    let hwnd = get_hwnd().unwrap();
    let wparam = WPARAM(keycode as usize);
    let down = LPARAM((keycode << 16) as isize);
    let up = LPARAM((keycode << 16 | 1 << 30 | 1 << 31) as isize);
    post_message(hwnd, WM_KEYDOWN, wparam, down);
    post_message(hwnd, WM_KEYUP, wparam, up);
}

pub fn terminate() {
    let hwnd = get_hwnd().unwrap();
    post_message(hwnd, WM_CLOSE, WPARAM(0), LPARAM(0));
}
