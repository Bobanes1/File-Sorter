//! The study folder in OneDrive, Dropbox, Google Drive or another synced folder, so it is
//! the same on every device.
//!
//! Three ways, chosen in the settings:
//! - [`CloudMode::Off`]: the study folder is only on this computer.
//! - [`CloudMode::Move`]: the study folder itself lives inside the cloud folder (moved there
//!   once by [`relocate`]); the cloud app keeps it in sync in both directions.
//! - [`CloudMode::Mirror`]: the study folder stays where it is and [`mirror`] keeps a copy
//!   of it inside the cloud folder (one way: this computer → cloud).
//!
//! With [`shared`], modules, keywords and file types travel along, so every computer sorts
//! the same way.

pub mod detect;
pub mod mirror;
pub mod relocate;
pub mod shared;
pub mod worker;

use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use serde::{Deserialize, Serialize};

use crate::config::{Config, expand_user};
#[allow(unused_imports)]
pub use detect::{CloudRoot, Provider};

/// How the study folder is connected to a cloud folder.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CloudMode {
    /// Only on this computer.
    #[default]
    Off,
    /// The study folder lives inside the cloud folder; the cloud app syncs it both ways.
    Move,
    /// The study folder stays here; File Sorter keeps a copy inside the cloud folder.
    Mirror,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct CloudSettings {
    pub mode: CloudMode,
    /// The cloud folder, e.g. `C:\Users\Anna\OneDrive`; empty while none is chosen.
    pub folder: String,
    /// Which service `folder` belongs to (for its name and icon).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<Provider>,
    /// Share modules, keywords and file types with other computers through the cloud.
    pub share_settings: bool,
    /// Where the study folder was before it was moved into the cloud (to offer moving it back).
    pub local_root: String,
    /// `updated_at` of the shared settings this computer last read or wrote.
    pub shared_seen: i64,
}

/// The study folder's name inside the cloud folder: the current study folder's name
/// ("Studium", "University", …), or "Studium" if it has none.
pub fn study_folder_name(destination_root: &Path) -> String {
    destination_root
        .file_name()
        .map(|n| n.to_string_lossy().trim().to_string())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| "Studium".to_string())
}

/// Where the study folder is (or goes) inside the cloud folder: the study folder itself
/// when it was moved there, the copy's folder in mirror mode, `None` when off.
pub fn cloud_study_root(config: &Config) -> Option<PathBuf> {
    let root = expand_user(config.destination_root.trim());
    match config.cloud.mode {
        CloudMode::Off => None,
        CloudMode::Move => Some(root),
        CloudMode::Mirror => {
            let folder = config.cloud.folder.trim();
            (!folder.is_empty()).then(|| expand_user(folder).join(study_folder_name(&root)))
        }
    }
}

/// Files that are never moved or copied by the cloud features: folder settings and
/// thumbnails the system makes by itself, Office lock files, downloads in progress and
/// File Sorter's own temporary files.
pub fn is_junk(name: &str) -> bool {
    let lower = name.to_lowercase();
    matches!(lower.as_str(), "desktop.ini" | "thumbs.db" | ".ds_store" | "icon\r")
        || lower.starts_with("~$")
        || lower.starts_with(".~lock.")
        || lower.ends_with(TEMP_SUFFIX)
        || crate::sorter::TEMP_SUFFIXES.iter().any(|s| lower.ends_with(s))
}

/// Suffix of File Sorter's own temporary files while copying into the cloud folder.
pub const TEMP_SUFFIX: &str = ".filesorter-tmp";

/// This computer's name ("DESKTOP-4FJ2K"), for conflict copies and the shared settings.
pub fn computer_name() -> String {
    let from_env = std::env::var("COMPUTERNAME").or_else(|_| std::env::var("HOSTNAME")).ok();
    let from_file = || fs::read_to_string("/etc/hostname").ok();
    from_env
        .or_else(from_file)
        .map(|n| n.trim().to_string())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| "PC".to_string())
}

/// Size and modification time of a file, to notice changes without reading it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Stamp {
    pub size: u64,
    /// Nanoseconds since 1970.
    pub modified: u64,
}

impl Stamp {
    /// The same size, and modification times at most `slack_secs` apart (some cloud
    /// folders round or shift times a little after uploading).
    pub fn roughly_eq(&self, other: &Stamp, slack_secs: u64) -> bool {
        self.size == other.size && self.modified.abs_diff(other.modified) <= slack_secs * 1_000_000_000
    }
}

pub fn stamp(path: &Path) -> Option<Stamp> {
    let meta = fs::metadata(path).ok()?;
    if !meta.is_file() {
        return None;
    }
    let modified = meta.modified().ok()?.duration_since(UNIX_EPOCH).ok()?.as_nanos() as u64;
    Some(Stamp { size: meta.len(), modified })
}

/// A 64-bit FNV-1a hash of the file's content (stable across versions and computers).
pub fn content_hash(path: &Path) -> io::Result<u64> {
    let mut file = fs::File::open(path)?;
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            return Ok(hash);
        }
        for byte in &buf[..n] {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
}

