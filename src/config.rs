use once_cell::sync::Lazy;
use std::sync::{Arc, RwLock};
use winreg::{enums::*, types::FromRegValue, RegKey};

pub const DISPLAY_WIDTH: u32 = 1280;
pub const DISPLAY_HEIGHT: u32 = 720;

pub const EXTRAS_PORT: u16 = 50505;

const REG_PATH_CONFIG: &str = r"Software\PlayBridge\config";
const REG_PATH_NOTIFICATION: &str = r"Software\PlayBridge\notification";

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

pub struct Config {
    /// Window title pattern used to locate the game window
    pub title: String,
    /// Arknights package name used to launch the game
    pub package: String,
    /// Multiplier for swipe speed
    pub swipe_speed: u32,
    /// Maximum FPS for Extras capture
    #[allow(dead_code)]
    pub max_fps: u32,
    /// Enable or disable debug logging
    pub debug: bool,
    /// Enable or disable debug capture
    pub debug_capture: bool,

    pub notification_path: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            title: get_reg_value("TITLE", "명일방주".to_string()),
            package: get_reg_value("PACKAGE", "com.YoStarKR.Arknights".to_string()),
            swipe_speed: get_reg_value("SWIPE_SPEED", 10u32),
            max_fps: get_reg_value("MAX_FPS", 10u32),
            debug: get_reg_value("DEBUG", 0u32) != 0,
            debug_capture: get_reg_value("DEBUG_CAPTURE", 0u32) != 0,

            notification_path: REG_PATH_NOTIFICATION.to_string(),
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
