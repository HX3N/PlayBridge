use std::{
    env, fs, io,
    time::{SystemTime, UNIX_EPOCH},
};

use crate::utils::*;
use crate::CONFIG;
use winreg::{enums::*, RegKey};
use winrt_toast::{content::text::TextPlacement, register, Scenario, Toast, ToastManager};

pub const NOTIFICATION_REGISTRY_PATH: &str = r"Software\PlayBridge ADB";
const COOLDOWN_SECONDS: u64 = 20;
const AUM_ID: &str = "PlayBridge ADB";
const DISPLAY_NAME: &str = "PlayBridge ADB";

const ICON_DATA: &[u8] = include_bytes!("../assets/icon.png");

pub fn get_registry_dword(key_name: &str) -> io::Result<u32> {
    let hklm = RegKey::predef(HKEY_CURRENT_USER);
    let key = hklm.open_subkey(NOTIFICATION_REGISTRY_PATH)?;
    key.get_value(key_name)
}

pub fn set_registry_dword(key_name: &str, value: u32) -> io::Result<()> {
    let hklm = RegKey::predef(HKEY_CURRENT_USER);
    let (key, _) = hklm.create_subkey(NOTIFICATION_REGISTRY_PATH)?;
    key.set_value(key_name, &value)?;
    Ok(())
}

fn get_title_display(level: LogLevel) -> &'static str {
    match level {
        LogLevel::INFO => "ℹ️ Info",
        LogLevel::WARN => "⚠️ Warning",
        LogLevel::ERROR => "⛔ ERROR",
    }
}

pub fn show_notification(level: LogLevel, body: &str, tag: &str) {
    debug_log(level, &format!("{} (tag: {})", body, tag), None);

    if !CONFIG.notification {
        return;
    }

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

        let hklm = RegKey::predef(HKEY_CURRENT_USER);
        let (key, _) = hklm.create_subkey(NOTIFICATION_REGISTRY_PATH).expect("Failed to create or open registry key");
        key.set_value(tag, &now).expect("Failed to write to registry");
    }
}

fn check_notification_registry(tag: &str, now: u64, cooldown_seconds: u64) -> bool {
    let hklm = RegKey::predef(HKEY_CURRENT_USER);
    let key = hklm.open_subkey_with_flags(NOTIFICATION_REGISTRY_PATH, KEY_READ).ok();
    match key {
        Some(key) => {
            let last_time: u64 = key.get_value(tag).unwrap_or(0);
            now - last_time >= cooldown_seconds
        }
        None => true,
    }
}
