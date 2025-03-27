use std::{
    env, fs,
    time::{SystemTime, UNIX_EPOCH},
};

use winreg::{enums::*, RegKey};
use winrt_toast::{content::text::TextPlacement, register, Scenario, Toast, ToastManager};

pub const NOTIFICATION_REGISTRY_PATH: &str = r"Software\PlayBridge\Notification";
const COOLDOWN_SECONDS: u64 = 20; // Minimum time between notifications of the same type
const AUM_ID: &str = "PlayBridge"; // Application User Model ID for Windows notifications
const DISPLAY_NAME: &str = "PlayBridge";

pub const INFO: &str = "ℹ️ Info";
pub const WARN: &str = "⚠️ Warning";
pub const ERR: &str = "⛔ ERROR";
pub const DEBUG: &str = "🛠️ Debug";

const ICON_DATA: &[u8] = include_bytes!("../assets/icon.png");

pub fn show_notification(title: &str, body: &str, tag: &str) {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
    if title != WARN || check_notification_registry(tag, now, COOLDOWN_SECONDS) {
        let icon_path = env::temp_dir().join("playbridge_icon.png");

        if !icon_path.exists() {
            fs::write(&icon_path, ICON_DATA).expect("Failed to write icon file");
        }

        let _ = register(AUM_ID, DISPLAY_NAME, Some(&icon_path));

        let manager = ToastManager::new(AUM_ID);
        let mut toast = Toast::new();
        toast
            .tag(tag)
            .text1(title)
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

pub fn check_and_update_resolution(width: u32, height: u32) {
    let hklm = RegKey::predef(HKEY_CURRENT_USER);
    let (key, _) = hklm.create_subkey(NOTIFICATION_REGISTRY_PATH).expect("Failed to create or open registry key");

    let stored_width: u32 = key.get_value("resolution_width").unwrap_or(0);
    let stored_height: u32 = key.get_value("resolution_height").unwrap_or(0);

    if stored_width != width || stored_height != height {
        key.set_value("resolution_width", &width).expect("Failed to write width to registry");
        key.set_value("resolution_height", &height).expect("Failed to write height to registry");

        let (title, body, tag) = if stored_width == 0 || stored_height == 0 {
            (INFO, &format!("Resolution info ({}x{})", width, height), "resolution_init")
        } else {
            (INFO, &format!("Resolution changed ({}x{})", width, height), "resolution_changed")
        };

        show_notification(title, body, tag);
    }
}
