use std::{thread, time::Duration};

use windows::Win32::{
    Foundation::{HWND, LPARAM, WPARAM},
    System::Threading::{AttachThreadInput, GetCurrentThreadId},
    UI::{
        Input::KeyboardAndMouse::{EnableWindow, ReleaseCapture},
        WindowsAndMessaging::{PostMessageW, WM_CHAR, WM_CLOSE, WM_KEYDOWN, WM_KEYUP},
    },
};

use crate::game::window::{gui_thread_info, GameWindow};
use crate::shared::{DISPLAY_HEIGHT, DISPLAY_WIDTH};

const TEXT_INPUT_DELAY_MS: u64 = 50;

pub fn post_message(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) {
    unsafe { _ = PostMessageW(Some(hwnd), msg, wparam, lparam) };
}

pub fn set_input_enabled(top: HWND, enabled: bool) {
    unsafe { _ = EnableWindow(top, enabled) };
}

// Capture belongs to the holder's input state, so it cannot be released from outside without
// sharing that state for the call.
pub fn release_game_capture(top: HWND) -> bool {
    let Some((target, info)) = gui_thread_info(top) else {
        return false;
    };
    if info.hwndCapture.0.is_null() {
        return false;
    }

    let ours = unsafe { GetCurrentThreadId() };
    unsafe {
        if !AttachThreadInput(ours, target, true).as_bool() {
            return false;
        }
        let released = ReleaseCapture().is_ok();
        _ = AttachThreadInput(ours, target, false);
        released
    }
}

pub fn terminate(window: &GameWindow) {
    post_message(window.hwnd, WM_CLOSE, WPARAM(0), LPARAM(0));
}

pub fn get_relative_point(x: i32, y: i32, w: i32, h: i32) -> isize {
    let nx = (x as f32 / DISPLAY_WIDTH as f32 * w as f32).round() as isize;
    let ny = (y as f32 / DISPLAY_HEIGHT as f32 * h as f32).round() as isize;
    ny << 16 | nx
}

pub fn adb_keycode_to_vk(adb_keycode: i32) -> Option<i32> {
    match adb_keycode {
        111 => Some(0x1B), // KEYCODE_ESCAPE -> VK_ESCAPE
        _ => None,
    }
}

pub fn key_down(window: &GameWindow, vk_code: i32) {
    let lparam = LPARAM((vk_code << 16) as isize);
    post_message(window.hwnd, WM_KEYDOWN, WPARAM(vk_code as usize), lparam);
}

pub fn key_up(window: &GameWindow, vk_code: i32) {
    let lparam = LPARAM((vk_code << 16 | 1 << 30 | 1 << 31) as isize);
    post_message(window.hwnd, WM_KEYUP, WPARAM(vk_code as usize), lparam);
}

pub fn input_keyevent(window: &GameWindow, adb_keycode: i32) {
    let Some(vk_code) = adb_keycode_to_vk(adb_keycode) else {
        return;
    };
    key_down(window, vk_code);
    key_up(window, vk_code);
}

pub fn input_text(window: &GameWindow, text: &str) {
    for ch in text.chars() {
        post_message(window.hwnd, WM_CHAR, WPARAM(ch as usize), LPARAM(0));
        thread::sleep(Duration::from_millis(TEXT_INPUT_DELAY_MS));
    }
}
