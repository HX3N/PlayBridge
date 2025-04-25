use serde::Deserialize;
use serde_json::Value;
use std::fs;

#[derive(Deserialize, Debug)]
#[serde(default)]
pub struct Config {
    /// Window title pattern used to locate the game window
    pub title: String,
    /// Arknights package name used to launch the game
    pub package: String,
    /// Polling rate during swipe
    pub polling_rate: u32,
    /// Multiplier for swipe speed
    pub swipe_speed: u32,
    /// Resolution used when resizing images for MAA or for screenshot
    pub width: u32,
    /// Resolution used when resizing images for MAA or for screenshot
    pub height: u32,
    /// Enable or disable debug logging
    pub debug: bool,
    /// Enable or disable notifications
    pub notification: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            title: "명일방주".into(), package: "com.YoStarKR.Arknights".into(), polling_rate: 1000, swipe_speed: 10, width: 1280, height: 720, debug: false, notification: true
        }
    }
}

fn merge_str<F>(v: &Value, key: &str, default: F) -> String
where
    F: FnOnce() -> String,
{
    v.get(key).and_then(Value::as_str).map(ToString::to_string).unwrap_or_else(default)
}

fn merge_u32<F>(v: &Value, key: &str, default: F) -> u32
where
    F: FnOnce() -> u32,
{
    v.get(key).and_then(|val| val.as_u64().map(|i| i as u32)).unwrap_or_else(default)
}

fn merge_bool<F>(v: &Value, key: &str, default: F) -> bool
where
    F: FnOnce() -> bool,
{
    v.get(key).and_then(Value::as_bool).unwrap_or_else(default)
}

impl Config {
    pub fn load_from_file(path: &str) -> Self {
        let raw = match fs::read_to_string(path) {
            Ok(s) => s,
            Err(_) => return Self::default(),
        };

        let v: Value = serde_json::from_str(&raw).unwrap_or_default();
        let mut cfg = Config::default();

        cfg.title = merge_str(&v, "title", || Config::default().title.clone());
        cfg.package = merge_str(&v, "package", || Config::default().package.clone());
        cfg.polling_rate = merge_u32(&v, "polling_rate", || Config::default().polling_rate);
        cfg.swipe_speed = merge_u32(&v, "swipe_speed", || Config::default().swipe_speed);
        cfg.width = merge_u32(&v, "width", || Config::default().width);
        cfg.height = merge_u32(&v, "height", || Config::default().height);
        cfg.debug = merge_bool(&v, "debug", || Config::default().debug);
        cfg.notification = merge_bool(&v, "notification", || Config::default().notification);

        cfg
    }
}

use once_cell::sync::Lazy;
pub static CONFIG: Lazy<Config> = Lazy::new(|| Config::load_from_file("PlayBridgeADB.json"));
