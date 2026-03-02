use std::{env, thread, time::Duration, time::Instant};

mod capture;
mod config;
mod input;
mod input_adb;
mod logging;
mod notification;
mod window;

use crate::config::{check_benchmark_mode, check_for_update, check_version, toggle_debug, DISPLAY_HEIGHT, DISPLAY_WIDTH};
use crate::config::{peek_benchmark_mode, set_benchmark_mode};
use crate::logging::{debug_log, LogLevel, LogMode};
use crate::notification::{display_notification, Notification};
use crate::window::{apply_intent_package, ensure_game_ready, print_window_list, start_game_if_needed, GameWindow};

const BENCHMARK_DELAY_MS: u64 = 50;

fn main() {
    let start = Instant::now();

    logging::register_panic_hook();
    logging::rotate_log();

    let raw_args: Vec<String> = env::args().collect();
    let full_joined = raw_args.join(" ");
    let args: Vec<String> = raw_args[1..].iter().flat_map(|s| s.split_whitespace()).map(String::from).collect();

    debug_log(LogLevel::Info, LogMode::Start, &full_joined);

    if !peek_benchmark_mode() {
        ensure_game_ready();
    }

    let command = parse_command(&args);
    execute_command(command);

    debug_log(LogLevel::Info, LogMode::End, &format!("{} ms", start.elapsed().as_millis()));
}

enum Command {
    // [Lifecycle & System]
    Empty,
    Connect,
    StartActivity { intent: String },
    Echo { text: String },
    ForceStop,
    ToggleDebug,

    // [Info & State]
    WindowList,
    WindowDisplays,
    Devices,
    GetPropRelease,
    GetUuid,

    // [Screen Capture]
    Screencap,
    ScreencapNc { port: u16 },

    // [Minitouch]
    GetPropAbilist,
    DumpsysInputOrientation,
    MinitouchDaemon,

    // [ADB Input]
    Tap { x: i32, y: i32 },
    Swipe { x1: i32, y1: i32, x2: i32, y2: i32, duration: i32 },
    KeyEvent { keycode: i32 },
    Text { text: String },

    // [Fallback]
    Ignore,
    Unknown(String),
}

fn parse_command(args: &[String]) -> Command {
    if args.is_empty() {
        return Command::Empty;
    }
    let full_command = args.join(" ");
    match full_command.as_str() {
        c if c.contains("connect") => Command::Connect,
        c if c.contains("am start -n") => Command::StartActivity { intent: args[6].clone() },
        c if c.contains("shell echo") => Command::Echo { text: args.get(4..).map_or(String::new(), |s| s.join(" ")) }, // Connection Preset - Compatible Mode
        c if c.contains("am force-stop") || c.contains("input keyevent HOME") => Command::ForceStop,
        c if c.contains("--debug") => Command::ToggleDebug,

        c if c.contains("--list") => Command::WindowList,
        c if c.contains("dumpsys window displays") || c.contains("wm size") => Command::WindowDisplays,
        c if c.contains("devices") => Command::Devices,
        c if c.contains("getprop ro.build.version.release") => Command::GetPropRelease,
        c if c.contains("settings get secure android_id") => Command::GetUuid,

        c if c.contains("exec-out screencap -p") => Command::Screencap,
        c if c.contains("exec-out screencap | nc -w 3 10.0.2.2") => Command::ScreencapNc { port: args[9].parse().unwrap_or(0) },

        c if c.contains("ro.product.cpu.abilist") => Command::GetPropAbilist,
        c if c.contains("dumpsys input") && c.contains("SurfaceOrientation") => Command::DumpsysInputOrientation,
        c if c.contains("/data/local/tmp/") && c.contains("-i") => Command::MinitouchDaemon,

        c if c.contains("input tap") => Command::Tap { x: args[5].parse().unwrap_or(0), y: args[6].parse().unwrap_or(0) },
        c if c.contains("input swipe") => Command::Swipe {
            x1: args[5].parse().unwrap_or(0),
            y1: args[6].parse().unwrap_or(0),
            x2: args[7].parse().unwrap_or(0),
            y2: args[8].parse().unwrap_or(0),
            duration: args[9].parse().unwrap_or(0),
        },
        c if c.contains("input keyevent 111") => Command::KeyEvent { keycode: 0x01 },
        c if c.contains("input text") => Command::Text { text: args[5..].join(" ") },

        c if c.contains("cat /proc/net/arp")
            || c.contains("disconnect")
            || c.contains("exec-out screencap | gzip -1")
            || c.contains("start-server")
            || c.contains("kill-server")
            || c.contains("push") && c.contains("/data/local/tmp/")
            || c.contains("chmod") && c.contains("/data/local/tmp/") =>
        {
            Command::Ignore
        }
        _ => Command::Unknown(full_command),
    }
}

