use std::{
    env, fs,
    time::{SystemTime, UNIX_EPOCH},
};

use crate::config::{config, get_registry_dword, set_registry_dword};
use crate::utils::*;
use winrt_toast::{content::text::TextPlacement, register, Scenario, Toast, ToastManager};

const COOLDOWN_SECONDS: u64 = 20;
const AUM_ID: &str = "PlayBridge";
const DISPLAY_NAME: &str = "PlayBridge";

const ICON_DATA: &[u8] = include_bytes!("../assets/icon.png");

fn get_notification_body(tag: &str, args: &[&str]) -> String {
    match tag {
        "screenshot" => "Screenshot saved to desktop!".to_string(),
        "gpg_shutdown" => "Shutdown Google Play Games".to_string(),
        "gpg_start_failed" => format!("Failed to start Google Play Games or detect\nTarget regex: {}", args[0]),
        "unknown_cmd" => format!("Unknown command!\n{}", args[0]),
        "window_minimized" => "Minimized window is not supported".to_string(),
        "wrong_ratio" => format!("Aspect ratio is not 16:9 (16:{})", args[0]),
        "window_too_small" => format!("Window size is too small ({}x{})", args[0], args[1]),
        "window_info" => format!("Window size info ({}x{})", args[0], args[1]),
        "window_changed" => format!("Window size changed ({}x{})", args[0], args[1]),
        "storage_warning" => format!("PlayBridge folder size is {}MB!\nPlease be careful of high storage usage", args[0]),
        "extras_stop" => "Stopping Extras".to_string(),
        "panic" => format!("PANIC at {}\n{}", args[0], args[1]),
        _ => format!("Unmatched tag: {}", tag),
    }
}

fn get_title_display(level: LogLevel) -> String {
    let base_text = match level {
        LogLevel::INFO => "ℹ️ Info",
        LogLevel::WARN => "⚠️ Warning",
        LogLevel::ERROR => "⛔ ERROR",
    };

    if config().debug {
        format!("{} 🛠️", base_text)
    } else {
        base_text.to_string()
    }
}

pub fn display_notification(level: LogLevel, tag: &str, args: &[&str]) {
    let body = get_notification_body(tag, args);

    debug_log(level, &format!("{} , tag: {}", body, tag), None);

    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();

    if !matches!(level, LogLevel::WARN) || check_notification_registry(tag, now, COOLDOWN_SECONDS) {
        let icon_path = env::temp_dir().join("playbridge_icon.png");

        if !icon_path.exists() {
            fs::write(&icon_path, ICON_DATA).unwrap();
        }

        let _ = register(AUM_ID, DISPLAY_NAME, Some(&icon_path));

        let manager = ToastManager::new(AUM_ID);
        let mut toast = Toast::new();

        toast
            .tag(tag)
            .text1(get_title_display(level))
            .text2(winrt_toast::content::text::Text::new(&body))
            .text3(winrt_toast::content::text::Text::new(format!("tag: {}", tag)).with_placement(TextPlacement::Attribution));
        toast.scenario(Scenario::Reminder);

        manager.show(&toast).unwrap();

        set_registry_dword(tag, now as u32, &config().notification_path).unwrap();
    }
}

fn check_notification_registry(tag: &str, now: u64, cooldown_seconds: u64) -> bool {
    match get_registry_dword(tag, &config().notification_path) {
        Ok(last_time) => now - last_time as u64 >= cooldown_seconds,
        Err(_) => true,
    }
}
