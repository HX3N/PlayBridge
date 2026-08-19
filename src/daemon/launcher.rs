use std::{
    ffi::c_void,
    os::windows::process::CommandExt,
    process::Command,
    thread,
    time::{Duration, Instant},
};

use win_screenshot::prelude::*;
use windows::Win32::Foundation::HWND;

use crate::game::window::{get_window_class, GameWindow};
use crate::shared::{CREATE_NO_WINDOW, DETACHED_PROCESS};
use crate::sys::config::{config, set_client, Client};
use crate::sys::logging::{debug_log, LogLevel, LogMode};
use crate::sys::notification::{display_notification, Notification};
use crate::sys::process::{already_running, mutex_exists};

const WRAPPER_CLASS: &str = "HwndWrapper";

const LOADING_TITLE: &str = "Google Play Games";

pub const LAUNCHER_ARG: &str = "--launcher-daemon";
pub const LAUNCHER_MUTEX: &str = "Local\\PlayBridgeLauncher";
// Upper bound on one launch attempt, after which the game is treated as unreachable and the next
// capture request gets to start a fresh launcher rather than this one retrying forever.
const LAUNCH_TIMEOUT: Duration = Duration::from_secs(180);
const LAUNCH_POLL: Duration = Duration::from_millis(500);
const LAUNCH_RETRY_COOLDOWN: Duration = Duration::from_secs(10);

/// Two clients installed side by side is assumed not to happen, and is not guessed at.
fn adopt_installed_client() -> Option<crate::sys::store::AppRecord> {
    let previous = config().client;

    let mut installed = [Client::KR, Client::JP, Client::EN]
        .into_iter()
        .filter(|&client| client != previous)
        .filter_map(|client| crate::sys::store::app_record(client.package()).map(|record| (client, record)));

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
        debug_log(LogLevel::Info, LogMode::Plain, &format!("Client: intent matches {}", package));
        return true;
    }

    let Some(client) = resolve_client(package) else {
        debug_log(LogLevel::Warn, LogMode::Plain, &format!("Client: unsupported package {}", package));
        display_notification(Notification::UnsupportedClient(package.to_string()));
        return false;
    };

    let current_client = config().client;
    if current_client != Client::Empty && current_client != client {
        debug_log(
            LogLevel::Warn,
            LogMode::Plain,
            &format!("Client: intent {} contradicts configured {}", client.package(), current_client.package()),
        );
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
        debug_log(LogLevel::Info, LogMode::Plain, "Launcher: already running");
        return;
    }
    if GameWindow::find().is_some() {
        debug_log(LogLevel::Info, LogMode::Plain, "Launcher: game already up");
        return;
    }

    let package = config().client.package();
    if package.is_empty() {
        debug_log(LogLevel::Warn, LogMode::Plain, "Launcher: no client set");
        return;
    }

    // Without this the launch URI only raises GPG's own window, which reads as a loading screen forever.
    let Some(app) = crate::sys::store::app_record(package).or_else(adopt_installed_client) else {
        display_notification(Notification::GameNotInstalled(package.to_string()));
        debug_log(LogLevel::Warn, LogMode::Plain, "Launcher: game not installed");
        return;
    };
    debug_log(LogLevel::Info, LogMode::Plain, &format!("Store: {}", app.describe()));

    let package = config().client.package();
    debug_log(LogLevel::Info, LogMode::Start, &format!("Launcher: waiting for {}", package));

    let started = Instant::now();
    let mut last_launch: Option<Instant> = None;
    let mut was_loading = false;
    let mut attempts = 0;

    while started.elapsed() < LAUNCH_TIMEOUT {
        // Detached from MAA, so nothing else would end it.
        if !crate::sys::maa::is_alive() {
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
                debug_log(LogLevel::Warn, LogMode::Plain, "Launcher: loading screen gone without the game, retrying");
                was_loading = false;
                last_launch = None;
            }
            if last_launch.is_none_or(|t| t.elapsed() >= LAUNCH_RETRY_COOLDOWN) {
                last_launch = Some(Instant::now());
                attempts += 1;
                debug_log(LogLevel::Info, LogMode::Plain, &format!("Launcher: launch attempt {}", attempts));
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
