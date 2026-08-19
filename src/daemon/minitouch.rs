use std::{
    io::{self, BufRead},
    time::Duration,
};

use windows::Win32::{
    Foundation::{LPARAM, WPARAM},
    UI::WindowsAndMessaging::{IsWindow, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE},
};

use crate::game::capture::capture_touch_overlay;
use crate::game::input::{adb_keycode_to_vk, get_relative_point, key_down, key_up, post_message};
use crate::game::window::{await_modal_end, describe_window, parent_or_self, GameWindow};
use crate::sys::config::{DISPLAY_HEIGHT, DISPLAY_WIDTH};
use crate::sys::logging::{debug_log, LogLevel, LogMode};

const RESUME_LIMIT: Duration = Duration::from_millis(500);

// GameWindow::find() re-enumerates every window, so the cached handle is reused while it stays alive.
fn refresh_window(window: Option<GameWindow>, w_width: &mut i32, w_height: &mut i32) -> Option<GameWindow> {
    if let Some(window) = window.filter(|w| unsafe { IsWindow(Some(w.hwnd)).as_bool() }) {
        let (w, h) = window.get_client_size();
        *w_width = w;
        *w_height = h;
        return Some(window);
    }
    let new_win = GameWindow::find()?;
    let (w, h) = new_win.get_client_size();
    *w_width = w;
    *w_height = h;
    debug_log(LogLevel::Info, LogMode::Event, &format!("Minitouch: bound {}", describe_window(new_win.hwnd, None)));
    Some(new_win)
}

pub fn run_minitouch_daemon() {
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

    // The handshake above already told MAA this succeeded, so exiting now would close the pipe it just opened.
    let (mut w_width, mut w_height) = (0, 0);
    let mut window = refresh_window(None, &mut w_width, &mut w_height);
    if window.is_none() {
        debug_log(LogLevel::Warn, LogMode::Event, "Minitouch: window not found, waiting");
    }

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

                let held = match window.as_ref().map(|w| parent_or_self(w.hwnd)) {
                    Some(top) => await_modal_end(top),
                    None => Some(Duration::ZERO),
                };
                let Some(held) = held else {
                    window = None;
                    is_down = false;
                    touch_path.clear();
                    continue;
                };

                // A resize during the hold lands a new client size, so the one read before it is stale.
                if !held.is_zero() {
                    let before = (w_width, w_height);
                    window = refresh_window(window, &mut w_width, &mut w_height);

                    if is_down {
                        let resized = (w_width, w_height) != before;
                        let discard = window.is_none() || resized || held >= RESUME_LIMIT;
                        if let Some(win) = window.as_ref() {
                            if discard {
                                let (x, y) = touch_path.last().copied().unwrap_or_default();
                                last_relative_pos = get_relative_point(x, y, w_width, w_height);
                                post_message(win.hwnd, WM_MOUSEMOVE, WPARAM(1), LPARAM(last_relative_pos));
                                post_message(win.hwnd, WM_LBUTTONUP, WPARAM(1), LPARAM(last_relative_pos));
                                let reason = if resized { "window resized" } else { "held too long" };
                                debug_log(LogLevel::Info, LogMode::End, &format!("Minitouch: touch discarded after hold ({})", reason));
                            } else {
                                post_message(win.hwnd, WM_MOUSEMOVE, WPARAM(1), LPARAM(last_relative_pos));
                            }
                        }
                        if discard {
                            is_down = false;
                            touch_path.clear();
                        }
                    }
                }

                let Some(win) = window.as_ref() else {
                    // Nothing to press against, so the press is forgotten and a rebind starts from a fresh DOWN.
                    is_down = false;
                    touch_path.clear();
                    continue;
                };

                if let Some((x, y)) = current_pos {
                    let pos = get_relative_point(x, y, w_width, w_height);
                    if !is_down {
                        debug_log(LogLevel::Info, LogMode::Start, &format!("Minitouch: DOWN at x={}, y={}", x, y));
                        post_message(win.hwnd, WM_MOUSEMOVE, WPARAM(1), LPARAM(pos));
                        post_message(win.hwnd, WM_LBUTTONDOWN, WPARAM(1), LPARAM(pos));
                        is_down = true;
                        touch_path.clear();
                        touch_path.push((x, y));
                    } else if pos != last_relative_pos {
                        touch_path.push((x, y));
                        post_message(win.hwnd, WM_MOUSEMOVE, WPARAM(1), LPARAM(pos));
                    }
                    last_relative_pos = pos;
                } else if is_down {
                    debug_log(LogLevel::Info, LogMode::End, "Minitouch: UP");
                    post_message(win.hwnd, WM_MOUSEMOVE, WPARAM(1), LPARAM(last_relative_pos));
                    post_message(win.hwnd, WM_LBUTTONUP, WPARAM(1), LPARAM(last_relative_pos));
                    is_down = false;
                    capture_touch_overlay(win, &touch_path);
                    touch_path.clear();
                }
            }
            // "r" (RESET): r
            "r" => {
                debug_log(LogLevel::Info, LogMode::Plain, "Minitouch: RESET");

                window = refresh_window(window, &mut w_width, &mut w_height);

                if is_down {
                    if let Some(win) = window.as_ref() {
                        post_message(win.hwnd, WM_MOUSEMOVE, WPARAM(1), LPARAM(last_relative_pos));
                        post_message(win.hwnd, WM_LBUTTONUP, WPARAM(1), LPARAM(last_relative_pos));
                    }
                    is_down = false;
                }
                current_pos = None;
                touch_path.clear();
            }
            // "k" (KEY EVENT): k <keycode> <action>
            "k" if parts.len() >= 3 => {
                let keycode: i32 = parts[1].parse().unwrap_or(0);
                if let (Some(win), Some(vk_code)) = (window.as_ref(), adb_keycode_to_vk(keycode)) {
                    match parts[2] {
                        "d" => key_down(win, vk_code),
                        "u" => key_up(win, vk_code),
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }

    // The loop only ends when the pipe closes, which is MAA letting go.
    debug_log(LogLevel::Info, LogMode::Event, "Minitouch: MAA gone, stopped");
}
