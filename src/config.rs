use once_cell::sync::Lazy;
use std::sync::{Arc, RwLock};
use winreg::{enums::*, types::FromRegValue, RegKey};

use crate::logging::{debug_log, LogLevel, LogMode};

pub const DISPLAY_WIDTH: u32 = 1280;
pub const DISPLAY_HEIGHT: u32 = 720;

const REG_PATH_CONFIG: &str = r"Software\PlayBridge\config";

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

pub fn set_registry_value(key_name: &str, value: &str) -> std::io::Result<()> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let (key, _) = hkcu.create_subkey(REG_PATH_CONFIG)?;
    key.set_value(key_name, &value)?;
    Ok(())
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

pub struct Config {
    /// Window title pattern used to locate the game window
    pub title: String,
    /// Package name used to launch the game
    pub package: String,
    /// Enable or disable debug capture
    pub debug_capture: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            title: get_reg_value("TITLE", String::new()),
            package: get_reg_value("PACKAGE", String::new()),
            debug_capture: get_reg_value("DEBUG_CAPTURE", 0u32) != 0,
        }
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