/// A path for comparing: absolute, without `.` and `..`, its deepest existing folder resolved
/// (so `RUNNER~1` and `runneradmin`, or a symlinked `/var`, compare equal also for files
/// that don't exist yet), without `\\?\`, lower-case on Windows.
fn comparable(path: &Path) -> PathBuf {
    let full = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    let mut clean = PathBuf::new();
    for part in full.components() {
        match part {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                clean.pop();
            }
            other => clean.push(other),
        }
    }
    let mut missing = Vec::new();
    let mut existing = clean.as_path();
    let resolved = loop {
        if let Ok(real) = existing.canonicalize() {
            break missing.iter().rev().fold(real, |path: PathBuf, name| path.join(name));
        }
        match (existing.parent(), existing.file_name()) {
            (Some(parent), Some(name)) => {
                missing.push(name.to_os_string());
                existing = parent;
            }
            _ => break clean.clone(),
        }
    };
    let text = resolved.to_string_lossy();
    let text = text.strip_prefix(r"\\?\").unwrap_or(&text);
    PathBuf::from(if cfg!(windows) { text.to_lowercase() } else { text.to_string() })
}

/// True if `path` is `folder` itself or somewhere inside it.
pub fn is_inside(path: &Path, folder: &Path) -> bool {
    comparable(path).starts_with(comparable(folder))
}

/// True for a file of OneDrive or another cloud app that is only in the cloud: reading it
/// makes the cloud app download it first (Windows only; elsewhere always false).
pub fn is_placeholder(path: &Path) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        // FILE_ATTRIBUTE_OFFLINE, _RECALL_ON_OPEN, _RECALL_ON_DATA_ACCESS
        const CLOUD_ONLY: u32 = 0x0000_1000 | 0x0004_0000 | 0x0040_0000;
        fs::symlink_metadata(path).is_ok_and(|m| m.file_attributes() & CLOUD_ONLY != 0)
    }
    #[cfg(not(windows))]
    {
        let _ = path;
        false
    }
}

/// Write JSON to `file` through a temporary file, so readers never see half of it.
pub fn save_json<T: Serialize>(value: &T, file: &Path) -> io::Result<()> {
    if let Some(dir) = file.parent() {
        fs::create_dir_all(dir)?;
    }
    let name = file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let tmp = file.with_file_name(format!("{name}{TEMP_SUFFIX}"));
    let mut out = io::BufWriter::new(fs::File::create(&tmp)?);
    // Written as a stream: large lists (the search index) needn't be in memory twice.
    serde_json::to_writer_pretty(&mut out, value).map_err(io::Error::other)?;
    let out = out.into_inner().map_err(|e| e.into_error())?;
    // On the disk before the rename: after a power cut the rename may be kept and data that
    // was still in the cache lost, leaving an empty file that the cloud app then uploads.
    out.sync_all()?;
    drop(out);
    fs::rename(&tmp, file)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn junk_and_names() {
        assert!(is_junk("desktop.ini") && is_junk("Thumbs.db") && is_junk(".DS_Store"));
        assert!(is_junk("~$Notizen.docx") && is_junk("vl3.pdf.part") && is_junk("x.pdf.filesorter-tmp"));
        assert!(!is_junk("Skript.pdf") && !is_junk("notes.md"));
        assert_eq!(study_folder_name(Path::new("/home/a/Studium")), "Studium");
        assert_eq!(study_folder_name(Path::new("/")), "Studium");
        assert!(!computer_name().is_empty());
    }

    /// The same folder spelled two ways (Windows: `RUNNER~1` and `runneradmin`; here a symlink).
    #[cfg(unix)]
    #[test]
    fn inside_through_another_spelling() {
        let tmp = tempfile::tempdir().unwrap();
        let real = tmp.path().join("real");
        fs::create_dir_all(&real).unwrap();
        let link = tmp.path().join("link");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        assert!(is_inside(&link.join("new").join("x.pdf"), &real));
        assert!(is_inside(&real.join("new"), &link));
        assert!(!is_inside(&tmp.path().join("other"), &link));
    }

    #[test]
    fn inside_and_stamps() {
        let tmp = tempfile::tempdir().unwrap();
        let a = tmp.path().join("a");
        fs::create_dir_all(a.join("b")).unwrap();
        assert!(is_inside(&a.join("b"), &a) && is_inside(&a, &a));
        assert!(is_inside(&a.join("b").join("..").join("b").join("new.pdf"), &a));
        assert!(!is_inside(&tmp.path().join("ab"), &a));
        assert!(is_inside(&a.join("not").join("there.pdf"), &a), "paths that don't exist yet");
        assert!(!is_inside(&a, &a.join("b")));
        let f = a.join("f.txt");
        fs::write(&f, "hello").unwrap();
        let s = stamp(&f).unwrap();
        assert_eq!(s.size, 5);
        assert!(s.roughly_eq(&Stamp { modified: s.modified + 1_000_000_000, ..s }, 2));
        assert!(!s.roughly_eq(&Stamp { size: 6, ..s }, 2));
        assert_eq!(content_hash(&f).unwrap(), 0xa430_d846_80aa_bd0b, "FNV-1a of \"hello\"");
        assert!(stamp(&a).is_none(), "folders have no stamp");
    }
}
