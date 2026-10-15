//! Window control through Windows itself: minimising, putting the window into the
//! notification area ("tray") and getting it back, and waking an already running
//! File Sorter instead of opening a second window.
//!
//! egui's own `Minimized` command left the window on screen while eframe treated it as
//! minimised and stopped drawing, so these go straight to Windows.

/// The main window, as Windows knows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MainWindow {
    #[cfg_attr(not(windows), allow(dead_code))]
    hwnd: isize,
}

#[cfg(windows)]
mod imp {
    use super::MainWindow;
    use windows_sys::Win32::Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE, HWND};
    use windows_sys::Win32::System::Threading::{
        CreateEventW, EVENT_MODIFY_STATE, INFINITE, OpenEventW, SetEvent, WaitForSingleObject,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        ASFW_ANY, AllowSetForegroundWindow, GWL_EXSTYLE, GetWindowLongPtrW, IsIconic, IsWindowVisible, PostMessageW,
        SC_MINIMIZE, SPI_GETWORKAREA, SW_HIDE, SW_MINIMIZE, SW_RESTORE, SW_SHOWMINNOACTIVE, SetForegroundWindow,
        SetWindowLongPtrW, ShowWindow, SystemParametersInfoW, WM_SYSCOMMAND, WS_EX_APPWINDOW, WS_EX_TOOLWINDOW,
    };

    impl MainWindow {
        fn hwnd(self) -> HWND {
            self.hwnd as HWND
        }

        /// Like clicking the window's minimise button.
        pub fn minimise(self) {
            // SAFETY: plain Win32 calls on our own window.
            unsafe {
                ShowWindow(self.hwnd(), SW_MINIMIZE);
                if IsIconic(self.hwnd()) == 0 {
                    PostMessageW(self.hwnd(), WM_SYSCOMMAND, SC_MINIMIZE as usize, 0);
                }
            }
        }

        /// Into the notification area: minimised and without a taskbar button. (A minimised
        /// window keeps getting eframe's logic passes, unlike a hidden one.)
        pub fn to_tray(self) {
            let hwnd = self.hwnd();
            // SAFETY: plain Win32 calls on our own window.
            unsafe {
                ShowWindow(hwnd, SW_HIDE);
                let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
                SetWindowLongPtrW(hwnd, GWL_EXSTYLE, (style | WS_EX_TOOLWINDOW as isize) & !(WS_EX_APPWINDOW as isize));
                ShowWindow(hwnd, SW_SHOWMINNOACTIVE);
            }
        }

        /// Back from the notification area (or from minimised), in front of everything.
        pub fn bring_back(self) {
            let hwnd = self.hwnd();
            // SAFETY: plain Win32 calls on our own window.
            unsafe {
                let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
                if style & WS_EX_TOOLWINDOW as isize != 0 {
                    ShowWindow(hwnd, SW_HIDE);
                    SetWindowLongPtrW(hwnd, GWL_EXSTYLE, style & !(WS_EX_TOOLWINDOW as isize));
                }
                ShowWindow(hwnd, SW_RESTORE);
                SetForegroundWindow(hwnd);
            }
        }

        pub fn in_tray(self) -> bool {
            // SAFETY: reading our own window's style.
            unsafe { GetWindowLongPtrW(self.hwnd(), GWL_EXSTYLE) & WS_EX_TOOLWINDOW as isize != 0 }
        }

        /// Minimised, or not shown at all.
        pub fn is_minimised(self) -> bool {
            // SAFETY: reading our own window's state.
            unsafe { IsIconic(self.hwnd()) != 0 || IsWindowVisible(self.hwnd()) == 0 }
        }
    }

    /// The name of the event a second File Sorter uses to wake this one; one per settings file.
    fn event_name(key: &str) -> Vec<u16> {
        let clean: String = key.chars().map(|c| if c.is_alphanumeric() { c } else { '_' }).collect();
        let tail: String = clean.chars().rev().take(180).collect::<Vec<_>>().into_iter().rev().collect();
        format!("Local\\FileSorter.Show.{tail}").encode_utf16().chain([0]).collect()
    }

    pub struct Wakeup {
        event: HANDLE,
    }

    // SAFETY: an event handle can be used from any thread.
    unsafe impl Send for Wakeup {}

    impl Drop for Wakeup {
        fn drop(&mut self) {
            // SAFETY: we own the handle.
            unsafe { CloseHandle(self.event) };
        }
    }

    pub fn claim(key: &str) -> Option<Wakeup> {
        let name = event_name(key);
        // SAFETY: creating a named auto-reset event; the handle is closed in Drop.
        unsafe {
            let event = CreateEventW(std::ptr::null(), 0, 0, name.as_ptr());
            if event.is_null() {
                return Some(Wakeup { event }); // can't coordinate; just run
            }
            if GetLastError() == ERROR_ALREADY_EXISTS {
                CloseHandle(event);
                return None;
            }
            Some(Wakeup { event })
        }
    }

    pub fn wake_other(key: &str) -> bool {
        let name = event_name(key);
        // SAFETY: opening, signalling and closing a named event.
        unsafe {
            let event = OpenEventW(EVENT_MODIFY_STATE, 0, name.as_ptr());
            if event.is_null() {
                return false;
            }
            // We were started by the user, so we may let the other one come to the front.
            AllowSetForegroundWindow(ASFW_ANY);
            let ok = SetEvent(event) != 0;
            CloseHandle(event);
            ok
        }
    }

    pub fn wait(wakeup: &Wakeup) -> bool {
        // SAFETY: waiting on our own event handle.
        !wakeup.event.is_null() && unsafe { WaitForSingleObject(wakeup.event, INFINITE) } == 0
    }

    /// The part of the main screen not covered by the taskbar, in physical pixels.
    pub fn work_area() -> Option<[f32; 4]> {
        let mut rect = windows_sys::Win32::Foundation::RECT { left: 0, top: 0, right: 0, bottom: 0 };
        // SAFETY: SPI_GETWORKAREA writes a RECT.
        let ok = unsafe { SystemParametersInfoW(SPI_GETWORKAREA, 0, (&raw mut rect).cast(), 0) } != 0;
        ok.then_some([rect.left as f32, rect.top as f32, rect.right as f32, rect.bottom as f32])
    }

    /// The scale of the main screen (1.0 at 96 dpi).
    pub fn screen_scale() -> f32 {
        // SAFETY: no arguments.
        let dpi = unsafe { windows_sys::Win32::UI::HiDpi::GetDpiForSystem() };
        if dpi == 0 { 1.0 } else { dpi as f32 / 96.0 }
    }
}

