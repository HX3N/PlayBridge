//! Values the bin and the cdylib must agree on. Separate crates cannot share a module tree, but both
//! roots sit in `src/`, so each declares `mod shared;` over this one file instead of keeping copies in step.

pub const REG_PATH_STATE: &str = r"Software\PlayBridge\state";
pub const KEY_DAEMON_PORT: &str = "WGC_DAEMON_PORT";
pub const KEY_EXE_PATH: &str = "EXE_PATH";
/// The MAA that ran `devices`. The daemon has no other way to tell that MAA is gone.
#[allow(dead_code)]
pub const KEY_MAA_PID: &str = "MAA_PID";

/// The daemon resizes to this, and MAA's reported `wm size` matches it so click coordinates line up.
pub const DISPLAY_WIDTH: u32 = 1280;
pub const DISPLAY_HEIGHT: u32 = 720;

pub const DETACHED_PROCESS: u32 = 0x0000_0008;
pub const CREATE_NO_WINDOW: u32 = 0x0800_0000;
