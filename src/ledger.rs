//! File Sorter's memory of individual files.
//!
//! - Copy mode: which originals were already copied. The original stays in the watched
//!   folder, so it would be found again by "Sort existing files" or the next start. As
//!   long as it is unchanged, it is not copied a second time – even if its copy was
//!   annotated, renamed or deleted in the meantime.
//! - Files to leave alone: sorting was undone for them, so they stay where they are.

use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use serde::{Deserialize, Serialize};

/// Copy mode's list, in File Sorter's settings folder.
pub const COPIED_LIST: &str = "copied.json";
/// Files whose sorting was undone, in File Sorter's settings folder.
pub const KEPT_LIST: &str = "kept.json";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Entry {
    size: u64,
    /// Modification time of the original, in nanoseconds since 1970.
    modified: u64,
    course: String,
    copy: PathBuf,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct CopyLedger {
    #[serde(default)]
    copied: HashMap<String, Entry>,
}

/// An earlier copy of a file.
#[derive(Debug, Clone, PartialEq)]
pub struct EarlierCopy {
    pub course: String,
    pub copy: PathBuf,
}

/// The same file always gets the same key, however its path was spelled.
fn key(path: &Path) -> String {
    let full = match (path.parent().and_then(|p| p.canonicalize().ok()), path.file_name()) {
        (Some(dir), Some(name)) => dir.join(name),
        _ => path.to_path_buf(),
    };
    let text = full.to_string_lossy();
    let text = text.strip_prefix(r"\\?\").unwrap_or(&text);
    if cfg!(windows) { text.to_lowercase() } else { text.to_string() }
}

fn stamp(path: &Path) -> Option<(u64, u64)> {
    let meta = fs::metadata(path).ok()?;
    let modified = meta.modified().ok()?.duration_since(UNIX_EPOCH).ok()?.as_nanos() as u64;
    Some((meta.len(), modified))
}

/// Size, modification and (where the system records it) creation time of a file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Identity {
    size: u64,
    modified: u64,
    #[serde(default)]
    created: Option<u64>,
}

fn identity(path: &Path) -> Option<Identity> {
    let meta = fs::metadata(path).ok()?;
    let nanos = |t: std::io::Result<std::time::SystemTime>| {
        t.ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_nanos() as u64)
    };
    Some(Identity { size: meta.len(), modified: nanos(meta.modified())?, created: nanos(meta.created()) })
}

fn load_json<T: Default + for<'de> Deserialize<'de>>(file: &Path) -> T {
    fs::read_to_string(file).ok().and_then(|text| serde_json::from_str(&text).ok()).unwrap_or_default()
}

fn save_json<T: Serialize>(value: &T, file: &Path) -> io::Result<()> {
    if let Some(dir) = file.parent() {
        fs::create_dir_all(dir)?;
    }
    let tmp = file.with_extension("tmp");
    fs::write(&tmp, serde_json::to_string_pretty(value).map_err(io::Error::other)?)?;
    fs::rename(&tmp, file)
}

/// Files to leave where they are, because the user undid their sorting.
///
/// A file counts as the same while it keeps its path and either its creation time (so
/// editing it doesn't matter) or its size and modification time. A new download that
/// happens to get the same name is a different file and is sorted as usual.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct KeptFiles {
    #[serde(default)]
    kept: HashMap<String, Identity>,
}

impl KeptFiles {
    pub fn load(file: &Path) -> KeptFiles {
        load_json(file)
    }

    pub fn contains(&self, path: &Path) -> bool {
        let (Some(saved), Some(now)) = (self.kept.get(&key(path)), identity(path)) else { return false };
        (saved.created.is_some() && saved.created == now.created)
            || (saved.size == now.size && saved.modified == now.modified)
    }

    pub fn keep(&mut self, path: &Path) {
        if let Some(id) = identity(path) {
            self.kept.insert(key(path), id);
        }
    }

    /// Returns whether the file was on the list.
    pub fn forget(&mut self, path: &Path) -> bool {
        self.kept.remove(&key(path)).is_some()
    }

