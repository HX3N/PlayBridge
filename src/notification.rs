use std::{
    env, fs,
    time::{SystemTime, UNIX_EPOCH},
};

use crate::config::{config, get_registry_dword, set_registry_dword, Region, REG_PATH_COOLDOWN, REG_PATH_STATE};
use crate::logging::{debug_log, LogLevel, LogMode};
use winrt_toast::{content::text::TextPlacement, register, Scenario, Toast, ToastManager};

const AUM_ID: &str = "PlayBridge";
const DISPLAY_NAME: &str = "PlayBridge";

const ICON_DATA: &[u8] = include_bytes!("../assets/icon.png");

pub fn get_value(key: &str) -> u32 {
    get_registry_dword(key, REG_PATH_STATE).unwrap_or(0)
}

pub fn set_value(key: &str, value: u32) {
    let _ = set_registry_dword(key, value, REG_PATH_STATE);
}

#[derive(Debug)]
pub enum Notification {
    Screenshot,
    ScreenshotFailed,
    GpgShutdown,
    WindowChanged(u32, u32),
    WindowMinimized,
    WindowTooSmall(u32, u32),
    WindowTooLarge(u32, u32),
    WindowWrongRatio(f32),
    UnknownCommand(String),
    Panic(String),
    UpdateAvailable(String),
}

impl Notification {
    fn level(&self) -> LogLevel {
        match self {
            Self::Screenshot | Self::GpgShutdown | Self::WindowChanged(..) => LogLevel::Info,
            Self::WindowMinimized | Self::WindowTooSmall(..) | Self::WindowTooLarge(..) | Self::WindowWrongRatio(..) => LogLevel::Warn,
            Self::ScreenshotFailed | Self::UnknownCommand(..) | Self::Panic(..) => LogLevel::Error,
            Self::UpdateAvailable(..) => LogLevel::Update,
        }
    }

    fn tag(&self) -> String {
        let debug_str = format!("{:?}", self);
        debug_str.split('(').next().unwrap_or(&debug_str).to_string()
    }

    fn body(&self) -> String {
        match config().region {
            Region::KR => self.body_kr(),
            _ => self.body_en(),
        }
    }

    fn body_en(&self) -> String {
        match self {
            Self::Screenshot => "Screenshot saved to desktop".into(),
            Self::GpgShutdown => "Google Play Games is shutting down".into(),
            Self::WindowChanged(w, h) => format!("Window size changed ({}x{})", w, h),

            Self::WindowMinimized => "Minimized window is not supported".into(),
            Self::WindowTooSmall(w, h) => format!("Window too small ({}x{})\nMAA may not work properly", w, h),
            Self::WindowTooLarge(w, h) => format!("Window too large ({}x{})\nMAA may not work properly", w, h),
            Self::WindowWrongRatio(r) => format!("Wrong aspect ratio (16:{:.2})\nPlease set to 16:9", r),

            Self::ScreenshotFailed => "Screenshot failed, can't find the window".into(),
            Self::UnknownCommand(c) => format!("Unknown command\n{}", c),
            Self::Panic(msg) => format!("Fatal error\n{}", msg),

            Self::UpdateAvailable(v) => format!("New version found ({})\nDownload from GitHub Releases", v),
        }
    }

    fn body_kr(&self) -> String {
        match self {
            Self::Screenshot => "스크린샷이 바탕화면에 저장됐어요".into(),
            Self::GpgShutdown => "Google Play Games가 종료됐어요".into(),
            Self::WindowChanged(w, h) => format!("창 크기가 변경됐어요 ({}x{})", w, h),

            Self::WindowMinimized => "최소화된 창은 지원하지 않아요".into(),
            Self::WindowTooSmall(w, h) => format!("창 크기가 너무 작아요 ({}x{})\nMAA가 제대로 동작하지 않을 수 있어요", w, h),
            Self::WindowTooLarge(w, h) => format!("창 크기가 너무 커요 ({}x{})\nMAA가 제대로 동작하지 않을 수 있어요", w, h),
            Self::WindowWrongRatio(r) => format!("화면 비율이 맞지 않아요 (16:{:.2})\n16:9 비율로 설정해주세요", r),

            Self::ScreenshotFailed => "스크린샷 실패, 창을 찾을 수 없어요".into(),
            Self::UnknownCommand(c) => format!("알 수 없는 명령어\n{}", c),
            Self::Panic(msg) => format!("치명적인 오류 발생\n{}", msg),

            Self::UpdateAvailable(v) => format!("신규 버전을 발견했어요 ({})\nGitHub Releases에서 다운로드해주세요", v),
        }
    }

    fn cooldown(&self) -> Option<u64> {
        match self {
            Self::WindowTooSmall(..) | Self::WindowTooLarge(..) | Self::WindowWrongRatio(..) => Some(10),
            Self::WindowMinimized | Self::WindowChanged(..) => Some(2),
            _ => None,
        }
    }

    fn title(&self) -> String {
        let base = match config().region {
            Region::KR => self.title_kr(),
            _ => self.title_en(),
        };

        if config().debug_capture {
            format!("{} 🛠️", base)
        } else {
            base
        }
    }

    fn title_en(&self) -> String {
        match self.level() {
            LogLevel::Update => "🎉 Update",
            LogLevel::Info => "ℹ️ Info",
            LogLevel::Warn => "⚠️ Warning",
            LogLevel::Error => "⛔ ERROR",
        }
        .into()
    }

    fn title_kr(&self) -> String {
        match self.level() {
            LogLevel::Update => "🎉 업데이트",
            LogLevel::Info => "ℹ️ 정보",
            LogLevel::Warn => "⚠️ 경고",
            LogLevel::Error => "⛔ 오류",
        }
        .into()
    }
}

pub fn display_notification(notification: Notification) {
    let level = notification.level();
    let tag = notification.tag();
    let title = notification.title();
    let body = notification.body();

    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();

    if let Some(cooldown) = notification.cooldown() {
        if !check_notification_registry(&tag, now, cooldown) {
            return;
        }
    }

    debug_log(level, LogMode::Nested, &format!("Notification: {}", body));

    let icon_path = env::temp_dir().join("playbridge.png");

    if !icon_path.exists() {
        fs::write(&icon_path, ICON_DATA).unwrap();
    }

    let _ = register(AUM_ID, DISPLAY_NAME, Some(&icon_path));

    let manager = ToastManager::new(AUM_ID);
    let mut toast = Toast::new();

    toast
        .tag(&tag)
        .text1(title)
        .text2(winrt_toast::content::text::Text::new(&body))
        .text3(winrt_toast::content::text::Text::new(format!("tag: {}", tag)).with_placement(TextPlacement::Attribution));
    toast.scenario(Scenario::Reminder);

    manager.show(&toast).unwrap();

    set_registry_dword(&tag, now as u32, REG_PATH_COOLDOWN).unwrap();
}

fn check_notification_registry(tag: &str, now: u64, cooldown_seconds: u64) -> bool {
    match get_registry_dword(tag, REG_PATH_COOLDOWN) {
        Ok(last_time) => now - last_time as u64 >= cooldown_seconds,
        Err(_) => true,
    }
}
