use std::{
    io::{self, BufRead},
    thread,
    time::Duration,
};

use windows::Win32::{
    Foundation::{HWND, LPARAM, WPARAM},
    UI::WindowsAndMessaging::{
        IsWindow, PostMessageW, WM_CANCELMODE, WM_CHAR, WM_CLOSE, WM_KEYDOWN, WM_KEYUP, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE,
    },
};

use crate::{
    capture::debug_capture,
    config::{DISPLAY_HEIGHT, DISPLAY_WIDTH},
    logging::{debug_log, LogLevel, LogMode},
    notification::{display_notification, Notification},
    window::{parent_or_self, GameWindow},
};

const TEXT_INPUT_DELAY_MS: u64 = 50;

pub fn post_message(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) {
    unsafe { _ = PostMessageW(Some(hwnd), msg, wparam, lparam) };
}

pub fn send_cancel_mode(hwnd: HWND) {
    let target_hwnd = parent_or_self(hwnd);
    post_message(target_hwnd, WM_CANCELMODE, WPARAM(0), LPARAM(0));
}

pub fn terminate(window: &GameWindow) {
    post_message(window.hwnd, WM_CLOSE, WPARAM(0), LPARAM(0));
}

pub fn get_relative_point(x: i32, y: i32, w: i32, h: i32) -> isize {
    let nx = (x as f32 / DISPLAY_WIDTH as f32 * w as f32).round() as isize;
    let ny = (y as f32 / DISPLAY_HEIGHT as f32 * h as f32).round() as isize;
    ny << 16 | nx
}

fn adb_keycode_to_vk(adb_keycode: i32) -> Option<i32> {
    match adb_keycode {
        111 => Some(0x1B), // KEYCODE_ESCAPE -> VK_ESCAPE
        _ => None,
    }
}

fn key_down(window: &GameWindow, vk_code: i32) {
    let lparam = LPARAM((vk_code << 16) as isize);
    post_message(window.hwnd, WM_KEYDOWN, WPARAM(vk_code as usize), lparam);
}

fn key_up(window: &GameWindow, vk_code: i32) {
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

// Skip the full GameWindow::find() (window_list enumeration) while the cached handle is valid;
// just refresh its client size. Re-enumerate only if it's gone.
fn refresh_window(window: GameWindow, w_width: &mut i32, w_height: &mut i32) -> GameWindow {
    if unsafe { IsWindow(Some(window.hwnd)).as_bool() } {
        let (w, h) = window.get_client_size();
        *w_width = w;
        *w_height = h;
        return window;
    }
    if let Some(new_win) = GameWindow::find() {
        let (w, h) = new_win.get_client_size();
        *w_width = w;
        *w_height = h;
        return new_win;
    }
    window
}

pub fn run_minitouch_daemon() {
    display_notification(Notification::MinitouchStarted);
    debug_log(LogLevel::Info, LogMode::End, "Minitouch: started / awaiting handshake");

    println!("v 1");
    println!("^ 10 {} {} 100", DISPLAY_WIDTH, DISPLAY_HEIGHT);
    println!("$");

    let stdin = io::stdin();
    let mut iterator = stdin.lock().lines();

    let mut current_pos: Option<(i32, i32)> = None;
    let mut is_down = false;
    let mut last_relative_pos = 0;
    let mut touch_path: Vec<(i32, i32)> = Vec::new();

    let mut window = match GameWindow::find() {
        Some(w) => w,
        None => return,
    };
    let (mut w_width, mut w_height) = window.get_client_size();

    while let Some(Ok(line)) = iterator.next() {
        if line.is_empty() {
            continue;
        }

        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.is_empty() {
            continue;
        }

        match parts[0] {
            // "d" (DOWN): d <contact_id> <x> <y> <pressure>
            // "m" (MOVE): m <contact_id> <x> <y> <pressure>
            "d" | "m" => {
                if parts.len() >= 4 {
                    let contact: i32 = parts[1].parse().unwrap_or(-1);
                    if contact == 0 {
                        let x: i32 = parts[2].parse().unwrap_or(0);
                        let y: i32 = parts[3].parse().unwrap_or(0);
                        current_pos = Some((x, y));
                    }
                }
            }
            // "u" (UP): u <contact_id>
            "u" => {
                if parts.len() >= 2 {
                    let contact: i32 = parts[1].parse().unwrap_or(-1);
                    if contact == 0 {
                        current_pos = None;
                    }
                }
            }
            // "w" (WAIT): w <milliseconds>
            "w" => {
                if parts.len() >= 2 {
                    let ms: u64 = parts[1].parse().unwrap_or(0);
                    if ms > 0 {
                        spin_sleep::sleep(Duration::from_millis(ms));
                    }
                }
            }
            // "c" (COMMIT): c
            "c" => {
                window = refresh_window(window, &mut w_width, &mut w_height);

                if let Some((x, y)) = current_pos {
                    let pos = get_relative_point(x, y, w_width, w_height);
                    if !is_down {
                        debug_log(LogLevel::Info, LogMode::Start, &format!("Minitouch: DOWN at x={}, y={}", x, y));
                        send_cancel_mode(window.hwnd);
                        post_message(window.hwnd, WM_MOUSEMOVE, WPARAM(1), LPARAM(pos));
                        post_message(window.hwnd, WM_LBUTTONDOWN, WPARAM(1), LPARAM(pos));
                        is_down = true;
                        touch_path.clear();
                        touch_path.push((x, y));
                    } else if pos != last_relative_pos {
                        touch_path.push((x, y));
                        post_message(window.hwnd, WM_MOUSEMOVE, WPARAM(1), LPARAM(pos));
                    }
                    last_relative_pos = pos;
                } else if is_down {
                    debug_log(LogLevel::Info, LogMode::End, "Minitouch: UP");
                    post_message(window.hwnd, WM_MOUSEMOVE, WPARAM(1), LPARAM(last_relative_pos));
                    post_message(window.hwnd, WM_LBUTTONUP, WPARAM(1), LPARAM(last_relative_pos));
                    is_down = false;
                    debug_capture(&window, &touch_path);
                    touch_path.clear();
                }
            }
            // "r" (RESET): r
            "r" => {
                debug_log(LogLevel::Info, LogMode::Event, "Minitouch: RESET");

                window = refresh_window(window, &mut w_width, &mut w_height);

                if is_down {
                    post_message(window.hwnd, WM_MOUSEMOVE, WPARAM(1), LPARAM(last_relative_pos));
                    post_message(window.hwnd, WM_LBUTTONUP, WPARAM(1), LPARAM(last_relative_pos));
                    is_down = false;
                }
                current_pos = None;
                touch_path.clear();
            }
            // "k" (KEY EVENT): k <keycode> <action>
            "k" if parts.len() >= 3 => {
                let keycode: i32 = parts[1].parse().unwrap_or(0);
                if let Some(vk_code) = adb_keycode_to_vk(keycode) {
                    match parts[2] {
                        "d" => key_down(&window, vk_code),
                        "u" => key_up(&window, vk_code),
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }

    display_notification(Notification::MinitouchStopped);
    debug_log(LogLevel::Info, LogMode::End, "Minitouch: stopped");
}
