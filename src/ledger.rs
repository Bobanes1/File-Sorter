//! Copy mode: remembers which originals were already copied.
//!
//! In copy mode the original stays in the watched folder, so it would be found again
//! by "Sort existing files" or the next start. As long as the original is unchanged,
//! it is not copied a second time – even if its copy was annotated, renamed or
//! deleted in the meantime.

use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use serde::{Deserialize, Serialize};

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

impl CopyLedger {
    /// The list saved in `file`; empty if there is none yet (or it can't be read).
    pub fn load(file: &Path) -> CopyLedger {
        fs::read_to_string(file).ok().and_then(|text| serde_json::from_str(&text).ok()).unwrap_or_default()
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
        if let Some(dir) = file.parent() {
            fs::create_dir_all(dir)?;
        }
        let tmp = file.with_extension("tmp");
        fs::write(&tmp, serde_json::to_string_pretty(self).map_err(io::Error::other)?)?;
        fs::rename(&tmp, file)
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
    fn unreadable_list_starts_empty() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("copied.json");
        fs::write(&file, "not json").unwrap();
        assert!(CopyLedger::load(&file).copied.is_empty());
    }
}
