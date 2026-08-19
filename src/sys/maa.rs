//! The MAA process that invoked us: the client it selected, and whether it is still there.
//! Only the process handling `devices` has MAA as its parent — a detached daemon does not,
//! which is why its PID is published for the daemons to poll.

use std::{fs, mem, path::Path, path::PathBuf};

use windows::core::PWSTR;
use windows::Win32::{
    Foundation::{CloseHandle, MAX_PATH, WAIT_TIMEOUT},
    System::{
        Diagnostics::ToolHelp::{CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS},
        Threading::{
            OpenProcess, QueryFullProcessImageNameW, WaitForSingleObject, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
            PROCESS_SYNCHRONIZE,
        },
    },
};

use crate::shared::KEY_MAA_PID;
use crate::sys::config::{get_registry, REG_PATH_STATE};

const CONFIG_RELATIVE_PATH: &str = "config/gui.new.json";

/// None when the parent is not MAA, or its config is missing or unreadable.
pub fn client_type() -> Option<String> {
    let dir = parent_exe_dir()?;
    read_client_type(&dir.join(CONFIG_RELATIVE_PATH))
}

fn parent_exe_dir() -> Option<PathBuf> {
    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, parent_pid()?) }.ok()?;

    let mut buffer = [0u16; MAX_PATH as usize];
    let mut len = buffer.len() as u32;
    let query = unsafe { QueryFullProcessImageNameW(handle, PROCESS_NAME_WIN32, PWSTR(buffer.as_mut_ptr()), &mut len) };
    unsafe {
        let _ = CloseHandle(handle);
    }
    query.ok()?;

    let exe = PathBuf::from(String::from_utf16_lossy(&buffer[..len as usize]));
    exe.parent().map(Path::to_path_buf)
}

pub fn parent_pid() -> Option<u32> {
    let pid = std::process::id();
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) }.ok()?;

    let mut entry = PROCESSENTRY32W { dwSize: mem::size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };

    let mut parent = None;
    if unsafe { Process32FirstW(snapshot, &mut entry) }.is_ok() {
        loop {
            if entry.th32ProcessID == pid {
                parent = Some(entry.th32ParentProcessID);
                break;
            }
            if unsafe { Process32NextW(snapshot, &mut entry) }.is_err() {
                break;
            }
        }
    }

    unsafe {
        let _ = CloseHandle(snapshot);
    }
    parent
}

/// False once the MAA that ran `devices` is gone. Unknown or unreadable PIDs report true, so a
/// failed lookup never ends a daemon on its own.
pub fn is_alive() -> bool {
    let pid: u32 = get_registry(KEY_MAA_PID, 0u32, REG_PATH_STATE);
    if pid == 0 {
        return true;
    }
    let Ok(handle) = (unsafe { OpenProcess(PROCESS_SYNCHRONIZE, false, pid) }) else {
        return false;
    };
    let alive = unsafe { WaitForSingleObject(handle, 0) } == WAIT_TIMEOUT;
    unsafe {
        let _ = CloseHandle(handle);
    }
    alive
}

fn read_client_type(path: &Path) -> Option<String> {
    let json: serde_json::Value = serde_json::from_str(&fs::read_to_string(path).ok()?).ok()?;
    let current = json["Current"].as_str()?;
    json["Configurations"][current]["Gui"]["RuntimeSettings"]["ClientType"].as_str().map(str::to_owned)
}
