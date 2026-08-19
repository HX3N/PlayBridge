//! Named-mutex helpers the daemons use to claim their singleton slot.

use windows::core::PCWSTR;
use windows::Win32::Foundation::{CloseHandle, GetLastError, ERROR_ALREADY_EXISTS};
use windows::Win32::System::Threading::{CreateMutexW, OpenMutexW, SYNCHRONIZATION_SYNCHRONIZE};

/// Claims the name for this process, so only a process that means to *be* the daemon may call it.
pub fn already_running(mutex_name: &str) -> bool {
    let name: Vec<u16> = mutex_name.encode_utf16().chain(Some(0)).collect();
    unsafe {
        let _ = CreateMutexW(None, false, PCWSTR(name.as_ptr()));
        GetLastError() == ERROR_ALREADY_EXISTS
    }
}

/// Checks without claiming, which `already_running` cannot do: creating the mutex would make the
/// caller its owner and every later check would report a daemon that never started.
pub fn mutex_exists(mutex_name: &str) -> bool {
    let name: Vec<u16> = mutex_name.encode_utf16().chain(Some(0)).collect();
    match unsafe { OpenMutexW(SYNCHRONIZATION_SYNCHRONIZE, false, PCWSTR(name.as_ptr())) } {
        Ok(handle) => {
            unsafe {
                let _ = CloseHandle(handle);
            }
            true
        }
        Err(_) => false,
    }
}
