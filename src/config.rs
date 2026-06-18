use chrono::Utc;
use std::sync::{Arc, LazyLock, RwLock};
use winreg::{enums::*, types::FromRegValue, RegKey};

use crate::logging::{debug_log, LogLevel, LogMode};
use crate::notification::{display_notification, Notification};

pub const DISPLAY_WIDTH: u32 = 1280;
pub const DISPLAY_HEIGHT: u32 = 720;

const REPOSITORY_URL: &str = "https://api.github.com/repos/HX3N/PlayBridge/releases/latest";

pub const REG_PATH_CONFIG: &str = r"Software\PlayBridge\config";
pub const REG_PATH_STATE: &str = r"Software\PlayBridge\state";
pub const REG_PATH_COOLDOWN: &str = r"Software\PlayBridge\cooldown";
pub const UPDATE_CHECK_COOLDOWN: u64 = 60 * 60 * 24; // 24 hours

pub const MAX_DEBUG_CAPTURE_FILES: usize = 100;

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
    pub debug_capture: bool,
    pub client: Client,
}

impl Default for Config {
    fn default() -> Self {
        Self { debug_capture: get_reg_value("DEBUG_CAPTURE", 0u32) != 0, client: Client::from_str(&get_reg_value("CLIENT", String::new())) }
    }
}

impl Config {
    pub fn reload() {
        let new_config = Config::default();
        let mut config_guard = CONFIG.write().unwrap();
        *config_guard = new_config;
    }
}

pub static CONFIG: LazyLock<Arc<RwLock<Config>>> = LazyLock::new(|| Arc::new(RwLock::new(Config::default())));

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
    set_registry_value("CLIENT", client.as_str()).unwrap();
    Config::reload();
}

pub fn set_benchmark_mode(count: u32) {
    let _ = set_registry_dword("BENCHMARK_COUNT", count, REG_PATH_STATE);
    debug_log(LogLevel::Info, LogMode::Nested, &format!("Benchmark: mode set to {}", count));
}

pub fn check_benchmark_mode() -> bool {
    let count = get_registry_dword("BENCHMARK_COUNT", REG_PATH_STATE).unwrap_or(0);
    if count > 0 {
        let _ = set_registry_dword("BENCHMARK_COUNT", count - 1, REG_PATH_STATE);
        debug_log(LogLevel::Info, LogMode::Nested, &format!("Benchmark: active (remaining {} -> {})", count, count - 1));
        return true;
    }
    false
}

pub fn peek_benchmark_mode() -> bool {
    get_registry_dword("BENCHMARK_COUNT", REG_PATH_STATE).unwrap_or(0) > 0
}

pub fn version() -> &'static str {
    option_env!("PLAYBRIDGE_VERSION").unwrap_or(DEVELOPMENT_VERSION)
}

pub fn check_version() {
    let current_version = version();
    let stored_version: String = get_reg_value("VERSION", String::new());

    println!("PlayBridge {}", current_version);

    if stored_version == current_version {
        debug_log(LogLevel::Info, LogMode::Nested, &format!("Version: {}", current_version));
        return;
    }

    debug_log(LogLevel::Info, LogMode::Nested, &format!("Version: updated {} -> {}", stored_version, current_version));
    set_registry_value("VERSION", current_version).unwrap();
    Config::reload();
}

pub fn check_for_update() {
    let current = version();
    let last_check: u64 = get_reg_value("LAST_UPDATE_CHECK", 0);
    let now = Utc::now().timestamp() as u64;

    let is_dev = current == DEVELOPMENT_VERSION;
    let is_cooldown = now > last_check && now - last_check < UPDATE_CHECK_COOLDOWN;

    if is_dev || is_cooldown {
        let reason = if is_dev { "development" } else { "cooldown" };
        debug_log(LogLevel::Info, LogMode::Nested, &format!("Update: skip check ({})", reason));
        return;
    }

    let _ = set_registry_value("LAST_UPDATE_CHECK", now);

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
        if let Some(url) = json["html_url"].as_str() {
            debug_log(LogLevel::Info, LogMode::Nested, &format!("URL: opening {}", url));
            let _ = open::that(url);
        }
        display_notification(Notification::UpdateAvailable(latest.to_string()));
    }
}

pub fn toggle_debug() {
    toggle_config("DEBUG_CAPTURE");
}

pub fn get_registry_dword(key_name: &str, path: &str) -> std::io::Result<u32> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let key = hkcu.open_subkey(path)?;
    key.get_value(key_name)
}

pub fn set_registry_dword(key_name: &str, value: u32, path: &str) -> std::io::Result<()> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let (key, _) = hkcu.create_subkey(path)?;
    key.set_value(key_name, &value)?;
    Ok(())
}

/// Write a string to an HKCU subkey. Used to publish EXE_PATH so the fake
/// nemu DLL can locate and spawn the WGC daemon.
pub fn set_registry_string(key_name: &str, value: &str, path: &str) -> std::io::Result<()> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let (key, _) = hkcu.create_subkey(path)?;
    key.set_value(key_name, &value)?;
    Ok(())
}

pub fn set_registry_value<T: winreg::types::ToRegValue>(key_name: &str, value: T) -> std::io::Result<()> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let (key, _) = hkcu.create_subkey(REG_PATH_CONFIG)?;
    key.set_value(key_name, &value)?;
    Ok(())
}

fn get_reg_value<T>(key_name: &str, default_value: T) -> T
where
    T: FromRegValue + 'static,
{
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let (key, _) = hkcu.create_subkey(REG_PATH_CONFIG).unwrap();
    key.get_value(key_name).unwrap_or(default_value)
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
