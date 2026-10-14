//! Remove Windows' "downloaded from the web" mark from the program and its shortcuts.
//!
//! Windows stores that mark in a hidden `Zone.Identifier` stream next to a downloaded
//! file. Shortcuts pinned from a marked exe carry it too, and Windows' app control
//! (Smart App Control) refuses to open such shortcuts ("Gefährliche Dateierweiterung
//! aus dem Web"). Removing the stream is what *Properties → Unblock* does.

#[cfg(windows)]
mod imp {
    use std::fs;
    use std::path::{Path, PathBuf};

    /// Delete the `Zone.Identifier` stream of `path`. Returns true if there was one.
    pub fn remove_web_mark(path: &Path) -> bool {
        let mut stream = path.as_os_str().to_owned();
        stream.push(":Zone.Identifier");
        fs::remove_file(PathBuf::from(stream)).is_ok()
    }

    /// Folders where Windows keeps the user's shortcuts to this program.
    fn shortcut_folders() -> Vec<PathBuf> {
        let mut folders = Vec::new();
        if let Some(appdata) = std::env::var_os("APPDATA").map(PathBuf::from) {
            folders.push(appdata.join(r"Microsoft\Internet Explorer\Quick Launch\User Pinned\TaskBar"));
            folders.push(appdata.join(r"Microsoft\Internet Explorer\Quick Launch"));
            folders.push(appdata.join(r"Microsoft\Windows\Start Menu\Programs"));
        }
        if let Some(desktop) = dirs::desktop_dir() {
            folders.push(desktop);
        }
        folders
    }

    /// Our own shortcuts in `folder` ("FileSorter.lnk", "FileSorter (2).lnk", …).
    pub fn own_shortcuts(folder: &Path) -> Vec<PathBuf> {
        let Ok(entries) = fs::read_dir(folder) else { return Vec::new() };
        entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                let name = p.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
                name.starts_with("filesorter") && name.ends_with(".lnk")
            })
            .collect()
    }

    pub fn unblock_self() {
        let mut targets: Vec<PathBuf> = std::env::current_exe().into_iter().collect();
        for folder in shortcut_folders() {
            targets.extend(own_shortcuts(&folder));
        }
        for path in targets {
            if remove_web_mark(&path) {
                log::info!("Removed the \"downloaded from the web\" block from {}", path.display());
            }
        }
    }
}

/// Unblock the running exe and its shortcuts (Windows only; nothing to do elsewhere).
pub fn unblock_self() {
    #[cfg(windows)]
    imp::unblock_self();
}

#[cfg(all(test, windows))]
mod tests {
    use super::imp::*;
    use std::fs;
    use std::path::PathBuf;

    fn mark(path: &std::path::Path) -> PathBuf {
        let mut stream = path.as_os_str().to_owned();
        stream.push(":Zone.Identifier");
        PathBuf::from(stream)
    }

    #[test]
    fn removes_mark_but_keeps_file() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("FileSorter.lnk");
        fs::write(&file, b"shortcut").unwrap();
        fs::write(mark(&file), "[ZoneTransfer]\r\nZoneId=3\r\n").unwrap();
        assert!(mark(&file).exists());

        assert!(remove_web_mark(&file));
        assert!(!mark(&file).exists());
        assert_eq!(fs::read(&file).unwrap(), b"shortcut");
        assert!(!remove_web_mark(&file), "nothing left to remove");
    }

    #[test]
    fn only_our_shortcuts_are_picked() {
        let tmp = tempfile::tempdir().unwrap();
        for name in ["FileSorter.lnk", "FileSorter (2).lnk", "Firefox.lnk", "FileSorter.exe"] {
            fs::write(tmp.path().join(name), b"x").unwrap();
        }
        let mut names: Vec<String> =
            own_shortcuts(tmp.path()).iter().map(|p| p.file_name().unwrap().to_string_lossy().into_owned()).collect();
        names.sort();
        assert_eq!(names, vec!["FileSorter (2).lnk", "FileSorter.lnk"]);
    }
}
