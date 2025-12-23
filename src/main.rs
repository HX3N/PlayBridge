mod capture;
mod config;
mod input;
mod logging;
mod notification;
mod window;

use std::{env, time::Instant};

use crate::capture::{screenshot, send_capture, send_capture_nc};
use crate::config::set_benchmark_mode;
use crate::config::{check_for_update, check_version, toggle_debug, toggle_encode, Config, DISPLAY_HEIGHT, DISPLAY_WIDTH};
use crate::logging::{debug_log, panic_hook, LogLevel, LogMode};
use crate::notification::{display_notification, Notification};
use crate::window::{ensure_game_ready, launch_arknights};

#[ctor::ctor]
fn init() {
    panic_hook();
    Config::default();
}

fn main() {
    let start = Instant::now();
    let raw_args: Vec<String> = env::args().collect();
    let full_joined = raw_args.join(" ");
    let args: Vec<String> = full_joined.split_whitespace().map(|s| s.to_string()).collect();

    debug_log(LogLevel::Info, LogMode::Start, &full_joined);

    ensure_game_ready();

    let command = parse_command(&args);
    execute_command(command);

    debug_log(LogLevel::Info, LogMode::End, &format!("{} ms", start.elapsed().as_millis()));
}

enum Command {
    Empty,
    ToggleDebug,
    ToggleEncode,
    Connect,
    GetPropRelease,
    StartActivity { intent: String },
    Devices,
    WindowDisplays,
    GetUuid,
    Tap { x: i32, y: i32 },
    Text { text: String },
    Swipe { x1: i32, y1: i32, x2: i32, y2: i32, duration: i32 },
    KeyEvent { keycode: i32 },
    Screencap,
    ScreencapNc { port: u16 },
    ForceStop,
    Echo { text: String },
    Ignore,
    Unknown(String),
}

fn parse_command(args: &[String]) -> Command {
    if args.len() < 2 {
        return Command::Empty;
    }
    let full_command = args.join(" ");
    match full_command.as_str() {
        c if c.contains("--debug") => Command::ToggleDebug,
        c if c.contains("--encode") => Command::ToggleEncode,
        c if c.contains("disconnect") => Command::Ignore,
        c if c.contains("connect") => Command::Connect,
        c if c.contains("getprop ro.build.version.release") => Command::GetPropRelease,
        c if c.contains("am start -n") => Command::StartActivity { intent: args[7].clone() },
        c if c.contains("devices") => Command::Devices,
        c if c.contains("input tap") => Command::Tap { x: args[6].parse().unwrap_or(0), y: args[7].parse().unwrap_or(0) },
        c if c.contains("input text") => Command::Text { text: args[6..].join(" ") },
        c if c.contains("input swipe") => Command::Swipe {
            x1: args[6].parse().unwrap_or(0),
            y1: args[7].parse().unwrap_or(0),
            x2: args[8].parse().unwrap_or(0),
            y2: args[9].parse().unwrap_or(0),
            duration: args[10].parse().unwrap_or(0),
        },
        c if c.contains("input keyevent 111") => Command::KeyEvent { keycode: 0x01 },
        c if c.contains("dumpsys window displays") || c.contains("wm size") => Command::WindowDisplays,
        c if c.contains("exec-out screencap | nc -w 3 10.0.2.2") => Command::ScreencapNc { port: args[10].parse().unwrap_or(0) },
        c if c.contains("exec-out screencap -p") => Command::Screencap,
        c if c.contains("am force-stop") || c.contains("input keyevent HOME") => Command::ForceStop,
        c if c.contains("settings get secure android_id") => Command::GetUuid,
        c if c.contains("shell echo") => Command::Echo { text: args[5..].join(" ") }, // Connection Preset - Compatible Mode
        c if c.contains("cat /proc/net/arp")
            || c.contains("exec-out screencap | gzip -1")
            || c.contains("start-server")
            || c.contains("kill-server") =>
        {
            Command::Ignore
        }
        _ => Command::Unknown(full_command),
    }
}

fn execute_command(command: Command) {
    match command {
        Command::Empty => {
            screenshot();
            check_for_update();
        }
        Command::ToggleDebug => {
            toggle_debug();
        }
        Command::ToggleEncode => {
            toggle_encode();
        }
        Command::Connect => {
            println!("connected to Google Play Games");
        }
        Command::GetPropRelease => {
            println!("14");
        }
        Command::StartActivity { intent } => {
            launch_arknights(&intent);

            println!("Starting: Intent {{ cmp={} }}", intent);
            println!("Warning: Activity not started, intent has been delivered to currently running top-most instance.");
        }
        Command::Devices => {
            println!("List of devices attached");
            println!("GooglePlayGames\tdevice");

            check_version();
            check_for_update();
        }
        Command::WindowDisplays => {
            println!("{} {}", DISPLAY_WIDTH, DISPLAY_HEIGHT);
            set_benchmark_mode(2);
        }
        Command::GetUuid => {
            println!("0000000000000000");
        }
        Command::Tap { x, y } => {
            input::input_tap(x, y);
        }
        Command::Text { text } => {
            input::input_text(&text);
        }
        Command::Swipe { x1, y1, x2, y2, duration } => {
            input::input_swipe(x1, y1, x2, y2, duration);
        }
        Command::KeyEvent { keycode } => {
            input::input_keyevent(keycode);
        }
        Command::Screencap => {
            send_capture();
        }
        Command::ScreencapNc { port } => {
            send_capture_nc(port);
        }
        Command::ForceStop => {
            input::terminate();
            display_notification(Notification::GpgShutdown);
        }
        Command::Echo { text } => {
            println!("{}", text);
        }
        Command::Ignore => {}
        Command::Unknown(cmd) => {
            display_notification(Notification::UnknownCommand(cmd));
        }
    }
}
