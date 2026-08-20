use std::{env, path::Path, time::Instant};

mod daemon;
mod game;
mod shared;
mod shim;
mod sys;

use windows::Win32::UI::HiDpi::{SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2};

use crate::sys::logging::{debug_log, LogLevel, LogMode};
use crate::sys::process::mutex_exists;

fn main() {
    unsafe { _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) }

    let start = Instant::now();

    sys::logging::register_panic_hook();

    // No daemon left means no block is open anywhere,
    // which is the only moment a stranded depth can be told apart from a real one.
    if !mutex_exists(daemon::wgc::DAEMON_MUTEX) && !mutex_exists(daemon::launcher::LAUNCHER_MUTEX) {
        sys::logging::reset_log_depth();
    }

    sys::logging::rotate_log();

    // We spawn our own daemons by full path while MAA calls us by name, so argv[0] alone would
    // make the same process read two different ways in the log.
    let mut raw_args: Vec<String> = env::args().collect();
    if let Some(exe) = raw_args.first_mut() {
        if let Some(name) = Path::new(exe.as_str()).file_name() {
            *exe = name.to_string_lossy().into_owned();
        }
    }
    let full_joined = raw_args.join(" ");
    let args: Vec<String> = raw_args[1..].iter().flat_map(|s| s.split_whitespace()).map(String::from).collect();

    debug_log(LogLevel::Info, LogMode::Start, &full_joined);

    // Publish exe path so the fake nemu DLL can spawn the WGC daemon.
    if let Ok(exe) = env::current_exe() {
        let _ = sys::config::set_registry(shared::KEY_EXE_PATH, exe.to_string_lossy().as_ref(), shared::REG_PATH_STATE);
    }

    // The daemon outlives this call, so the call block closes before it runs instead of after.
    let close_call = || debug_log(LogLevel::Info, LogMode::End, &format!("{} ms", start.elapsed().as_millis()));

    if args.iter().any(|a| a == "--wgc-daemon") {
        close_call();
        daemon::wgc::run_daemon();
        return;
    }

    if args.iter().any(|a| a == daemon::launcher::LAUNCHER_ARG) {
        close_call();
        daemon::launcher::run_launcher_daemon();
        return;
    }

    // shell /data/local/tmp/<uuid> -i
    if args.iter().any(|a| a == "-i") {
        close_call();
        daemon::minitouch::run_minitouch_daemon();
        return;
    }

    let command = shim::parse_command(&args);
    // Its frame goes out over a socket, so an empty stdout here is not silence.
    let answers_off_stdout = matches!(command, shim::Command::ScreencapNc { .. });
    shim::execute_command(command);
    if !answers_off_stdout {
        sys::logging::log_silent_reply();
    }

    close_call();
}
