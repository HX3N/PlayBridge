use std::{
    ffi::c_void,
    os::windows::process::CommandExt,
    process::Command,
    thread,
    time::{Duration, Instant},
};

use win_screenshot::prelude::*;
use windows::core::PCWSTR;
use windows::Win32::{
    Foundation::{HWND, RECT},
    UI::WindowsAndMessaging::{FindWindowExW, GetClassNameW, GetClientRect, GetParent, IsIconic, ShowWindow, SW_RESTORE},
};

use crate::config::{config, set_client, Client};
use crate::logging::{debug_log, LogLevel, LogMode};
use crate::notification::{display_notification, Notification};
use crate::shared::{CREATE_NO_WINDOW, DETACHED_PROCESS};
use crate::wgc::{already_running, mutex_exists};

const WRAPPER_CLASS: &str = "HwndWrapper";
const CROSVM_CLASS: &str = "CROSVM_1";

const LOADING_TITLE: &str = "Google Play Games";

const WINDOW_RESTORE_DELAY_MS: u64 = 300;

pub const LAUNCHER_ARG: &str = "--launcher-daemon";
const LAUNCHER_MUTEX: &str = "Local\\PlayBridgeLauncher";
// Upper bound on one launch attempt, after which the game is treated as unreachable and the next
// capture request gets to start a fresh launcher rather than this one retrying forever.
const LAUNCH_TIMEOUT: Duration = Duration::from_secs(180);
const LAUNCH_POLL: Duration = Duration::from_millis(500);
const LAUNCH_RETRY_COOLDOWN: Duration = Duration::from_secs(10);

pub fn parent_or_self(hwnd: HWND) -> HWND {
    unsafe { GetParent(hwnd).ok().filter(|p| !p.0.is_null()).unwrap_or(hwnd) }
}

pub struct GameWindow {
    pub hwnd: HWND,
}

impl GameWindow {
    pub fn find() -> Option<Self> {
        let windows = window_list().ok()?;
        let current_title = config().client.title();

        if !current_title.is_empty() {
            if let Some(hwnd) = match_window_by_title(&windows, current_title) {
                return Some(Self { hwnd });
            }
        }

        for client in [Client::KR, Client::JP, Client::EN] {
            if client.title() != current_title {
                if let Some(hwnd) = match_window_by_title(&windows, client.title()) {
                    adopt_running_client(client);
                    return Some(Self { hwnd });
                }
            }
        }

        None
    }

    pub fn restore(&self) {
        let target_hwnd = parent_or_self(self.hwnd);

        if unsafe { IsIconic(target_hwnd).as_bool() } {
            unsafe { _ = ShowWindow(target_hwnd, SW_RESTORE) };
            thread::sleep(Duration::from_millis(WINDOW_RESTORE_DELAY_MS));
        }
    }

    pub fn get_client_size(&self) -> (i32, i32) {
        let mut rect = RECT::default();
        if unsafe { GetClientRect(self.hwnd, &mut rect) }.is_ok() {
            (rect.right - rect.left, rect.bottom - rect.top)
        } else {
            (0, 0)
        }
    }
}

/// Notification language is picked by the stored client, so the update lands before the toast.
fn adopt_running_client(client: Client) {
    let previous = config().client;
    set_client(client);

    if previous != Client::Empty {
        display_notification(Notification::ClientMismatch(previous.package().to_string(), client.package().to_string()));
    }
}

/// Two clients installed side by side is assumed not to happen, and is not guessed at.
fn adopt_installed_client() -> Option<crate::store::AppRecord> {
    let previous = config().client;

    let mut installed = [Client::KR, Client::JP, Client::EN]
        .into_iter()
        .filter(|&client| client != previous)
        .filter_map(|client| crate::store::app_record(client.package()).map(|record| (client, record)));

    let (client, record) = installed.next()?;
    if installed.next().is_some() {
        return None;
    }

    set_client(client);
    display_notification(Notification::ClientMismatch(previous.package().to_string(), client.package().to_string()));
    Some(record)
}

/// False means the intent contradicts the client MAA already told us about, so the caller
/// should not go on to launch anything.
pub fn apply_intent_package(intent: &str) -> bool {
    let package = intent.split('/').next().unwrap_or(intent);

    if config().client.package() == package {
        debug_log(LogLevel::Info, LogMode::Nested, &format!("Client: intent matches {}", package));
        return true;
    }

    let Some(client) = resolve_client(package) else {
        display_notification(Notification::UnsupportedClient(package.to_string()));
        return false;
    };

    let current_client = config().client;
    if current_client != Client::Empty && current_client != client {
        display_notification(Notification::ClientMismatch(client.package().to_string(), current_client.package().to_string()));
        return false;
    }

    set_client(client);
    true
}

/// Safe to call on every request: a launcher already at work is left to finish.
pub fn ensure_launcher() {
    if mutex_exists(LAUNCHER_MUTEX) {
        return;
    }
    if let Ok(exe) = std::env::current_exe() {
        let _ = Command::new(exe).arg(LAUNCHER_ARG).creation_flags(DETACHED_PROCESS | CREATE_NO_WINDOW).spawn();
    }
}

