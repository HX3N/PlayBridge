//! The one-shot side of the bin: what MAA gets when it calls us like `adb`.

use crate::daemon::launcher::{apply_intent_package, ensure_launcher};
use crate::daemon::wgc;
use crate::game::capture;
use crate::game::input;
use crate::game::window::GameWindow;
use crate::shared;
use crate::sys::config::{self, check_for_update, check_version, set_client, toggle_touch_overlay, Client, DISPLAY_HEIGHT, DISPLAY_WIDTH};
use crate::sys::logging::{debug_log, reply, LogLevel, LogMode};
use crate::sys::maa;
use crate::sys::notification::{display_notification, Notification};

pub enum Command {
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

pub fn parse_command(args: &[String]) -> Command {
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

        // `disconnect` never reaches here — the `connect` arm above matches it first and answers it.
        c if c.contains("cat /proc/net/arp")
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

/// Only this process has MAA as its parent, so nowhere else can read the config it selected.
/// False means MAA is set to a client with no Google Play Games package, so nothing should be launched.
fn apply_maa_client() -> bool {
    if let Some(pid) = maa::parent_pid() {
        let _ = config::set_registry(shared::KEY_MAA_PID, pid, config::REG_PATH_STATE);
    }

    let Some(client_type) = maa::client_type() else {
        debug_log(LogLevel::Warn, LogMode::Nested, "Client: MAA config unreadable, falling back to window title");
        return true;
    };

    let Some(client) = Client::from_maa_client_type(&client_type) else {
        display_notification(Notification::UnsupportedClient(client_type));
        return false;
    };

    // set_client is silent when nothing changes, which would leave no trace that MAA was read at all.
    debug_log(LogLevel::Info, LogMode::Nested, &format!("Client: {} from MAA config", client.as_str()));
    set_client(client);
    true
}

pub fn execute_command(command: Command) {
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
            reply("connected to Google Play Games");
        }
        Command::StartActivity { intent } => {
            if apply_intent_package(&intent) {
                ensure_launcher();
            }

            reply(&format!("Starting: Intent {{ cmp={} }}", intent));
            reply("Warning: Activity not started, intent has been delivered to currently running top-most instance.");
        }
        Command::Echo { text } => {
            reply(&text);
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
            reply(&format!("{} {}", DISPLAY_WIDTH, DISPLAY_HEIGHT));
        }
        Command::Devices => {
            // Must be host:port with a port get_mumu_index() accepts (7555, >=16384, or >=5555), or MAA skips MumuExtras.
            reply("List of devices attached");
            reply("127.0.0.1:6000\tdevice");

            check_version();
            check_for_update();

            // Preload: GPG takes a while to come up, so the launcher starts before MAA asks for a screen.
            if apply_maa_client() {
                ensure_launcher();
            }
        }
        Command::GetPropRelease => {
            reply("14");
        }
        Command::GetUuid => {
            reply("0000000000000000");
        }
        Command::Fps => {
            // 60fps frame period in ns, the value MAA's fps probe reads as the first SurfaceFlinger --latency line.
            reply("16666666");
        }

        Command::ScreencapNc { port } => {
            if wgc::deliver_via_daemon(port) {
                debug_log(LogLevel::Info, LogMode::Nested, "RawByNc: delivered via WGC daemon");
            } else {
                // Daemon not warm yet: spawn it and send a black frame this once; the next request hits the warm daemon.
                wgc::ensure_daemon();
                debug_log(LogLevel::Info, LogMode::Nested, "RawByNc: daemon cold, spawned + sent black frame");
                capture::send_black_frame_nc(port);
            }
        }

        Command::GetPropAbilist => {
            reply("x86_64,x86,arm64-v8a,armeabi-v7a,armeabi");
        }
        Command::DumpsysInputOrientation => {
            reply("0");
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
