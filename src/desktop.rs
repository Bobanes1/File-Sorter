//! Desktop integration: the "Sort with File Sorter" entry in Explorer's right-click
//! menu, and showing a file in its folder.

use std::io;
use std::path::Path;

/// Where Explorer looks for per-user context menu entries.
#[cfg_attr(not(windows), allow(dead_code))]
pub const CLASSES: &str = r"Software\Classes";

/// The command Explorer runs for a right-clicked file or folder.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn menu_command(exe: &Path) -> String {
    format!("\"{}\" --sort \"%1\"", exe.display())
}

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

#[cfg(all(test, windows))]
mod tests {
    use super::imp::*;
    use super::*;

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
