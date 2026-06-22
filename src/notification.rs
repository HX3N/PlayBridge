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
pub enum Notification {
    Screenshot,
    ScreenshotFailed,
    GpgShutdown,
    WindowMinimized,
    WindowWrongRatio(f32),
    InternalResolution { w: u32, h: u32 },
    UnknownCommand(String),
    Panic(String),
    UpdateAvailable(String),
    UnsupportedClient(String),
    ClientMismatch(String, String),
    AdbInputUnsupported,
    MinitouchStopped,
    WgcDaemonStopped,
}

impl Notification {
    fn level(&self) -> LogLevel {
        match self {
            Self::Screenshot | Self::GpgShutdown | Self::MinitouchStopped | Self::WgcDaemonStopped => LogLevel::Info,

            Self::WindowMinimized
            | Self::InternalResolution { .. }
            | Self::UnsupportedClient(..)
            | Self::ClientMismatch(..)
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
                "Google Play Games has shut down".into(),
                "Google Play Games가 종료됐어요".into(),
            ),

            Self::WindowMinimized => (
                "Minimized windows are not supported".into(),
                "최소화된 창은 지원하지 않아요".into(),
            ),
            Self::WindowWrongRatio(r) => (
                format!("Incorrect aspect ratio (16:{:.2})\nPlease set it to 16:9", r),
                format!("화면 비율이 맞지 않아요 (16:{:.2})\n16:9 비율로 설정해주세요", r),
            ),
            Self::InternalResolution { w, h } => (
                format!("Google Play Games internal resolution is {}×{}\nRecommended: 1280×720", w, h),
                format!("Google Play Games 내부 해상도 {}×{}\n권장 해상도 1280×720", w, h),
            ),
            Self::ScreenshotFailed => (
                "Screenshot failed; window not found".into(),
                "스크린샷 실패, 창을 찾을 수 없어요".into(),
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
                format!("A new version is available ({})\nDownload it from GitHub Releases", v),
                format!("신규 버전을 발견했어요 ({})\nGitHub Releases에서 다운로드해주세요", v),
            ),
            Self::UnsupportedClient(r) => (
                format!("The requested client is not supported\nPlease check 'Client' in MAA 'Game Settings'\nRequested: {}", r),
                format!("요청된 클라이언트는 지원하지 않아요\nMAA '실행 설정'에서 '클라이언트'를 확인해주세요\n요청됨: {}", r),
            ),
            Self::ClientMismatch(r, i) => (
                format!("The requested client does not match the installed version\nPlease check 'Client' in MAA 'Game Settings'\nRequested: {}\nInstalled: {}", r, i),
                format!("요청된 클라이언트와 설치된 클라이언트가 달라요\nMAA '실행 설정'에서 '클라이언트'를 확인해주세요\n요청됨: {}\n설치됨: {}", r, i),
            ),
            Self::AdbInputUnsupported => (
                "ADB Input is no longer supported\nPlease switch to Minitouch".into(),
                "ADB Input은 지원하지 않습니다\nMinitouch로 전환해주세요".into(),
            ),
            Self::MinitouchStopped => (
                "Minitouch daemon stopped".into(),
                "Minitouch 데몬 종료".into(),
            ),
            Self::WgcDaemonStopped => (
                "WGC capture daemon stopped".into(),
                "WGC 캡처 데몬 종료".into(),
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
            Self::WindowWrongRatio(..) | Self::AdbInputUnsupported => Some(10),
            Self::WindowMinimized => Some(2),
            _ => None,
        }
    }

    fn title(&self) -> String {
        let (en, kr) = match self.level() {
            LogLevel::Update => ("🎉 Update", "🎉 업데이트"),
            LogLevel::Info => ("ℹ️ Info", "ℹ️ 정보"),
            LogLevel::Warn => ("⚠️ Warning", "⚠️ 경고"),
            LogLevel::Error => ("⛔ ERROR", "⛔ 오류"),
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

    let _ = manager.show(&toast);

    let _ = set_registry(&tag, now as u32, REG_PATH_COOLDOWN);
}

fn check_notification_registry(tag: &str, now: u64, cooldown_seconds: u64) -> bool {
    // An absent record reads as 0, so a never-shown tag clears the cooldown on its first call.
    let last_time = get_registry(tag, 0u32, REG_PATH_COOLDOWN) as u64;
    now - last_time >= cooldown_seconds
}
