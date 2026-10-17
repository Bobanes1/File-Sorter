//! Desktop integration: the "Sort with File Sorter" entry in Explorer's right-click
//! menu, starting at sign-in, and showing a file in its folder.

use std::io;
use std::path::{Path, PathBuf};

/// Where Explorer looks for per-user context menu entries.
#[cfg_attr(not(windows), allow(dead_code))]
pub const CLASSES: &str = r"Software\Classes";

/// The command Explorer runs for a right-clicked file or folder.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn menu_command(exe: &Path) -> String {
    format!("\"{}\" --sort \"%1\"", exe.display())
}

/// What runs at sign-in: the window, minimised and watching.
#[cfg_attr(not(any(windows, target_os = "linux")), allow(dead_code))]
pub fn autostart_command(exe: &Path) -> String {
    format!("\"{}\" --autostart", exe.display())
}

/// Where Windows keeps the programs that start at sign-in, and our entry's name.
#[cfg_attr(not(windows), allow(dead_code))]
pub const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
#[cfg_attr(not(windows), allow(dead_code))]
pub const RUN_VALUE: &str = "FileSorter";

#[cfg(windows)]
mod imp {
    use super::*;
    use winreg::RegKey;
    use winreg::enums::HKEY_CURRENT_USER;

    /// One entry for files (`*`) and one for folders.
    fn keys(classes: &str) -> [String; 2] {
        [format!(r"{classes}\*\shell\FileSorter"), format!(r"{classes}\Directory\shell\FileSorter")]
    }

    pub fn register(classes: &str, exe: &Path, label: &str) -> io::Result<()> {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        for key in keys(classes) {
            let (entry, _) = hkcu.create_subkey(&key)?;
            entry.set_value("", &label)?;
            entry.set_value("Icon", &format!("\"{}\",0", exe.display()))?;
            // One File Sorter per selected item (they hand their files to one window), also
            // when several files are selected.
            entry.set_value("MultiSelectModel", &"Document")?;
            let (command, _) = entry.create_subkey("command")?;
            command.set_value("", &menu_command(exe))?;
        }
        Ok(())
    }

    pub fn unregister(classes: &str) -> io::Result<()> {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        for key in keys(classes) {
            match hkcu.delete_subkey_all(&key) {
                Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
                _ => {}
            }
        }
        Ok(())
    }

    pub fn set_autostart(run_key: &str, exe: &Path, enabled: bool) -> io::Result<()> {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let (key, _) = hkcu.create_subkey(run_key)?;
        if enabled {
            return key.set_value(RUN_VALUE, &autostart_command(exe));
        }
        match key.delete_value(RUN_VALUE) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        }
    }

    #[cfg(test)]
    pub fn autostart_entry(run_key: &str) -> Option<String> {
        RegKey::predef(HKEY_CURRENT_USER).open_subkey(run_key).ok()?.get_value(RUN_VALUE).ok()
    }

    /// The command currently registered for files, if any.
    #[cfg(test)]
    pub fn registered_command(classes: &str) -> Option<String> {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        hkcu.open_subkey(format!(r"{}\command", keys(classes)[0])).ok()?.get_value("").ok()
    }
}

/// Add (or refresh) the right-click entry for the running exe. Windows only.
pub fn register_menu(label: &str) -> io::Result<()> {
    #[cfg(windows)]
    {
        let exe = std::env::current_exe()?;
        imp::register(CLASSES, &exe, label)
    }
    #[cfg(not(windows))]
    {
        let _ = label;
        Ok(())
    }
}

/// Remove the right-click entry. Windows only.
pub fn unregister_menu() -> io::Result<()> {
    #[cfg(windows)]
    {
        imp::unregister(CLASSES)
    }
    #[cfg(not(windows))]
    {
        Ok(())
    }
}

/// Keep the menu entry in line with the setting (and with where the exe lives now).
pub fn apply_menu_setting(enabled: bool, label: &str) {
    let result = if enabled { register_menu(label) } else { unregister_menu() };
    if let Err(e) = result {
        log::info!("Could not update the right-click menu: {e}");
    }
}

/// True where File Sorter can start itself at sign-in.
pub fn autostart_supported() -> bool {
    cfg!(any(windows, target_os = "linux"))
}

