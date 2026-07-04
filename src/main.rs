use std::{env, time::Instant};

mod capture;
mod config;
mod input;
mod logging;
mod notification;
mod store;
mod wgc;
mod window;

use windows::Win32::UI::HiDpi::{SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2};

use crate::config::{check_benchmark_mode, check_for_update, check_version, toggle_touch_overlay, DISPLAY_HEIGHT, DISPLAY_WIDTH};
use crate::config::{peek_benchmark_mode, set_benchmark_mode};
use crate::logging::{debug_log, LogLevel, LogMode};
use crate::notification::{display_notification, Notification};
use crate::window::{apply_intent_package, ensure_game_ready, start_game_if_needed, GameWindow};

fn main() {
    unsafe { _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) }

    let start = Instant::now();

    logging::register_panic_hook();
    logging::rotate_log();

    let raw_args: Vec<String> = env::args().collect();
    let full_joined = raw_args.join(" ");
    let args: Vec<String> = raw_args[1..].iter().flat_map(|s| s.split_whitespace()).map(String::from).collect();

    debug_log(LogLevel::Info, LogMode::Start, &full_joined);

    // Publish exe path so the fake nemu DLL can spawn the WGC daemon.
    if let Ok(exe) = env::current_exe() {
        let _ = config::set_registry("EXE_PATH", exe.to_string_lossy().as_ref(), config::REG_PATH_STATE);
    }

    if args.iter().any(|a| a == "--wgc-daemon") {
        wgc::run_daemon();
        return;
    }

    // shell /data/local/tmp/<uuid> -i
    if args.iter().any(|a| a == "-i") {
        input::run_minitouch_daemon();
        return;
    }

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
    ToggleTouchOverlay,

    // [Info & State]
    WindowDisplays,
    Devices,
    GetPropRelease,
    GetUuid,
    Fps,

    // [Screen Capture]
    ScreencapNc { port: u16 },

    // [Minitouch]
    GetPropAbilist,
    DumpsysInputOrientation,

    // [Key & Text Input]
    KeyEvent { keycode: i32 },
    Text { text: String },

    // [Unsupported ADB Touch Input]
    AdbInputUnsupported,

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
        // Connection Preset - Compatible Mode
        c if c.contains("shell echo") => Command::Echo { text: args.get(4..).map_or(String::new(), |s| s.join(" ")) },
        c if c.contains("am force-stop") || c.contains("input keyevent HOME") => Command::ForceStop,
        c if c.contains("--touch-overlay") => Command::ToggleTouchOverlay,

        c if c.contains("dumpsys window displays") || c.contains("wm size") => Command::WindowDisplays,
        c if c.contains("devices") => Command::Devices,
        c if c.contains("getprop ro.build.version.release") => Command::GetPropRelease,
        c if c.contains("settings get secure android_id") => Command::GetUuid,
        c if c.contains("dumpsys SurfaceFlinger") => Command::Fps,

        c if c.contains("exec-out screencap | nc -w 3 10.0.2.2") => Command::ScreencapNc { port: args[9].parse().unwrap_or(0) },

        c if c.contains("ro.product.cpu.abilist") => Command::GetPropAbilist,
        c if c.contains("dumpsys input") && c.contains("SurfaceOrientation") => Command::DumpsysInputOrientation,

        c if c.contains("input tap") || c.contains("input swipe") => Command::AdbInputUnsupported,
        c if c.contains("input keyevent") => Command::KeyEvent { keycode: args[5].parse().unwrap_or(0) },
        c if c.contains("input text") => Command::Text { text: args[5..].join(" ") },

        c if c.contains("cat /proc/net/arp")
            || c.contains("disconnect")
            // Unsupported screencap modes (Encode -p, gzip): stay silent so MAA falls back instead of an "unknown command" toast.
            || c.contains("exec-out screencap -p")
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
                debug_log(LogLevel::Warn, LogMode::Nested, "ForceStop: window not found");
            }
            display_notification(Notification::GpgShutdown);
        }
        Command::ToggleTouchOverlay => {
            toggle_touch_overlay();
        }

        Command::WindowDisplays => {
            println!("{} {}", DISPLAY_WIDTH, DISPLAY_HEIGHT);
            set_benchmark_mode();
        }
        Command::Devices => {
            // Must be host:port with a port get_mumu_index() accepts (7555, >=16384, or >=5555), or MAA skips MumuExtras.
            println!("List of devices attached");
            println!("127.0.0.1:6000\tdevice");

            check_version();
            check_for_update();
        }
        Command::GetPropRelease => {
            println!("14");
        }
        Command::GetUuid => {
            println!("0000000000000000");
        }
        Command::Fps => {
            // 60fps frame period in ns, the value MAA's fps probe reads as the first SurfaceFlinger --latency line.
            println!("16666666");
        }

        Command::ScreencapNc { port } => {
            if check_benchmark_mode() {
                debug_log(LogLevel::Info, LogMode::Nested, "Benchmark: sent black frame (RawByNc)");
                capture::send_black_frame_nc(port);
            } else if wgc::deliver_via_daemon(port) {
                debug_log(LogLevel::Info, LogMode::Nested, "RawByNc: delivered via WGC daemon");
            } else {
                // Daemon not warm yet: spawn it and send a black frame this once; the next request hits the warm daemon.
                wgc::ensure_daemon();
                debug_log(LogLevel::Info, LogMode::Nested, "RawByNc: daemon cold, spawned + sent black frame");
                capture::send_black_frame_nc(port);
            }
        }

        Command::GetPropAbilist => {
            println!("x86_64,x86,arm64-v8a,armeabi-v7a,armeabi");
        }
        Command::DumpsysInputOrientation => {
            println!("0");
        }
        Command::KeyEvent { keycode } => {
            if let Some(w) = window {
                input::input_keyevent(&w, keycode);
            } else {
                debug_log(LogLevel::Warn, LogMode::Nested, "KeyEvent: window not found");
            }
        }
        Command::Text { text } => {
            if let Some(w) = window {
                input::input_text(&w, &text);
            } else {
                debug_log(LogLevel::Warn, LogMode::Nested, "Text: window not found");
            }
        }

        Command::AdbInputUnsupported => {
            display_notification(Notification::AdbInputUnsupported);
        }

        Command::Ignore => {}
        Command::Unknown(cmd) => {
            display_notification(Notification::UnknownCommand(cmd));
        }
    }
}