fn execute_command(command: Command) {
    let window = GameWindow::find();

    match command {
        Command::Empty => {
            if let Some(w) = window {
                capture::screenshot(&w);
            } else {
                display_notification(Notification::ScreenshotFailed);
            }
            check_for_update();
        }
        Command::Connect => {
            println!("connected to Google Play Games");
        }
        Command::StartActivity { intent } => {
            apply_intent_package(&intent);
            start_game_if_needed();

            println!("Starting: Intent {{ cmp={} }}", intent);
            println!("Warning: Activity not started, intent has been delivered to currently running top-most instance.");
        }
        Command::Echo { text } => {
            println!("{}", text);
        }
        Command::ForceStop => {
            if let Some(w) = window {
                input::terminate(&w);
            } else {
                debug_log(LogLevel::Warn, LogMode::Nested, "ForceStop: Window not found");
            }
            display_notification(Notification::GpgShutdown);
        }
        Command::ToggleDebug => {
            toggle_debug();
        }

        Command::WindowList => {
            print_window_list();
        }
        Command::WindowDisplays => {
            println!("{} {}", DISPLAY_WIDTH, DISPLAY_HEIGHT);
            set_benchmark_mode(2);
        }
        Command::Devices => {
            println!("List of devices attached");
            println!("GooglePlayGames\tdevice");

            check_version();
            check_for_update();
        }
        Command::GetPropRelease => {
            println!("14");
        }
        Command::GetUuid => {
            println!("0000000000000000");
        }

        Command::Screencap => {
            if check_benchmark_mode() {
                thread::sleep(Duration::from_millis(BENCHMARK_DELAY_MS));
                debug_log(LogLevel::Info, LogMode::Nested, "Benchmark: Sent Black Frame (Encode)");
                capture::send_black_frame();
            } else if let Some(w) = window {
                capture::send_capture(&w);
            } else {
                debug_log(LogLevel::Info, LogMode::Nested, "Window not found: Sent Black Frame (Encode)");
                capture::send_black_frame();
            }
        }
        Command::ScreencapNc { port } => {
            if check_benchmark_mode() {
                debug_log(LogLevel::Info, LogMode::Nested, &format!("RawByNc: Connecting to 127.0.0.1:{}", port));
                debug_log(LogLevel::Info, LogMode::Nested, "Benchmark: Sent Black Frame (RawByNc)");
                capture::send_black_frame_nc(port);
            } else if let Some(w) = window {
                capture::send_capture_nc(&w, port);
            } else {
                debug_log(LogLevel::Info, LogMode::Nested, "Window not found: Sent Black Frame (RawByNc)");
                capture::send_black_frame_nc(port);
            }
        }

        Command::GetPropAbilist => {
            println!("x86_64,x86,arm64-v8a,armeabi-v7a,armeabi");
        }
        Command::DumpsysInputOrientation => {
            println!("0");
        }
        Command::MinitouchDaemon => {
            input::run_minitouch_daemon();
        }

        Command::Tap { x, y } => {
            if let Some(w) = window {
                input_adb::input_tap(&w, x, y);
            } else {
                debug_log(LogLevel::Warn, LogMode::Nested, "Tap: Window not found");
            }
        }
        Command::Swipe { x1, y1, x2, y2, duration } => {
            if let Some(w) = window {
                input_adb::input_swipe(&w, x1, y1, x2, y2, duration);
            } else {
                debug_log(LogLevel::Warn, LogMode::Nested, "Swipe: Window not found");
            }
        }
        Command::KeyEvent { keycode } => {
            if let Some(w) = window {
                input_adb::input_keyevent(&w, keycode);
            } else {
                debug_log(LogLevel::Warn, LogMode::Nested, "KeyEvent: Window not found");
            }
        }
        Command::Text { text } => {
            if let Some(w) = window {
                input_adb::input_text(&w, &text);
            } else {
                debug_log(LogLevel::Warn, LogMode::Nested, "Text: Window not found");
            }
        }

        Command::Ignore => {}
        Command::Unknown(cmd) => {
            display_notification(Notification::UnknownCommand(cmd));
        }
    }
}