/// Its lifetime is the "game is starting" signal, so nothing else tracks launch state —
/// capture and input simply find no window until it exits.
pub fn run_launcher_daemon() {
    if already_running(LAUNCHER_MUTEX) {
        debug_log(LogLevel::Info, LogMode::End, "Launcher: already running");
        return;
    }
    if GameWindow::find().is_some() {
        debug_log(LogLevel::Info, LogMode::End, "Launcher: game already up");
        return;
    }

    let package = config().client.package();
    if package.is_empty() {
        debug_log(LogLevel::Warn, LogMode::End, "Launcher: no client set");
        return;
    }

    // Without this the launch URI only raises GPG's own window, which reads as a loading screen forever.
    let Some(app) = crate::store::app_record(package).or_else(adopt_installed_client) else {
        display_notification(Notification::GameNotInstalled(package.to_string()));
        debug_log(LogLevel::Warn, LogMode::End, "Launcher: game not installed");
        return;
    };
    debug_log(LogLevel::Info, LogMode::Nested, &format!("Store: {}", app.describe()));

    let package = config().client.package();
    debug_log(LogLevel::Info, LogMode::Nested, &format!("Launcher: waiting for {}", package));

    let started = Instant::now();
    let mut last_launch: Option<Instant> = None;
    let mut was_loading = false;
    let mut attempts = 0;

    while started.elapsed() < LAUNCH_TIMEOUT {
        // Detached from MAA, so nothing else would end it.
        if !crate::maa::is_alive() {
            debug_log(LogLevel::Info, LogMode::End, "Launcher: MAA gone, stopped");
            return;
        }

        if GameWindow::find().is_some() {
            debug_log(LogLevel::Info, LogMode::End, &format!("Launcher: game ready in {} ms", started.elapsed().as_millis()));
            return;
        }

        if is_loading_screen_active() {
            was_loading = true;
        } else {
            // The loading screen going away without the game appearing means the launch died partway.
            if was_loading {
                debug_log(LogLevel::Warn, LogMode::Nested, "Launcher: loading screen gone without the game, retrying");
                was_loading = false;
                last_launch = None;
            }
            if last_launch.is_none_or(|t| t.elapsed() >= LAUNCH_RETRY_COOLDOWN) {
                last_launch = Some(Instant::now());
                attempts += 1;
                debug_log(LogLevel::Info, LogMode::Nested, &format!("Launcher: launch attempt {}", attempts));
                let _ = open::that(format!("googleplaygames://launch/?id={}&pid=1", package));
            }
        }

        thread::sleep(LAUNCH_POLL);
    }

    debug_log(LogLevel::Warn, LogMode::End, "Launcher: gave up waiting for the game");
}

fn resolve_client(package: &str) -> Option<Client> {
    [Client::KR, Client::JP, Client::EN]
        .into_iter()
        .find(|&client| package.starts_with(client.package()))
}

fn match_window_by_title(windows: &[HwndName], title: &str) -> Option<HWND> {
    windows.iter().filter(|w| w.window_name.starts_with(title)).find_map(|win| {
        let hwnd = HWND(win.hwnd as usize as *mut c_void);
        let class_name = get_window_class(hwnd)?;

        // Old: CROSVM_1 > subWin
        // New: HwndWrapper > CROSVM_1 > subWin
        if class_name == CROSVM_CLASS {
            Some(hwnd)
        } else if class_name.starts_with(WRAPPER_CLASS) {
            find_crosvm_child(hwnd)
        } else {
            None
        }
    })
}

fn find_crosvm_child(parent_hwnd: HWND) -> Option<HWND> {
    let crosvm_class_wide: Vec<u16> = CROSVM_CLASS.encode_utf16().chain(Some(0)).collect();

    unsafe {
        FindWindowExW(Some(parent_hwnd), None, PCWSTR(crosvm_class_wide.as_ptr()), None)
            .ok()
            .filter(|h| !h.0.is_null())
    }
}

fn is_loading_screen_active() -> bool {
    window_list().unwrap_or_default().into_iter().any(|i| {
        if i.window_name == LOADING_TITLE {
            let hwnd = HWND(i.hwnd as usize as *mut c_void);
            if let Some(class_name) = get_window_class(hwnd) {
                return class_name.starts_with(WRAPPER_CLASS);
            }
        }
        false
    })
}

fn get_window_class(hwnd: HWND) -> Option<String> {
    let mut buffer: [u16; 256] = [0; 256];
    unsafe {
        let len = GetClassNameW(hwnd, &mut buffer);
        if len > 0 {
            Some(String::from_utf16_lossy(&buffer[..len as usize]))
        } else {
            None
        }
    }
}

/// Bound-window log summary. client is the child's physical size the WGC frame pool binds to, so a change forces a rebind.
pub(crate) fn describe_window(child: HWND, top: Option<HWND>) -> String {
    let class = get_window_class(child).unwrap_or_default();
    let (w, h) = GameWindow { hwnd: child }.get_client_size();
    match top {
        Some(t) => format!("top=0x{:X} child=0x{:X} class={} client={}x{}", t.0 as usize, child.0 as usize, class, w, h),
        None => format!("child=0x{:X} class={} client={}x{}", child.0 as usize, class, w, h),
    }
}
