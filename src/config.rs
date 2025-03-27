use lazy_static::lazy_static;
use std::{
    env,
    io::{stdin, stdout, Write},
    thread,
    time::Duration,
};
use winreg::{enums::*, RegKey};

pub const CONFIG_REGISTRY_PATH: &str = r"Software\PlayBridge\Config";
pub const REGION: &str = "KR";
pub const DISPLAY_WIDTH: f32 = 1280.0;
pub const DISPLAY_HEIGHT: f32 = 720.0;
pub const SWIPE_SPEED: f32 = 10.0;

#[derive(Debug)]
pub struct Config {
    pub region: String,
    pub package: String,
    pub title: String,
    pub display_width: f32,
    pub display_height: f32,
    pub swipe_speed: f32,
}

pub fn load_config() -> Config {
    let hklm = RegKey::predef(HKEY_CURRENT_USER);
    let key = hklm.open_subkey(CONFIG_REGISTRY_PATH).ok();
    let region: String = key.as_ref().and_then(|k| k.get_value("REGION").ok()).unwrap_or_else(|| REGION.to_string());
    let swipe_speed: f32 = key.as_ref().and_then(|k| k.get_value::<u32, _>("SWIPE_SPEED").ok()).map(|v| v as f32).unwrap_or(SWIPE_SPEED);
    let display_width: f32 = key.as_ref().and_then(|k| k.get_value::<u32, _>("DISPLAY_WIDTH").ok()).map(|v| v as f32).unwrap_or(DISPLAY_WIDTH);
    let display_height: f32 = key.as_ref().and_then(|k| k.get_value::<u32, _>("DISPLAY_HEIGHT").ok()).map(|v| v as f32).unwrap_or(DISPLAY_HEIGHT);

    let (package, title) = match region.as_str() {
        "JP" => ("com.YoStarJP.Arknights".to_string(), "アークナイツ".to_string()),
        "EN" => ("com.YoStarEN.Arknights".to_string(), "Arknights".to_string()),
        _ => ("com.YoStarKR.Arknights".to_string(), "명일방주".to_string()),
    };

    Config { region, package, title, display_width, display_height, swipe_speed }
}

pub fn save_config(config: &Config) {
    let hklm = RegKey::predef(HKEY_CURRENT_USER);
    let (key, _) = hklm.create_subkey(CONFIG_REGISTRY_PATH).expect("Failed to open config registry key");
    key.set_value("REGION", &config.region).unwrap();
    key.set_value("DISPLAY_WIDTH", &(config.display_width as u32)).unwrap();
    key.set_value("DISPLAY_HEIGHT", &(config.display_height as u32)).unwrap();
    key.set_value("SWIPE_SPEED", &(config.swipe_speed as u32)).unwrap();
}

pub fn prompt(prompt_text: &str, current: &str) -> String {
    print!("{} [{}]: ", prompt_text, current);
    stdout().flush().unwrap();
    let mut input = String::new();
    stdin().read_line(&mut input).unwrap();
    let input = input.trim();
    if input.is_empty() {
        current.to_string()
    } else {
        input.to_string()
    }
}

pub fn prompt_parse<T: std::str::FromStr + std::fmt::Display>(prompt_text: &str, current: T) -> T {
    loop {
        print!("{} [{}]: ", prompt_text, current);
        stdout().flush().unwrap();
        let mut input = String::new();
        stdin().read_line(&mut input).unwrap();
        let input = input.trim();
        if input.is_empty() {
            return current;
        } else if let Ok(val) = input.parse::<T>() {
            return val;
        } else {
            println!("Invalid input. Please try again.");
        }
    }
}

pub fn remove_config_from_exe_name() {
    let current_exe = env::current_exe().expect("Failed to get current exe path");
    let file_name_os = current_exe.file_name().expect("Failed to get exe file name");
    let file_name = file_name_os.to_string_lossy();
    let lower_file_name = file_name.to_lowercase();

    if let Some(idx) = lower_file_name.find("-config") {
        let new_file_name = format!("{}{}", &file_name[..idx], &file_name[idx + "-config".len()..]).trim().to_string();

        if new_file_name.is_empty() {
            return;
        }

        let new_path = current_exe.with_file_name(new_file_name);
        let _ = std::fs::rename(&current_exe, &new_path);
    }
}

pub fn clear_screen() {
    print!("\x1B[2J\x1B[H");
    stdout().flush().unwrap();
}

pub fn config_mode() {
    let mut config = load_config();
    loop {
        clear_screen();
        println!("================================ Config Mode ================================\n");
        println!("{:<4} {:<20} {:<15} {:<15}", "No.", "Setting", "Current", "Default");
        println!("{:<4} {:<20} {:<15} {:<15}", "1.", "REGION", config.region, format!("{} [명일방주]", REGION));
        println!("{:<4} {:<20} {:<15} {:<15}", "4.", "SWIPE_SPEED", config.swipe_speed, format!("{} [Recommended 4~16]", SWIPE_SPEED));
        println!("{:<4} {:<20} {:<15} {:<15}", "2.", "DISPLAY_WIDTH", config.display_width, DISPLAY_WIDTH);
        println!("{:<4} {:<20} {:<15} {:<15}", "3.", "DISPLAY_HEIGHT", config.display_height, DISPLAY_HEIGHT);
        println!("{:<4} {:<20}", "5.", "Reset configs");
        println!("{:<4} {:<20}", "6.", "Save and exit");
        println!("\n=============================================================================\n");

        print!("Enter your selection (1-6): ");
        stdout().flush().unwrap();

        let mut selection = String::new();
        stdin().read_line(&mut selection).unwrap();
        match selection.trim() {
            "1" => {
                let new_region = prompt("Enter REGION (KR/EN/JP)", &config.region);
                if new_region != "KR" && new_region != "JP" && new_region != "EN" {
                    println!("Invalid REGION");
                    thread::sleep(Duration::from_secs(1));
                } else {
                    config.region = new_region;
                    let (package, title) = match config.region.as_str() {
                        "JP" => ("com.YoStarJP.Arknights".to_string(), "アークナイツ".to_string()),
                        "EN" => ("com.YoStarEN.Arknights".to_string(), "Arknights".to_string()),
                        _ => ("com.YoStarKR.Arknights".to_string(), "명일방주".to_string()),
                    };
                    config.package = package;
                    config.title = title;
                }
            }
            "2" => {
                config.display_width = prompt_parse("Enter DISPLAY_WIDTH", config.display_width);
            }
            "3" => {
                config.display_height = prompt_parse("Enter DISPLAY_HEIGHT", config.display_height);
            }
            "4" => {
                config.swipe_speed = prompt_parse("Enter SWIPE_SPEED", config.swipe_speed);
            }
            "5" => {
                config.region = REGION.to_string();
                config.display_width = DISPLAY_WIDTH;
                config.display_height = DISPLAY_HEIGHT;
                config.swipe_speed = SWIPE_SPEED;
                config.package = "com.YoStarKR.Arknights".to_string();
                config.title = "명일방주".to_string();

                println!("Reset Configs");
                thread::sleep(Duration::from_secs(1));
            }
            "6" => {
                save_config(&config);
                crate::notification::show_notification(crate::notification::INFO, "Settings have been saved!", "config_saved");

                remove_config_from_exe_name();

                break;
            }
            _ => {
                println!("Invalid selection");
                thread::sleep(Duration::from_secs(1));
            }
        }
    }
}

lazy_static! {
    pub static ref CONFIG: Config = load_config();
}
