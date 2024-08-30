use std::{
    env,
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};
use winreg::{enums::*, RegKey};
use winrt_toast::{content::text::TextPlacement, register, Scenario, Text, Toast, ToastManager};

pub const REGISTRY_PATH: &str = r"Software\PlayBridge";
const COOLDOWN_SECONDS: u64 = 20; // Minimum time between notifications of the same type
const AUM_ID: &str = "PlayBridge"; // Application User Model ID for Windows notifications
const DISPLAY_NAME: &str = "PlayBridge";

const INFO: &str = "ℹ️ 정보";
const WARNING: &str = "⚠️ 경고";
const ERROR: &str = "⛔ 오류";

pub struct NotificationDetails {
    title: String,
    body: String,
    tag: String,
}

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

fn get_notification_details(cause: &str, spec: Option<&str>) -> NotificationDetails {
    let (title, body) = match cause {
        "start_arknights" => (INFO, "명일방주 실행".to_string()),
        "shutdown_arknights" => (INFO, "명일방주 종료".to_string()),
        "resolution_change" => (INFO, format!("해상도가 변경되었습니다 ({})", spec.expect(""))),
        "not_16_9_ratio" | "resolution_too_low" => {
            let prefix = if cause == "not_16_9_ratio" { "화면 비율이 16:9가 아닙니다" } else { "해상도가 너무 작습니다" };
            (WARNING, format!("{} ({})\nMAA의 인식에 문제가 생길 수 있습니다!", prefix, spec.expect("")))
        }
        "unknown_command" => (ERROR, format!("알 수 없는 명령어입니다!\n{}", spec.expect(""))),
        _ => ("Unknown notification", cause.to_string()),
    };

    NotificationDetails { title: title.to_string(), body, tag: cause.to_string() }
}

pub fn show_notification(cause: &str, spec: Option<&str>) {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();

    let details = get_notification_details(cause, spec);

    if details.tag == "resolution_change" || check_notification_registry(&details.tag, now, COOLDOWN_SECONDS) {
        let current_dir = env::current_dir().expect("Failed to get current directory");
        let icon_path = current_dir.join(Path::new("resource\\template\\items\\act24side_melding_6.png"));
        let _ = register(AUM_ID, DISPLAY_NAME, Some(&icon_path));

        let manager = ToastManager::new(AUM_ID);
        let mut toast = Toast::new();
        toast.tag(&details.tag);
        toast
            .text1(&details.title)
            .text2(Text::new(&details.body))
            .text3(Text::new(format!("tag: {}", &details.tag)).with_placement(TextPlacement::Attribution));
        toast.scenario(Scenario::Reminder);

        manager.show(&toast).expect("Failed to show toast");

        let hklm = RegKey::predef(HKEY_CURRENT_USER);
        let (key, _) = hklm.create_subkey(REGISTRY_PATH).expect("Failed to create or open registry key");
        key.set_value(&details.tag, &now).expect("Failed to write to registry");
    }
}

pub fn check_and_update_resolution(width: u32, height: u32) {
    let hklm = RegKey::predef(HKEY_CURRENT_USER);
    let (key, _) = hklm.create_subkey(REGISTRY_PATH).expect("Failed to create or open registry key");

    let stored_width: u32 = key.get_value("resolution_width").unwrap_or(0);
    let stored_height: u32 = key.get_value("resolution_height").unwrap_or(0);

    let resolution_changed = stored_width != width || stored_height != height;
    let first_run = stored_width * stored_height == 0;

    if first_run || resolution_changed {
        key.set_value("resolution_width", &width).expect("Failed to write width to registry");
        key.set_value("resolution_height", &height).expect("Failed to write height to registry");

        if resolution_changed && !first_run {
            show_notification("resolution_change", Some(&format!("{}x{}", width, height)));
        }
    }
}