    /// Save to `file`, forgetting files that no longer exist.
    pub fn save(&mut self, file: &Path) -> io::Result<()> {
        self.kept.retain(|path, _| Path::new(path).exists());
        save_json(self, file)
    }
}

impl CopyLedger {
    /// The list saved in `file`; empty if there is none yet (or it can't be read).
    pub fn load(file: &Path) -> CopyLedger {
        load_json(file)
    }

    /// Returns whether the file was on the list.
    pub fn forget(&mut self, original: &Path) -> bool {
        self.copied.remove(&key(original)).is_some()
    }

    /// Where `original` was copied to before, if it hasn't changed since.
    pub fn earlier_copy(&self, original: &Path) -> Option<EarlierCopy> {
        let entry = self.copied.get(&key(original))?;
        (stamp(original) == Some((entry.size, entry.modified)))
            .then(|| EarlierCopy { course: entry.course.clone(), copy: entry.copy.clone() })
    }

    pub fn record(&mut self, original: &Path, course: &str, copy: &Path) {
        if let Some((size, modified)) = stamp(original) {
            let entry = Entry { size, modified, course: course.to_string(), copy: copy.to_path_buf() };
            self.copied.insert(key(original), entry);
        }
    }

    /// Save to `file`, forgetting originals that no longer exist.
    pub fn save(&mut self, file: &Path) -> io::Result<()> {
        self.copied.retain(|original, _| Path::new(original).exists());
        save_json(self, file)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remembers_unchanged_originals_only() {
        let tmp = tempfile::tempdir().unwrap();
        let original = tmp.path().join("slides.pdf");
        let copy = tmp.path().join("Module").join("slides.pdf");
        fs::write(&original, "v1").unwrap();
        fs::create_dir_all(copy.parent().unwrap()).unwrap();
        let file = tmp.path().join("copied.json");

        let mut ledger = CopyLedger::load(&file);
        assert_eq!(ledger.earlier_copy(&original), None);
        ledger.record(&original, "Module", &copy);
        ledger.save(&file).unwrap();

        // Spelled differently (with "..") it is still the same file.
        let other_spelling = tmp.path().join("Module").join("..").join("slides.pdf");
        let earlier = CopyLedger::load(&file).earlier_copy(&other_spelling).unwrap();
        assert_eq!(earlier, EarlierCopy { course: "Module".into(), copy: copy.clone() });

        // A changed original (new version downloaded under the same name) is copied again.
        fs::write(&original, "version 2").unwrap();
        assert_eq!(CopyLedger::load(&file).earlier_copy(&original), None);

        // Deleted originals are forgotten when saving.
        fs::remove_file(&original).unwrap();
        let mut ledger = CopyLedger::load(&file);
        ledger.save(&file).unwrap();
        assert!(ledger.copied.is_empty());
    }

    #[test]
    fn kept_files_survive_edits_but_not_a_new_download() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("kept.json");
        let doc = tmp.path().join("notes.pdf");
        fs::write(&doc, "v1").unwrap();
        let mut kept = KeptFiles::load(&file);
        kept.keep(&doc);
        kept.save(&file).unwrap();
        assert!(KeptFiles::load(&file).contains(&doc));

        // Edited in place: still the same file.
        std::thread::sleep(std::time::Duration::from_millis(20));
        fs::write(&doc, "v1 with notes").unwrap();
        let created_known = fs::metadata(&doc).and_then(|m| m.created()).is_ok();
        assert_eq!(KeptFiles::load(&file).contains(&doc), created_known);

        // Deleted and downloaded again later: a different file. (Windows gives a file that is
        // re-created within 15 s the old creation time, so that case can't be checked there.)
        fs::remove_file(&doc).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        fs::write(&doc, "another file").unwrap();
        if created_known && !cfg!(windows) {
            assert!(!KeptFiles::load(&file).contains(&doc));
        }

        let mut kept = KeptFiles::load(&file);
        assert!(kept.forget(&doc));
        assert!(!kept.contains(&doc));
    }

    #[test]
    fn unreadable_list_starts_empty() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("copied.json");
        fs::write(&file, "not json").unwrap();
        assert!(CopyLedger::load(&file).copied.is_empty());
    }
}
