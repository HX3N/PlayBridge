use std::{
    env,
    fs::{self, OpenOptions},
    io::Write,
    panic,
    path::{Path, PathBuf},
    sync::OnceLock,
};

use chrono::Local;

use crate::notification::{display_notification, Notification};

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
    Nested,
    Event,
    End,
}

// Resolved once per process, so a per-line exe lookup and directory create are not paid on every log.
// Trade-off: a folder deleted mid-run is not recreated, and logging goes quiet until restart.
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

    let flat_message = message.replace('\n', " ");

    let log = match mode {
        LogMode::Start => format!("[{}][{}] {}", now, level_tag, flat_message),
        LogMode::Nested => format!("[{}][{}] │ {}", now, level_tag, flat_message),
        LogMode::Event => format!("[{}][{}] X {}", now, level_tag, flat_message),
        LogMode::End => format!("[{}][{}] └ {}", now, level_tag, flat_message),
    };

    let _ = writeln!(file, "{}", log);
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
                debug_log(LogLevel::Error, LogMode::Start, &format!("Failed to rotate log: {}", e));
            }
        }
    }
}
