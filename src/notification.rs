use std::{
    env, fs,
    time::{SystemTime, UNIX_EPOCH},
};

use crate::config::{config, get_registry_dword, set_registry_dword};
use crate::logging::{debug_log, LogLevel, LogMode};
use winrt_toast::{content::text::TextPlacement, register, Scenario, Toast, ToastManager};

const AUM_ID: &str = "PlayBridge";
const DISPLAY_NAME: &str = "PlayBridge";

const REG_PATH_NOTIFICATION: &str = r"Software\PlayBridge\notification";
const ICON_DATA: &[u8] = include_bytes!("../assets/icon.png");

pub fn get_value(key: &str) -> u32 {
    get_registry_dword(key, REG_PATH_NOTIFICATION).unwrap_or(0)
}

pub fn set_value(key: &str, value: u32) {
    let _ = set_registry_dword(key, value, REG_PATH_NOTIFICATION);
}

#[derive(Debug)]
pub enum Notification {
    Screenshot,
    ScreenshotFailed,
    GpgShutdown,
    WindowInfo(u32, u32),
    WindowChanged(u32, u32, u32, u32),
    WindowMinimized,
    WindowTooSmall(u32, u32),
    WindowTooLarge(u32, u32),
    WindowWrongRatio(f32),
    UnknownCommand(String),
    Panic(String),
}

impl Notification {
    fn level(&self) -> LogLevel {
        match self {
            Self::Screenshot | Self::GpgShutdown | Self::WindowInfo(..) | Self::WindowChanged(..) => LogLevel::Info,
            Self::WindowMinimized | Self::WindowTooSmall(..) | Self::WindowTooLarge(..) | Self::WindowWrongRatio(..) => LogLevel::Warn,
            Self::ScreenshotFailed | Self::UnknownCommand(..) | Self::Panic(..) => LogLevel::Error,
        }
    }

    fn tag(&self) -> String {
        let debug_str = format!("{:?}", self);
        debug_str.split('(').next().unwrap_or(&debug_str).to_string()
    }

    fn body(&self) -> String {
        match self {
            Self::Screenshot => "Screenshot saved to desktop".into(),
            Self::ScreenshotFailed => "Screenshot failed, can't find the window".into(),
            Self::GpgShutdown => "Google Play Games is shutting down".into(),
            Self::WindowInfo(w, h) => format!("Window size: {}x{}", w, h),
            Self::WindowChanged(old_w, old_h, w, h) => format!("Window size changed: {}x{} to {}x{}", old_w, old_h, w, h),
            Self::WindowMinimized => "Minimized window is not supported".into(),
            Self::WindowTooSmall(w, h) => format!("Window too small: {}x{}\nBelow minimum 1280x720", w, h),
            Self::WindowTooLarge(w, h) => format!("Window too large: {}x{}\nExceeds maximum 1920x1080", w, h),
            Self::WindowWrongRatio(r) => format!("Window ratio: 16:{:.2} (expected 16:9)", r),
            Self::UnknownCommand(c) => format!("Unknown command:\n{}", c),
            Self::Panic(msg) => format!("PANIC:\n{}", msg),
        }
    }

    fn cooldown(&self) -> Option<u64> {
        match self {
            Self::WindowMinimized | Self::WindowTooSmall(..) | Self::WindowTooLarge(..) | Self::WindowWrongRatio(..) => Some(20),
            Self::WindowChanged(..) => Some(1),
            _ => None,
        }
    }
}

fn get_title_display(level: LogLevel) -> String {
    let base_text = match level {
        LogLevel::Info => "ℹ️ Info",
        LogLevel::Warn => "⚠️ Warning",
        LogLevel::Error => "⛔ ERROR",
    };

    if config().debug_capture {
        format!("{} 🛠️", base_text)
    } else {
        base_text.to_string()
    }
}

pub fn display_notification(notification: Notification) {
    let level = notification.level();
    let tag = notification.tag();
    let body = notification.body();

    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();

    if let Some(cooldown) = notification.cooldown() {
        if !check_notification_registry(&tag, now, cooldown) {
            return;
        }
    }

    debug_log(level, LogMode::Nested, &body);

    let icon_path = env::temp_dir().join("playbridge.png");

    if !icon_path.exists() {
        fs::write(&icon_path, ICON_DATA).unwrap();
    }

    let _ = register(AUM_ID, DISPLAY_NAME, Some(&icon_path));

    let manager = ToastManager::new(AUM_ID);
    let mut toast = Toast::new();

    toast
        .tag(&tag)
        .text1(get_title_display(level))
        .text2(winrt_toast::content::text::Text::new(&body))
        .text3(winrt_toast::content::text::Text::new(format!("tag: {}", tag)).with_placement(TextPlacement::Attribution));
    toast.scenario(Scenario::Reminder);

    manager.show(&toast).unwrap();

    set_registry_dword(&tag, now as u32, REG_PATH_NOTIFICATION).unwrap();
}

fn check_notification_registry(tag: &str, now: u64, cooldown_seconds: u64) -> bool {
    match get_registry_dword(tag, REG_PATH_NOTIFICATION) {
        Ok(last_time) => now - last_time as u64 >= cooldown_seconds,
        Err(_) => true,
    }
}
