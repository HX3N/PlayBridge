use std::{
    env,
    fs::{self, OpenOptions},
    io::Write,
    panic,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
    sync::OnceLock,
};

use chrono::Local;

use crate::shared::{KEY_LOG_DEPTH, REG_PATH_STATE};
use crate::sys::config::{get_registry, set_registry};
use crate::sys::notification::{display_notification, Notification};
use crate::sys::process::acquire_lock;

#[derive(Copy, Clone)]
pub enum LogLevel {
    Info,
    Warn,
    Error,
    Update,
}

#[derive(Copy, Clone)]
pub enum LogMode {
    Start,
    End,
    Plain,
}

// A folder deleted mid-run is not recreated, and logging goes quiet until restart.
static DEBUG_FOLDER: OnceLock<PathBuf> = OnceLock::new();
static LOG_PATH: OnceLock<PathBuf> = OnceLock::new();

pub fn get_debug_folder() -> &'static Path {
    DEBUG_FOLDER.get_or_init(|| {
        let exe_dir = env::current_exe().unwrap().parent().unwrap().to_path_buf();
        let folder_path = exe_dir.join("debug");
        let _ = fs::create_dir_all(&folder_path);
        folder_path
    })
}

fn log_path() -> &'static Path {
    LOG_PATH.get_or_init(|| get_debug_folder().join("PlayBridge.log"))
}

const LOG_DEPTH_MUTEX: &str = "Local\\PlayBridgeLogDepth";
const LOG_DEPTH_WAIT_MS: u32 = 200;

fn log_depth() -> u32 {
    get_registry(KEY_LOG_DEPTH, 0u32, REG_PATH_STATE)
}

fn shift_log_depth(delta: i32) {
    let _lock = acquire_lock(LOG_DEPTH_MUTEX, LOG_DEPTH_WAIT_MS);
    let next = (log_depth() as i32 + delta).max(0) as u32;
    let _ = set_registry(KEY_LOG_DEPTH, next, REG_PATH_STATE);
}

/// The stored depth outlives a process killed with a block open, and the reboot after it.
pub fn reset_log_depth() {
    let _lock = acquire_lock(LOG_DEPTH_MUTEX, LOG_DEPTH_WAIT_MS);
    let _ = set_registry(KEY_LOG_DEPTH, 0u32, REG_PATH_STATE);
}

pub fn debug_log(level: LogLevel, mode: LogMode, message: &str) {
    let Ok(mut file) = OpenOptions::new().append(true).create(true).open(log_path()) else {
        return;
    };

    let now = Local::now().format("%Y-%m-%d %H:%M:%S%.3f");
    let level_tag = match level {
        LogLevel::Info | LogLevel::Update => "INF",
        LogLevel::Warn => "WRN",
        LogLevel::Error => "ERR",
    };

    let flat_message = message.replace(['\n', '\t'], " ");

    // Both block lines are drawn at the outer depth, so End shifts before its line and Start after.
    if matches!(mode, LogMode::End) {
        shift_log_depth(-1);
    }
    let indent = "│".repeat(log_depth() as usize);

    let log = match mode {
        LogMode::Plain => format!("[{}][{}]{} {}", now, level_tag, indent, flat_message),
        LogMode::Start => format!("[{}][{}]{}┌ {}", now, level_tag, indent, flat_message),
        LogMode::End => format!("[{}][{}]{}└ {}", now, level_tag, indent, flat_message),
    };

    let _ = writeln!(file, "{}", log);

    if matches!(mode, LogMode::Start) {
        shift_log_depth(1);
    }
}

static REPLIED: AtomicBool = AtomicBool::new(false);

pub fn reply(text: &str) {
    println!("{}", text);
    REPLIED.store(true, Ordering::Relaxed);
    debug_log(LogLevel::Info, LogMode::Plain, &format!("→ {}", text));
}

pub fn log_silent_reply() {
    if !REPLIED.load(Ordering::Relaxed) {
        debug_log(LogLevel::Info, LogMode::Plain, "→");
    }
}

pub fn register_panic_hook() {
    panic::set_hook(Box::new(|info| {
        let msg = info
            .payload()
            .downcast_ref::<&str>()
            .copied()
            .or_else(|| info.payload().downcast_ref::<String>().map(|s| s.as_str()))
            .unwrap_or("Unknown panic message");

        let location = info
            .location()
            .map(|l| format!("{}:{}", l.file(), l.line()))
            .unwrap_or_else(|| "unknown location".into());

        debug_log(LogLevel::Error, LogMode::Plain, &format!("Panic: {} - {}", location, msg));
        display_notification(Notification::Panic(format!("{} - {}", location, msg)));
    }));
}

const MAX_LOG_FILE_SIZE: u64 = 1024 * 1024;
const BACKUP_LOG_NAME: &str = "PlayBridge.bak.log";

pub fn rotate_log() {
    let log_path = log_path();

    if let Ok(metadata) = fs::metadata(log_path) {
        if metadata.len() >= MAX_LOG_FILE_SIZE {
            let backup_path = get_debug_folder().join(BACKUP_LOG_NAME);

            if let Err(e) = fs::rename(log_path, &backup_path) {
                debug_log(LogLevel::Error, LogMode::Plain, &format!("Failed to rotate log: {}", e));
            }
        }
    }
}
