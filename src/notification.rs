use std::{
    env, fs,
    time::{SystemTime, UNIX_EPOCH},
};

use crate::config::*;
use crate::logging::{debug_log, LogLevel, LogMode};
use winrt_toast::{
    content::action::{Action, ActivationType},
    content::text::TextPlacement,
    register, Scenario, Toast, ToastManager,
};

const AUM_ID: &str = "PlayBridge";
const DISPLAY_NAME: &str = "PlayBridge";

// config.rs holds the API endpoint, which a user cannot open in a browser.
const RELEASES_URL: &str = "https://github.com/HX3N/PlayBridge/releases/latest";

const ICON_DATA: &[u8] = include_bytes!("../assets/icon.png");

#[derive(Debug)]
pub enum Notification {
    Screenshot,
    ScreenshotFailed,
    GpgShutdown,
    WindowParked,
    WindowWrongRatio(f32),
    InternalResolution { w: u32, h: u32 },
    UnknownCommand(String),
    Panic(String),
    UpdateAvailable(String),
    UnsupportedClient(String),
    ClientMismatch(String, String),
    GameNotInstalled(String),
    AdbInputUnsupported,
}

impl Notification {
    fn level(&self) -> LogLevel {
        match self {
            Self::Screenshot | Self::GpgShutdown | Self::WindowParked => LogLevel::Info,

            Self::InternalResolution { .. }
            | Self::UnsupportedClient(..)
            | Self::ClientMismatch(..)
            | Self::GameNotInstalled(..)
            | Self::AdbInputUnsupported => LogLevel::Warn,

            // Wrong render aspect ratio distorts everything MAA reads, so it ranks with the hard failures, not warnings.
            Self::ScreenshotFailed | Self::WindowWrongRatio(..) | Self::UnknownCommand(..) | Self::Panic(..) => LogLevel::Error,

            Self::UpdateAvailable(..) => LogLevel::Update,
        }
    }

    fn tag(&self) -> String {
        let debug_str = format!("{:?}", self);
        debug_str.split(['(', '{']).next().unwrap_or(&debug_str).to_string()
    }

    fn body(&self) -> String {
        let (en, kr) = match self {
            Self::Screenshot => (
                "Screenshot saved to the desktop".into(),
                "스크린샷이 바탕화면에 저장됐어요".into(),
            ),
            Self::GpgShutdown => (
                "Google Play Games shut down".into(),
                "Google Play Games 종료".into(),
            ),
            Self::WindowParked => (
                "Moved the window off-screen instead of minimizing".into(),
                "최소화하는 대신 창을 화면 밖으로 옮겼어요".into(),
            ),
            Self::WindowWrongRatio(r) => (
                format!("The current aspect ratio is not supported\nCurrent 16:{:.2} / Recommended 16:9", r),
                format!("현재 화면 비율은 지원하지 않아요\n현재 16:{:.2} / 권장 16:9", r),
            ),
            Self::InternalResolution { w, h } => (
                format!("The current resolution can cause issues\nCurrent {}×{} / Recommended 1280×720", w, h),
                format!("현재 해상도는 문제가 생길 수 있어요\n현재 {}×{} / 권장 1280×720", w, h),
            ),
            Self::ScreenshotFailed => (
                "Screenshot failed; window not found".into(),
                "창을 찾을 수 없어 스크린샷에 실패했어요".into(),
            ),
            Self::UnknownCommand(c) => (
                format!("Unknown command\n{}", c),
                format!("알 수 없는 명령어\n{}", c),
            ),
            Self::Panic(msg) => (
                format!("Fatal error\n{}", msg),
                format!("치명적인 오류 발생\n{}", msg),
            ),
            Self::UpdateAvailable(v) => (
                format!("A new version is available ({})", v),
                format!("신규 버전을 발견했어요 ({})", v),
            ),
            Self::UnsupportedClient(r) => (
                format!("The requested client is not supported\nPlease check 'Client' in MAA 'Game Settings'\nRequested: {}", r),
                format!("요청된 클라이언트는 지원하지 않아요\nMAA '실행 설정'에서 '클라이언트'를 확인해주세요\n요청됨: {}", r),
            ),
            Self::ClientMismatch(r, i) => (
                format!("The requested client does not match the installed version\nPlease check 'Client' in MAA 'Game Settings'\nRequested: {}\nInstalled: {}", r, i),
                format!("요청된 클라이언트와 설치된 클라이언트가 달라요\nMAA '실행 설정'에서 '클라이언트'를 확인해주세요\n요청됨: {}\n설치됨: {}", r, i),
            ),
            Self::GameNotInstalled(p) => (
                format!("The game is not installed in Google Play Games\nPackage: {}", p),
                format!("Google Play Games에 게임이 설치돼 있지 않아요\n패키지: {}", p),
            ),
            Self::AdbInputUnsupported => (
                "ADB Input is not supported\nPlease switch to Minitouch".into(),
                "ADB Input은 지원하지 않아요\nMinitouch로 전환해주세요".into(),
            ),
        };
        if config().client == Client::KR {
            kr
        } else {
            en
        }
    }

    fn cooldown(&self) -> Option<u64> {
        match self {
            Self::WindowWrongRatio(..)
            | Self::AdbInputUnsupported
            | Self::UnsupportedClient(..)
            | Self::ClientMismatch(..)
            | Self::GameNotInstalled(..) => Some(10),
            Self::WindowParked => Some(2),
            _ => None,
        }
    }

    // Protocol activation keeps the button working after this short-lived process exits.
    fn action(&self) -> Option<Action> {
        match self {
            Self::UpdateAvailable(..) => {
                let label = if config().client == Client::KR { "다운로드 하러 가기" } else { "Download page" };
                Some(Action::new(label, RELEASES_URL, "").with_activation_type(ActivationType::Protocol))
            }
            _ => None,
        }
    }

    fn title(&self) -> String {
        let (en, kr) = match self.level() {
            LogLevel::Update => ("🎉 Update", "🎉 업데이트"),
            LogLevel::Info => ("ℹ️ Info", "ℹ️ 정보"),
            LogLevel::Warn => ("⚠️ Warning", "⚠️ 경고"),
            LogLevel::Error => ("⛔ Error", "⛔ 오류"),
        };
        if config().client == Client::KR { kr } else { en }.into()
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
        let _ = fs::write(&icon_path, ICON_DATA);
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

    if let Some(action) = notification.action() {
        toast.action(action);
    }

    let _ = manager.show(&toast);

    let _ = set_registry(&tag, now as u32, REG_PATH_COOLDOWN);
}

fn check_notification_registry(tag: &str, now: u64, cooldown_seconds: u64) -> bool {
    // An absent record reads as 0, so a never-shown tag clears the cooldown on its first call.
    let last_time = get_registry(tag, 0u32, REG_PATH_COOLDOWN) as u64;
    now - last_time >= cooldown_seconds
}
