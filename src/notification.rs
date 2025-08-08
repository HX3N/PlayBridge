use std::{
    env, fs,
    time::{SystemTime, UNIX_EPOCH},
};

use crate::config::{get_config, get_registry_dword, set_registry_dword};
use crate::utils::*;
use winrt_toast::{content::text::TextPlacement, register, Scenario, Toast, ToastManager};

const COOLDOWN_SECONDS: u64 = 20;
const AUM_ID: &str = "PlayBridge";
const DISPLAY_NAME: &str = "PlayBridge";

const ICON_DATA: &[u8] = include_bytes!("../assets/icon.png");

fn get_title_display(level: LogLevel) -> String {
    let base_text = match level {
        LogLevel::INFO => "ℹ️ Info",
        LogLevel::WARN => "⚠️ Warning",
        LogLevel::ERROR => "⛔ ERROR",
    };

    if get_config().debug {
        format!("{} 🛠️", base_text)
    } else {
        base_text.to_string()
    }
}

pub fn show_notification(level: LogLevel, body: &str, tag: &str) {
    debug_log(level, &format!("{} , tag: {}", body, tag), None);

    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();

    if !matches!(level, LogLevel::WARN) || check_notification_registry(tag, now, COOLDOWN_SECONDS) {
        let icon_path = env::temp_dir().join("playbridge_icon.png");

        if !icon_path.exists() {
            fs::write(&icon_path, ICON_DATA).expect("Failed to write icon file");
        }

        let _ = register(AUM_ID, DISPLAY_NAME, Some(&icon_path));

        let manager = ToastManager::new(AUM_ID);
        let mut toast = Toast::new();

        toast
            .tag(tag)
            .text1(get_title_display(level))
            .text2(winrt_toast::content::text::Text::new(body))
            .text3(winrt_toast::content::text::Text::new(format!("tag: {}", tag)).with_placement(TextPlacement::Attribution));
        toast.scenario(Scenario::Reminder);

        manager.show(&toast).expect("Failed to show toast");

        set_registry_dword(tag, now as u32).expect("Failed to write to registry");
    }
}

fn check_notification_registry(tag: &str, now: u64, cooldown_seconds: u64) -> bool {
    match get_registry_dword(tag) {
        Ok(last_time) => now - last_time as u64 >= cooldown_seconds,
        Err(_) => true,
    }
}
