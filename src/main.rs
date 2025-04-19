mod config;
mod input;
mod notification;
mod utils;

use std::env;

use crate::config::CONFIG;
use notification::show_notification;
use utils::*;

#[ctor::ctor]
fn init() {
    debug_panic();
}

fn main() {
    let start = get_now();
    run_arknights();

    let args: Vec<String> = env::args().collect();
    let command = parse_command(&args);
    execute_command(command);

    debug_log(LogLevel::INFO, &args.join(" "), Some(start.elapsed().as_millis()));
}

enum Command {
    Empty,
    Connect,
    GetPropVersionRelease,
    StartActivity { intent: String },
    DumpsysWindowDisplays,
    GetUUID,
    InputTap { x: i32, y: i32 },
    InputText { text: String },
    InputSwipe { x1: i32, y1: i32, x2: i32, y2: i32, duration: i32 },
    InputKeyEvent { keycode: i32 },
    ExecOutScreencap,
    ForceStop,
    IgnoreCommand,
    Unknown(String),
}

fn parse_command(args: &[String]) -> Command {
    if args.len() <= 1 {
        return Command::Empty;
    }
    let full_command = args.join(" ");
    match full_command.as_str() {
        c if c.contains("connect") => Command::Connect,
        c if c.contains("getprop ro.build.version.release") => Command::GetPropVersionRelease,
        c if c.contains("am start -n") => Command::StartActivity { intent: args[7].clone() },
        c if c.contains("input tap") => Command::InputTap { x: args[6].parse().unwrap(), y: args[7].parse().unwrap() },
        c if c.contains("input text") => Command::InputText { text: args[6..].join(" ") },
        c if c.contains("input swipe") => Command::InputSwipe {
            x1: args[6].parse().unwrap(),
            y1: args[7].parse().unwrap(),
            x2: args[8].parse().unwrap(),
            y2: args[9].parse().unwrap(),
            duration: args[10].parse().unwrap(),
        },
        c if c.contains("input keyevent 111") => Command::InputKeyEvent { keycode: 0x01 },
        c if c.contains("dumpsys window displays") || c.contains("wm size") => Command::DumpsysWindowDisplays,
        c if c.contains("exec-out screencap -p") => Command::ExecOutScreencap,
        c if c.contains("am force-stop") || c.contains("input keyevent HOME") => Command::ForceStop,
        c if c.contains("settings get secure android_id") => Command::GetUUID,
        c if c.contains("cat /proc/net/arp")
            || c.contains("exec-out screencap | nc -w 3")
            || c.contains("exec-out screencap | gzip -1")
            || c.contains("start-server")
            || c.contains("kill-server")
            || c.contains("devices") =>
        {
            Command::IgnoreCommand
        }
        _ => Command::Unknown(full_command),
    }
}

fn execute_command(command: Command) {
    match command {
        Command::Empty => {
            capture_screenshot();
            show_notification(LogLevel::INFO, "Screenshot saved to desktop!", "screenshot_saved");
        }
        // ========= MAA needs this output =========
        Command::Connect => {
            println!("connected to Google Play Games");
        }
        Command::GetPropVersionRelease => {
            println!("14");
        }
        Command::StartActivity { intent } => {
            println!("Starting: Intent {{ cmp={} }}", intent);
            println!("Warning: Activity not started, intent has been delivered to currently running top-most instance.");
        }
        Command::DumpsysWindowDisplays => {
            println!("{} {}", CONFIG.width, CONFIG.height);
        }
        Command::GetUUID => {
            println!("googleplaygames");
        }
        // =========================================
        Command::InputTap { x, y } => {
            input::input_tap(x, y);
        }
        Command::InputText { text } => {
            input::input_text(&text);
        }
        Command::InputSwipe { x1, y1, x2, y2, duration } => {
            input::input_swipe(x1, y1, x2, y2, duration);
        }
        Command::InputKeyEvent { keycode } => {
            input::input_keyevent(keycode);
        }
        Command::ExecOutScreencap => {
            capture_maa();
        }
        Command::ForceStop => {
            input::terminate();
            show_notification(LogLevel::INFO, "Arknights shutdown", "shutdown_arknights");
        }
        Command::IgnoreCommand => {}
        Command::Unknown(cmd) => {
            debug_log(LogLevel::ERROR, &format!("Unknown command: {}", cmd), None);
            show_notification(LogLevel::ERROR, &format!("Unknown command!\n{}", cmd), "unknown_command");
        }
    }
}
