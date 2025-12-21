use once_cell::sync::Lazy;
use std::sync::{Arc, RwLock};
use winreg::{enums::*, types::FromRegValue, RegKey};

use crate::logging::{debug_log, LogLevel, LogMode};

pub const DISPLAY_WIDTH: u32 = 1280;
pub const DISPLAY_HEIGHT: u32 = 720;

const REPOSITORY_URL: &str = "https://api.github.com/repos/HX3N/PlayBridge/releases/latest";

pub const REG_PATH_CONFIG: &str = r"Software\PlayBridge\config";
pub const REG_PATH_STATE: &str = r"Software\PlayBridge\state";
pub const REG_PATH_COOLDOWN: &str = r"Software\PlayBridge\cooldown";
pub const UPDATE_CHECK_COOLDOWN: u64 = 86400;

const DEVELOPMENT_VERSION: &str = "development";

fn get_reg_value<T>(key_name: &str, default_value: T) -> T
where
    T: FromRegValue + 'static,
{
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let (key, _) = hkcu.create_subkey(REG_PATH_CONFIG).unwrap();
    key.get_value(key_name).unwrap_or(default_value)
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

pub fn set_registry_value<T: winreg::types::ToRegValue>(key_name: &str, value: T) -> std::io::Result<()> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let (key, _) = hkcu.create_subkey(REG_PATH_CONFIG)?;
    key.set_value(key_name, &value)?;
    Ok(())
}

#[derive(Clone, Copy, PartialEq, Default)]
pub enum Region {
    #[default]
    Empty,
    EN,
    KR,
    JP,
}

impl Region {
    pub fn from_str(s: &str) -> Self {
        match s {
            "YoStarKR" => Region::KR,
            "YoStarJP" => Region::JP,
            "YoStarEN" => Region::EN,
            _ => Region::Empty,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Region::KR => "YoStarKR",
            Region::JP => "YoStarJP",
            Region::EN => "YoStarEN",
            Region::Empty => "",
        }
    }

    pub fn title(&self) -> &'static str {
        match self {
            Region::KR => "명일방주",
            Region::JP => "アークナイツ",
            Region::EN => "Arknights",
            Region::Empty => "",
        }
    }

    pub fn package(&self) -> &'static str {
        match self {
            Region::KR => "com.YoStarKR.Arknights",
            Region::JP => "com.YoStarJP.Arknights",
            Region::EN => "com.YoStar.Arknights",
            Region::Empty => "",
        }
    }
}

pub fn set_region(region: Region) {
    let is_region_same = {
        let current_config = config();
        current_config.region == region
    };

    if is_region_same {
        return;
    }

    debug_log(
        LogLevel::Info,
        LogMode::Nested,
        &format!("REGION set: {} (title: {}, package: {})", region.as_str(), region.title(), region.package()),
    );
    set_registry_value("REGION", region.as_str()).unwrap();
    Config::reload();
}

pub struct Config {
    pub debug_capture: bool,
    pub region: Region,
}

impl Default for Config {
    fn default() -> Self {
        Self { debug_capture: get_reg_value("DEBUG_CAPTURE", 0u32) != 0, region: Region::from_str(&get_reg_value("REGION", String::new())) }
    }
}

impl Config {
    #[allow(dead_code)]
    pub fn reload() {
        let new_config = Config::default();
        let mut config_guard = CONFIG.write().unwrap();
        *config_guard = new_config;
    }
}

pub static CONFIG: Lazy<Arc<RwLock<Config>>> = Lazy::new(|| Arc::new(RwLock::new(Config::default())));

pub fn config() -> std::sync::RwLockReadGuard<'static, Config> {
    CONFIG.read().unwrap()
}

pub fn version() -> &'static str {
    option_env!("PLAYBRIDGE_VERSION").unwrap_or(DEVELOPMENT_VERSION)
}

pub fn check_version() {
    let current_version = version();
    let stored_version: String = get_reg_value("VERSION", String::new());

    println!("PlayBridge {}", current_version);

    if stored_version == current_version {
        debug_log(LogLevel::Info, LogMode::Nested, &format!("version: {}", current_version));
        return;
    }

    debug_log(LogLevel::Info, LogMode::Nested, &format!("version updated: {} -> {}", stored_version, current_version));

    set_registry_value("VERSION", current_version).unwrap();

    Config::reload();
}

pub fn check_for_update() {
    use crate::notification::{display_notification, Notification};
    use chrono::Utc;

    let current = version();
    let last_check: u64 = get_reg_value("LAST_UPDATE_CHECK", 0);
    let now = Utc::now().timestamp() as u64;

    let is_dev = current == DEVELOPMENT_VERSION;
    let is_cooldown = now > last_check && now - last_check < UPDATE_CHECK_COOLDOWN;

    if is_dev || is_cooldown {
        let reason = if is_dev { "development" } else { "cooldown" };
        debug_log(LogLevel::Info, LogMode::Nested, &format!("Skip update check ({})", reason));
        return;
    }

    let _ = set_registry_value("LAST_UPDATE_CHECK", now);

    let response = match ureq::get(REPOSITORY_URL).set("User-Agent", "PlayBridge").call() {
        Ok(r) => r,
        Err(e) => {
            debug_log(LogLevel::Warn, LogMode::Nested, &format!("Update check failed: {}", e));
            return;
        }
    };

    let json: serde_json::Value = match response.into_json() {
        Ok(j) => j,
        Err(e) => {
            debug_log(LogLevel::Warn, LogMode::Nested, &format!("Update check parse failed: {}", e));
            return;
        }
    };

    let latest = json["tag_name"].as_str().unwrap_or("").trim_start_matches('v');

    if latest == current {
        debug_log(LogLevel::Info, LogMode::Nested, &format!("Up to date: {}", current));
    } else {
        debug_log(LogLevel::Info, LogMode::Nested, &format!("Update available: {} -> {}", current, latest));

        if let Some(url) = json["html_url"].as_str() {
            debug_log(LogLevel::Info, LogMode::Nested, &format!("url open: {}", url));
            let _ = open::that(url);
        }

        display_notification(Notification::UpdateAvailable(latest.to_string()));
    }
}

pub fn toggle_debug() {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let (key, _) = hkcu.create_subkey(REG_PATH_CONFIG).unwrap();
    let current_val: u32 = key.get_value("DEBUG_CAPTURE").unwrap_or(0);
    let new_val: u32 = if current_val == 0 { 1 } else { 0 };
    key.set_value("DEBUG_CAPTURE", &new_val).unwrap();

    let status = if new_val == 1 { "ON" } else { "OFF" };
    let msg = format!("DEBUG_CAPTURE: {}", status);
    println!("{}", msg);

    debug_log(LogLevel::Info, LogMode::Nested, &msg);
}
