use std::{
    env, fs,
    time::{SystemTime, UNIX_EPOCH},
};

use crate::config::*;
use crate::logging::{debug_log, LogLevel, LogMode};
use winrt_toast::{content::text::TextPlacement, register, Scenario, Toast, ToastManager};

const AUM_ID: &str = "PlayBridge";
const DISPLAY_NAME: &str = "PlayBridge";

const ICON_DATA: &[u8] = include_bytes!("../assets/icon.png");

#[derive(Debug)]
pub enum ResizeReason {
    TooSmall,
    TooLarge,
}

#[derive(Debug)]
pub enum Notification {
    Screenshot,
    ScreenshotFailed,
    GpgShutdown,
    WindowMinimized,
    WindowWrongRatio(f32),
    WindowAutoResized { prev_w: u32, prev_h: u32, target_w: u32, target_h: u32, reason: ResizeReason },
    WindowMaximizedRestored,
    UnknownCommand(String),
    Panic(String),
    UpdateAvailable(String),
    UnsupportedClient(String),
    ClientMismatch(String, String),
}

impl Notification {
    fn level(&self) -> LogLevel {
        match self {
            Self::Screenshot | Self::GpgShutdown => LogLevel::Info,

            Self::WindowMinimized
            | Self::WindowWrongRatio(..)
            | Self::UnsupportedClient(..)
            | Self::ClientMismatch(..)
            | Self::WindowAutoResized { .. }
            | Self::WindowMaximizedRestored => LogLevel::Warn,

            Self::ScreenshotFailed | Self::UnknownCommand(..) | Self::Panic(..) => LogLevel::Error,

            Self::UpdateAvailable(..) => LogLevel::Update,
        }
    }

    fn tag(&self) -> String {
        let debug_str = format!("{:?}", self);
        debug_str.split(|c| c == '(' || c == '{').next().unwrap_or(&debug_str).to_string()
    }

    fn body(&self) -> String {
        match config().client {
            Client::KR => self.body_kr(),
            _ => self.body_en(),
        }
    }

    fn body_en(&self) -> String {
        match self {
            Self::Screenshot => "Screenshot saved to the desktop".into(),
            Self::GpgShutdown => "Google Play Games has shut down".into(),

            Self::WindowMinimized => "Minimized windows are not supported".into(),
            Self::WindowWrongRatio(r) => format!("Incorrect aspect ratio (16:{:.2})\nPlease set it to 16:9", r),
            Self::WindowAutoResized { prev_w, prev_h, target_w, target_h, reason } => {
                let reason_str = match reason {
                    ResizeReason::TooSmall => "too small",
                    ResizeReason::TooLarge => "too large",
                };
                format!("Window was {} ({}x{}).\nAuto-resized to {}x{}", reason_str, prev_w, prev_h, target_w, target_h)
            }
            Self::WindowMaximizedRestored => "Window was too large; restored from maximized state".into(),

            Self::ScreenshotFailed => "Screenshot failed; window not found".into(),
            Self::UnknownCommand(c) => format!("Unknown command\n{}", c),
            Self::Panic(msg) => format!("Fatal error\n{}", msg),

            Self::UpdateAvailable(v) => format!("A new version is available ({})\nDownload it from GitHub Releases", v),
            Self::UnsupportedClient(r) => {
                format!("The requested client is not supported\nPlease check 'Client' in MAA 'Game Settings'\nRequested: {}", r)
            }
            Self::ClientMismatch(r, i) => {
                format!("The requested client does not match the installed version\nPlease check 'Client' in MAA 'Game Settings'\nRequested: {}\nInstalled: {}", r, i)
            }
        }
    }

    fn body_kr(&self) -> String {
        match self {
            Self::Screenshot => "스크린샷이 바탕화면에 저장됐어요".into(),
            Self::GpgShutdown => "Google Play Games가 종료됐어요".into(),

            Self::WindowMinimized => "최소화된 창은 지원하지 않아요".into(),
            Self::WindowWrongRatio(r) => format!("화면 비율이 맞지 않아요 (16:{:.2})\n16:9 비율로 설정해주세요", r),
            Self::WindowAutoResized { prev_w, prev_h, target_w, target_h, reason } => {
                let reason_str = match reason {
                    ResizeReason::TooSmall => "너무 작아요",
                    ResizeReason::TooLarge => "너무 커요",
                };
                format!("창 크기가 {} ({}x{})\n{}x{}로 자동 조절됐어요", reason_str, prev_w, prev_h, target_w, target_h)
            }
            Self::WindowMaximizedRestored => "창이 너무 커서 최대화 상태를 해제했어요".into(),

            Self::ScreenshotFailed => "스크린샷 실패, 창을 찾을 수 없어요".into(),
            Self::UnknownCommand(c) => format!("알 수 없는 명령어\n{}", c),
            Self::Panic(msg) => format!("치명적인 오류 발생\n{}", msg),

            Self::UpdateAvailable(v) => format!("신규 버전을 발견했어요 ({})\nGitHub Releases에서 다운로드해주세요", v),
            Self::UnsupportedClient(r) => {
                format!("요청된 클라이언트는 지원하지 않아요\nMAA '실행 설정'에서 '클라이언트'를 확인해주세요\n요청됨: {}", r)
            }
            Self::ClientMismatch(r, i) => {
                format!("요청된 클라이언트와 설치된 클라이언트가 달라요\nMAA '실행 설정'에서 '클라이언트'를 확인해주세요\n요청됨: {}\n설치됨: {}", r, i)
            }
        }
    }

    fn cooldown(&self) -> Option<u64> {
        match self {
            Self::WindowWrongRatio(..) => Some(10),
            Self::WindowMinimized | Self::WindowAutoResized { .. } | Self::WindowMaximizedRestored => Some(2),
            _ => None,
        }
    }

    fn title(&self) -> String {
        match config().client {
            Client::KR => self.title_kr(),
            _ => self.title_en(),
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