#[cfg(not(windows))]
mod imp {
    use super::MainWindow;

    impl MainWindow {
        pub fn minimise(self) {}
        pub fn is_minimised(self) -> bool {
            false
        }
        pub fn to_tray(self) {}
        pub fn bring_back(self) {}
        pub fn in_tray(self) -> bool {
            false
        }
    }

    pub struct Wakeup;

    pub fn claim(_key: &str) -> Option<Wakeup> {
        Some(Wakeup)
    }

    pub fn wake_other(_key: &str) -> bool {
        false
    }

    pub fn wait(_wakeup: &Wakeup) -> bool {
        false
    }

    pub fn work_area() -> Option<[f32; 4]> {
        None
    }

    pub fn screen_scale() -> f32 {
        1.0
    }
}

pub use imp::{screen_scale, work_area};

impl MainWindow {
    /// The window eframe opened, once it exists.
    pub fn of(frame: &eframe::Frame) -> Option<MainWindow> {
        use raw_window_handle::{HasWindowHandle, RawWindowHandle};
        match frame.window_handle().ok()?.as_raw() {
            RawWindowHandle::Win32(handle) => Some(MainWindow { hwnd: handle.hwnd.get() }),
            _ => None,
        }
    }
}

/// Proof that this is the only File Sorter window for these settings.
pub struct SingleInstance(imp::Wakeup);

/// Become the only File Sorter window for these settings (`key`: the settings file).
/// If one is already running, it is brought to the front instead and `None` is returned.
pub fn single_instance(key: &str, wake_other: bool) -> Option<SingleInstance> {
    match imp::claim(key) {
        Some(wakeup) => Some(SingleInstance(wakeup)),
        None => {
            if wake_other {
                imp::wake_other(key);
            }
            None
        }
    }
}

impl SingleInstance {
    /// Call `on_wake` (on a background thread) whenever File Sorter is started again.
    pub fn on_wake(self, on_wake: impl Fn() + Send + 'static) {
        if !cfg!(windows) {
            return;
        }
        std::thread::Builder::new()
            .name("file-sorter-wakeup".into())
            .spawn(move || {
                while imp::wait(&self.0) {
                    on_wake();
                }
            })
            .ok();
    }
}