/// The autostart file of desktop Linux (XDG): ~/.config/autostart/file-sorter.desktop.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn linux_autostart_file() -> Option<PathBuf> {
    dirs::config_dir().map(|dir| dir.join("autostart").join("file-sorter.desktop"))
}

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn set_linux_autostart(file: &Path, exe: &Path, enabled: bool) -> io::Result<()> {
    if !enabled {
        return match std::fs::remove_file(file) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        };
    }
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let entry = format!(
        "[Desktop Entry]\nType=Application\nName=File Sorter\nExec={}\nX-GNOME-Autostart-enabled=true\n",
        autostart_command(exe)
    );
    std::fs::write(file, entry)
}

/// Start (or stop starting) File Sorter at sign-in, for the exe at its current place.
pub fn set_autostart(enabled: bool) -> io::Result<()> {
    #[cfg(windows)]
    {
        imp::set_autostart(RUN_KEY, &std::env::current_exe()?, enabled)
    }
    #[cfg(target_os = "linux")]
    {
        let file = linux_autostart_file().ok_or_else(|| io::Error::other("no config folder"))?;
        set_linux_autostart(&file, &std::env::current_exe()?, enabled)
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        let _ = enabled;
        Ok(())
    }
}

/// Keep the sign-in entry in line with the setting (and with where the exe lives now).
pub fn apply_autostart_setting(enabled: bool) {
    if let Err(e) = set_autostart(enabled) {
        log::info!("Could not update starting at sign-in: {e}");
    }
}

/// Open a file or folder with its usual program.
pub fn open(path: &Path) {
    #[cfg(windows)]
    let result = std::process::Command::new("explorer").arg(path).spawn();
    #[cfg(target_os = "macos")]
    let result = std::process::Command::new("open").arg(path).spawn();
    #[cfg(not(any(windows, target_os = "macos")))]
    let result = std::process::Command::new("xdg-open").arg(path).spawn();
    if let Err(e) = result {
        log::info!("Could not open {}: {e}", path.display());
    }
}

/// Open the folder that contains `path`, with the file selected where possible.
pub fn reveal(path: &Path) {
    #[cfg(windows)]
    let result = std::process::Command::new("explorer").arg(format!("/select,{}", path.display())).spawn();
    #[cfg(target_os = "macos")]
    let result = std::process::Command::new("open").arg("-R").arg(path).spawn();
    #[cfg(not(any(windows, target_os = "macos")))]
    let result = std::process::Command::new("xdg-open").arg(path.parent().unwrap_or(path)).spawn();
    if let Err(e) = result {
        log::info!("Could not open the folder of {}: {e}", path.display());
    }
}

#[cfg(all(test, target_os = "linux"))]
mod linux_tests {
    use super::*;

    #[test]
    fn autostart_desktop_file() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("autostart").join("file-sorter.desktop");
        let exe = Path::new("/opt/File Sorter/file-sorter");
        set_linux_autostart(&file, exe, true).unwrap();
        let entry = std::fs::read_to_string(&file).unwrap();
        assert!(entry.contains("Exec=\"/opt/File Sorter/file-sorter\" --autostart\n"), "{entry}");
        set_linux_autostart(&file, exe, false).unwrap();
        assert!(!file.exists());
        set_linux_autostart(&file, exe, false).unwrap(); // removing twice is fine
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::imp::*;
    use super::*;

    #[test]
    fn autostart_entry_on_and_off() {
        let run_key = r"Software\FileSorterAutostartTest\Run";
        let exe = Path::new(r"C:\Tools\File Sorter\FileSorter.exe");
        imp::set_autostart(run_key, exe, true).unwrap();
        assert_eq!(autostart_entry(run_key).as_deref(), Some(r#""C:\Tools\File Sorter\FileSorter.exe" --autostart"#));
        imp::set_autostart(run_key, exe, false).unwrap();
        assert_eq!(autostart_entry(run_key), None);
        imp::set_autostart(run_key, exe, false).unwrap();
        let _ = winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER)
            .delete_subkey_all(r"Software\FileSorterAutostartTest");
    }

    #[test]
    fn register_and_unregister_menu_entry() {
        let classes = r"Software\FileSorterTest\Classes";
        let exe = Path::new(r"C:\Tools\File Sorter\FileSorter.exe");
        register(classes, exe, "Mit File Sorter sortieren").unwrap();
        assert_eq!(
            registered_command(classes).as_deref(),
            Some(r#""C:\Tools\File Sorter\FileSorter.exe" --sort "%1""#)
        );
        unregister(classes).unwrap();
        assert_eq!(registered_command(classes), None);
        unregister(classes).unwrap(); // removing twice is fine
        let _ = winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER).delete_subkey_all(r"Software\FileSorterTest");
    }
}
