use std::{
    env,
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};
use winreg::{enums::*, RegKey};
use winrt_toast::{content::text::TextPlacement, register, Scenario, Text, Toast, ToastManager};

pub const REGISTRY_PATH: &str = r"Software\PlayBridge";
const COOLDOWN_SECONDS: u64 = 20; // Notification cooltime

fn check_notification_registry(tag: &str, now: u64, cooldown_seconds: u64) -> bool {
    let hklm = RegKey::predef(HKEY_CURRENT_USER);
    let key = hklm.open_subkey_with_flags(REGISTRY_PATH, KEY_READ).ok();

    match key {
        Some(key) => {
            let last_time: u64 = key.get_value(tag).unwrap_or(0);
            now - last_time >= cooldown_seconds
        }
        None => true,
    }
}

fn get_notification_details(cause: &str, spec: Option<&str>) -> (String, String, String) {
    match cause {
        "start_arknights" => ("명일방주 실행".to_string(), "".to_string(), "start_arknights".to_string()),
        "shutdown_arknights" => ("명일방주 종료".to_string(), "".to_string(), "shutdown_arknights".to_string()),
        "not_16_9_ratio" => ("화면 비율이 16:9가 아님".to_string(), spec.unwrap_or("").to_string(), "not_16_9_ratio".to_string()),
        "resolution_too_low" => ("화면 해상도가 너무 작음".to_string(), spec.unwrap_or("").to_string(), "resolution_too_low".to_string()),
        "unknown_command" => ("알 수 없는 명령어".to_string(), spec.unwrap_or("").to_string(), "unknown_command".to_string()),
        _ => ("Unknown notification 🤔".to_string(), cause.to_string(), "unknown_notification".to_string()),
    }
}

pub fn show_notification(cause: &str, spec: Option<&str>) {
    let aum_id = "PlayBridge"; // Application User Model ID
    let display_name = "PlayBridge"; // Display name of the application
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();

    let (toast_title, toast_body, toast_tag) = get_notification_details(cause, spec);

    if check_notification_registry(&toast_tag, now, COOLDOWN_SECONDS) {
        let current_dir = env::current_dir().expect("Failed to get current directory");
        let icon_path = current_dir.join(Path::new("resource\\template\\items\\act24side_melding_6.png"));
        let _ = register(aum_id, display_name, Some(&icon_path));

        let manager = ToastManager::new(aum_id);
        let mut toast = Toast::new();
        toast.tag(&toast_tag);
        toast.text1(&toast_title).text2(Text::new(&toast_body)).text3(Text::new(&toast_tag).with_placement(TextPlacement::Attribution));
        toast.scenario(Scenario::Reminder);

        manager.show(&toast).expect("Failed to show toast");

        let hklm = RegKey::predef(HKEY_CURRENT_USER);
        let (key, _) = hklm.create_subkey(REGISTRY_PATH).expect("Failed to create or open registry key");
        key.set_value(&toast_tag, &now).expect("Failed to write to registry");
    }
}
