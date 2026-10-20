//! Saving resources in the background: while the window is closed or minimised and the user
//! asked for it, File Sorter reads less of each file (the name and first page, no text
//! recognition, no learned model, no comparing of versions by content), drops the search
//! index from memory and pauses work that can wait.

use std::sync::atomic::{AtomicBool, Ordering};

static ACTIVE: AtomicBool = AtomicBool::new(false);

pub fn set(on: bool) {
    ACTIVE.store(on, Ordering::Relaxed);
}

pub fn active() -> bool {
    ACTIVE.load(Ordering::Relaxed)
}

/// Give memory that is no longer used back to the system right away (Windows keeps it in
/// the process's working set otherwise, where Task Manager still counts it).
pub fn trim_memory() {
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::System::Memory::{GetProcessHeap, HeapCompact};
        use windows_sys::Win32::System::Threading::{GetCurrentProcess, SetProcessWorkingSetSize};
        HeapCompact(GetProcessHeap(), 0);
        SetProcessWorkingSetSize(GetCurrentProcess(), usize::MAX, usize::MAX);
    }
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    {
        unsafe extern "C" {
            fn malloc_trim(pad: usize) -> i32;
        }
        // SAFETY: glibc's malloc_trim only hands free memory back to the system.
        unsafe {
            malloc_trim(0);
        }
    }
}
