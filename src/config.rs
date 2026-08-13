use chrono::Utc;
use std::sync::{LazyLock, RwLock};
use winreg::{
    enums::*,
    types::{FromRegValue, ToRegValue},
    RegKey,
};

use crate::logging::{debug_log, LogLevel, LogMode};
use crate::notification::{display_notification, Notification};

pub use crate::shared::{DISPLAY_HEIGHT, DISPLAY_WIDTH, REG_PATH_STATE};

const REPOSITORY_URL: &str = "https://api.github.com/repos/HX3N/PlayBridge/releases/latest";

pub const REG_PATH_CONFIG: &str = r"Software\PlayBridge\config";
pub const REG_PATH_COOLDOWN: &str = r"Software\PlayBridge\cooldown";
pub const UPDATE_CHECK_COOLDOWN: u64 = 60 * 60 * 24;

pub const MAX_TOUCH_OVERLAY_FILES: usize = 100;

const DEVELOPMENT_VERSION: &str = "development";

#[derive(Clone, Copy, PartialEq, Default)]
pub enum Client {
    #[default]
    Empty,
    EN,
    KR,
    JP,
}

impl Client {
    pub fn from_str(s: &str) -> Self {
        match s {
            "YoStarKR" => Client::KR,
            "YoStarJP" => Client::JP,
            "YoStarEN" => Client::EN,
            _ => Client::Empty,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Client::KR => "YoStarKR",
            Client::JP => "YoStarJP",
            Client::EN => "YoStarEN",
            Client::Empty => "",
        }
    }

    pub fn title(&self) -> &'static str {
        match self {
            Client::KR => "명일방주",
            Client::JP => "アークナイツ",
            Client::EN => "Arknights",
            Client::Empty => "",
        }
    }

    pub fn package(&self) -> &'static str {
        match self {
            Client::KR => "com.YoStarKR.Arknights",
            Client::JP => "com.YoStarJP.Arknights",
            Client::EN => "com.YoStarEN.Arknights",
            Client::Empty => "",
        }
    }
}

pub struct Config {
    pub touch_overlay: bool,
    pub client: Client,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            touch_overlay: get_registry("TOUCH_OVERLAY", 0u32, REG_PATH_CONFIG) != 0,
            client: Client::from_str(&get_registry("CLIENT", String::new(), REG_PATH_CONFIG)),
        }
    }
}

impl Config {
    pub fn reload() {
        let new_config = Config::default();
        let mut config_guard = CONFIG.write().unwrap();
        *config_guard = new_config;
    }
}

static CONFIG: LazyLock<RwLock<Config>> = LazyLock::new(|| RwLock::new(Config::default()));

pub fn config() -> std::sync::RwLockReadGuard<'static, Config> {
    CONFIG.read().unwrap()
}

pub fn set_client(client: Client) {
    let is_client_same = {
        let current_config = config();
        current_config.client == client
    };

    if is_client_same {
        return;
    }

    debug_log(
        LogLevel::Info,
        LogMode::Nested,
        &format!("Client: set {} (title: {}, package: {})", client.as_str(), client.title(), client.package()),
    );
    set_registry("CLIENT", client.as_str(), REG_PATH_CONFIG).unwrap();
    Config::reload();
}

pub fn set_benchmark_mode() {
    let _ = set_registry("BENCHMARK", 1u32, REG_PATH_STATE);
    debug_log(LogLevel::Info, LogMode::Nested, "Benchmark: armed");
}

// One-shot flag: MAA arms benchmark with `wm size`, then fires a single screencap to measure throughput.
// Consume it so only that one capture is silenced and normal capture resumes on the next request.
pub fn check_benchmark_mode() -> bool {
    if get_registry("BENCHMARK", 0u32, REG_PATH_STATE) != 0 {
        let _ = set_registry("BENCHMARK", 0u32, REG_PATH_STATE);
        debug_log(LogLevel::Info, LogMode::Nested, "Benchmark: consumed");
        return true;
    }
    false
}

pub fn peek_benchmark_mode() -> bool {
    get_registry("BENCHMARK", 0u32, REG_PATH_STATE) != 0
}

pub fn version() -> &'static str {
    option_env!("PLAYBRIDGE_VERSION").unwrap_or(DEVELOPMENT_VERSION)
}

pub fn check_version() {
    let current_version = version();
    let stored_version: String = get_registry("VERSION", String::new(), REG_PATH_CONFIG);

    println!("PlayBridge {}", current_version);

    if stored_version == current_version {
        debug_log(LogLevel::Info, LogMode::Nested, &format!("Version: {}", current_version));
        return;
    }

    debug_log(LogLevel::Info, LogMode::Nested, &format!("Version: updated {} -> {}", stored_version, current_version));
    set_registry("VERSION", current_version, REG_PATH_CONFIG).unwrap();
    Config::reload();
}

pub fn check_for_update() {
    let current = version();
    let last_check: u64 = get_registry("LAST_UPDATE_CHECK", 0, REG_PATH_CONFIG);
    let now = Utc::now().timestamp() as u64;

    let is_dev = current == DEVELOPMENT_VERSION;
    let is_cooldown = now > last_check && now - last_check < UPDATE_CHECK_COOLDOWN;

    if is_dev || is_cooldown {
        let reason = if is_dev { "development" } else { "cooldown" };
        debug_log(LogLevel::Info, LogMode::Nested, &format!("Update: skip check ({})", reason));
        return;
    }

    let _ = set_registry("LAST_UPDATE_CHECK", now, REG_PATH_CONFIG);

    let json: serde_json::Value = match ureq::get(REPOSITORY_URL)
        .header("User-Agent", "PlayBridge")
        .call()
        .and_then(|mut r| r.body_mut().read_json())
    {
        Ok(j) => j,
        Err(e) => {
            debug_log(LogLevel::Warn, LogMode::Nested, &format!("Update: check failed: {}", e));
            return;
        }
    };

    let latest = json["tag_name"].as_str().unwrap_or("");
    if latest == current {
        debug_log(LogLevel::Info, LogMode::Nested, &format!("Update: up to date ({})", current));
    } else {
        debug_log(LogLevel::Info, LogMode::Nested, &format!("Update: available {} -> {}", current, latest));
        display_notification(Notification::UpdateAvailable(latest.to_string()));
    }
}

pub fn toggle_touch_overlay() {
    toggle_config("TOUCH_OVERLAY");
}

pub fn get_registry<T: FromRegValue>(key_name: &str, default: T, path: &str) -> T {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    hkcu.open_subkey(path).and_then(|key| key.get_value(key_name)).unwrap_or(default)
}

/// Write a value to an HKCU subkey, creating it if needed.
pub fn set_registry<T: ToRegValue>(key_name: &str, value: T, path: &str) -> std::io::Result<()> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let (key, _) = hkcu.create_subkey(path)?;
    key.set_value(key_name, &value)
}

fn toggle_config(key_name: &str) {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let (key, _) = hkcu.create_subkey(REG_PATH_CONFIG).unwrap();
    let current_val: u32 = key.get_value(key_name).unwrap_or(0);
    let new_val: u32 = if current_val == 0 { 1 } else { 0 };
    key.set_value(key_name, &new_val).unwrap();

    let status = if new_val == 1 { "ON" } else { "OFF" };
    let msg = format!("{}: {}", key_name, status);
    println!("{}", msg);
    debug_log(LogLevel::Info, LogMode::Nested, &msg);
}
