//! Named-mutex helpers the daemons use to claim their singleton slot.

use windows::core::PCWSTR;
use windows::Win32::Foundation::{CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, HANDLE, WAIT_ABANDONED, WAIT_OBJECT_0};
use windows::Win32::System::Threading::{CreateMutexW, OpenMutexW, ReleaseMutex, WaitForSingleObject, SYNCHRONIZATION_SYNCHRONIZE};

/// Claims the name for this process, so only a process that means to *be* the daemon may call it.
pub fn already_running(mutex_name: &str) -> bool {
    let name: Vec<u16> = mutex_name.encode_utf16().chain(Some(0)).collect();
    unsafe {
        let _ = CreateMutexW(None, false, PCWSTR(name.as_ptr()));
        GetLastError() == ERROR_ALREADY_EXISTS
    }
}

pub struct NamedLock(HANDLE);

impl Drop for NamedLock {
    fn drop(&mut self) {
        unsafe {
            let _ = ReleaseMutex(self.0);
            let _ = CloseHandle(self.0);
        }
    }
}

/// An abandoned mutex counts as acquired:
/// a holder that died mid-update would otherwise lock the name for good.
pub fn acquire_lock(mutex_name: &str, timeout_ms: u32) -> Option<NamedLock> {
    let name: Vec<u16> = mutex_name.encode_utf16().chain(Some(0)).collect();
    let handle = unsafe { CreateMutexW(None, false, PCWSTR(name.as_ptr())) }.ok()?;

    match unsafe { WaitForSingleObject(handle, timeout_ms) } {
        WAIT_OBJECT_0 | WAIT_ABANDONED => Some(NamedLock(handle)),
        _ => {
            unsafe { _ = CloseHandle(handle) };
            None
        }
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
