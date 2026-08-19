use std::{
    ffi::c_void,
    io::{self, BufRead},
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

use windows::Win32::{
    Foundation::{HWND, LPARAM, WPARAM},
    UI::WindowsAndMessaging::{IsWindow, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE},
};

use crate::game::capture::capture_touch_overlay;
use crate::game::input::{adb_keycode_to_vk, get_relative_point, key_down, key_up, post_message, release_game_capture, set_input_enabled};
use crate::game::window::{await_modal_end, describe_window, parent_or_self, GameWindow};
use crate::sys::config::{DISPLAY_HEIGHT, DISPLAY_WIDTH};
use crate::sys::logging::{debug_log, LogLevel, LogMode};

const LOCK_GRACE: Duration = Duration::from_millis(500);
const LOCK_POLL: Duration = Duration::from_millis(50);
const CAPTURE_POLL: Duration = Duration::from_millis(2);
const CAPTURE_WAIT: Duration = Duration::from_millis(150);

// HWND is not Send, so the shared handle travels to the watchdog as a raw address.
struct InputLock {
    top: isize,
    held: bool,
    last_activity: Instant,
    capture_deadline: Option<Instant>,
}

fn mark_touch(lock: &Mutex<InputLock>, top: HWND, held: bool) {
    let Ok(mut state) = lock.lock() else {
        return;
    };
    state.held = held;
    state.last_activity = Instant::now();
    if state.top == 0 {
        set_input_enabled(top, false);
        state.top = top.0 as isize;
        debug_log(LogLevel::Info, LogMode::Event, "Minitouch: input locked");
    }
}

// The press is posted, so the game takes capture some time after we return and there is nothing to
// release yet; the watchdog chases it until the deadline instead.
fn await_capture_release(lock: &Mutex<InputLock>) {
    if let Ok(mut state) = lock.lock() {
        state.capture_deadline = Some(Instant::now() + CAPTURE_WAIT);
    }
}

fn settle_touch(lock: &Mutex<InputLock>) {
    let Ok(mut state) = lock.lock() else {
        return;
    };
    if state.top != 0 {
        state.held = false;
        state.last_activity = Instant::now();
    }
}

fn release_lock(lock: &Mutex<InputLock>, restore: bool) {
    let Ok(mut state) = lock.lock() else {
        return;
    };
    if state.top == 0 {
        return;
    }
    if restore {
        set_input_enabled(HWND(state.top as *mut c_void), true);
        debug_log(LogLevel::Info, LogMode::Event, "Minitouch: input unlocked");
    }
    state.top = 0;
    state.capture_deadline = None;
}

// The read loop blocks on MAA's pipe, so neither the grace period nor the capture chase can be timed there.
fn spawn_lock_watchdog(lock: Arc<Mutex<InputLock>>) {
    thread::spawn(move || loop {
        let chasing = {
            let Ok(state) = lock.lock() else {
                return;
            };
            state.capture_deadline.is_some()
        };
        thread::sleep(if chasing { CAPTURE_POLL } else { LOCK_POLL });

        let Ok(mut state) = lock.lock() else {
            return;
        };
        if state.top == 0 {
            continue;
        }
        let top = HWND(state.top as *mut c_void);

        if let Some(deadline) = state.capture_deadline {
            if release_game_capture(top) || Instant::now() >= deadline {
                state.capture_deadline = None;
            }
        }

        // A long press sends no commands for its whole duration, so a held touch gets no deadline.
        if !state.held && state.last_activity.elapsed() >= LOCK_GRACE {
            let idle = state.last_activity.elapsed().as_millis();
            set_input_enabled(top, true);
            state.top = 0;
            state.capture_deadline = None;
            debug_log(LogLevel::Info, LogMode::Event, &format!("Minitouch: input unlocked after {} ms idle", idle));
        }
    });
}

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
    // A daemon that died mid-swipe leaves the window refusing input, so a fresh bind clears it.
    set_input_enabled(parent_or_self(new_win.hwnd), true);
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
    let mut touch_started = Instant::now();

    let lock = Arc::new(Mutex::new(InputLock { top: 0, held: false, last_activity: Instant::now(), capture_deadline: None }));
    spawn_lock_watchdog(Arc::clone(&lock));

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

                let Some(top) = window.as_ref().map(|w| parent_or_self(w.hwnd)) else {
                    is_down = false;
                    release_lock(&lock, false);
                    touch_path.clear();
                    continue;
                };

                if !is_down {
                    let Some(held) = await_modal_end(top) else {
                        window = None;
                        touch_path.clear();
                        continue;
                    };
                    // A resize during the wait lands a new client size, so the one read before it is stale.
                    if !held.is_zero() {
                        window = refresh_window(window, &mut w_width, &mut w_height);
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
                        mark_touch(&lock, top, true);
                        post_message(win.hwnd, WM_MOUSEMOVE, WPARAM(1), LPARAM(pos));
                        post_message(win.hwnd, WM_LBUTTONDOWN, WPARAM(1), LPARAM(pos));
                        await_capture_release(&lock);
                        is_down = true;
                        touch_started = Instant::now();
                        touch_path.clear();
                        touch_path.push((x, y));
                    } else if pos != last_relative_pos {
                        mark_touch(&lock, top, true);
                        touch_path.push((x, y));
                        post_message(win.hwnd, WM_MOUSEMOVE, WPARAM(1), LPARAM(pos));
                    }
                    last_relative_pos = pos;
                } else if is_down {
                    post_message(win.hwnd, WM_MOUSEMOVE, WPARAM(1), LPARAM(last_relative_pos));
                    post_message(win.hwnd, WM_LBUTTONUP, WPARAM(1), LPARAM(last_relative_pos));
                    is_down = false;
                    // Consecutive swipes sit ~260 ms apart, so a touch ending only restarts the grace period.
                    mark_touch(&lock, top, false);

                    let moves = touch_path.len().saturating_sub(1);
                    let elapsed = touch_started.elapsed().as_millis();
                    if moves == 0 {
                        debug_log(LogLevel::Info, LogMode::End, &format!("Minitouch: TAP, {} ms", elapsed));
                    } else {
                        let (ex, ey) = touch_path.last().copied().unwrap_or_default();
                        debug_log(
                            LogLevel::Info,
                            LogMode::End,
                            &format!("Minitouch: SWIPE to ({},{}), {} moves, {} ms", ex, ey, moves, elapsed),
                        );
                    }
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
                    settle_touch(&lock);
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

    release_lock(&lock, true);

    // The loop only ends when the pipe closes, which is MAA letting go.
    debug_log(LogLevel::Info, LogMode::Event, "Minitouch: MAA gone, stopped");
}
