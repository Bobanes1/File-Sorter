//! Unpack a sorted ZIP file next to itself, safely.
//!
//! Everything goes into a new folder named like the ZIP file ("Blatt3.zip" → "Blatt3/"),
//! so nothing that was there before is ever touched. Entries that would land outside it
//! ("../x", absolute paths) are skipped, and very large or very long archives (often
//! made to fill the disk) aren't unpacked at all. If unpacking fails halfway, the new
//! folder is removed again, so there is never half an archive.

use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub const MAX_ENTRIES: usize = 5000;
pub const MAX_TOTAL_BYTES: u64 = 2 * 1024 * 1024 * 1024;

/// What was unpacked, to undo it later only if nobody changed it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Extracted {
    pub folder: PathBuf,
    /// Each file, relative to `folder`, with its size.
    pub files: Vec<(PathBuf, u64)>,
}

#[derive(Debug)]
pub enum UnzipError {
    TooBig,
    Encrypted,
    Failed(io::Error),
}

impl std::fmt::Display for UnzipError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            UnzipError::TooBig => write!(f, "the archive is too big to unpack"),
            UnzipError::Encrypted => write!(f, "the archive is password-protected"),
            UnzipError::Failed(e) => write!(f, "{e}"),
        }
    }
}

impl From<io::Error> for UnzipError {
    fn from(e: io::Error) -> Self {
        UnzipError::Failed(e)
    }
}

impl From<zip::result::ZipError> for UnzipError {
    fn from(e: zip::result::ZipError) -> Self {
        UnzipError::Failed(io::Error::other(e))
    }
}

/// Make a new, empty folder `name` in `parent` ("name (1)" etc. if taken).
fn new_folder(parent: &Path, name: &str) -> io::Result<PathBuf> {
    for n in 0.. {
        let candidate = if n == 0 { parent.join(name) } else { parent.join(format!("{name} ({n})")) };
        match fs::create_dir(&candidate) {
            Ok(()) => return Ok(candidate),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(_) if fs::symlink_metadata(&candidate).is_ok() => continue,
            Err(e) => return Err(e),
        }
    }
    unreachable!("an unused name always exists")
}

fn is_junk(path: &Path) -> bool {
    path.components().any(|c| {
        let name = c.as_os_str().to_string_lossy().to_lowercase();
        name == "__macosx" || name == ".ds_store" || name == "thumbs.db" || name == "desktop.ini"
    })
}

/// Unpack `zip_path` into a new folder next to it. Returns what was unpacked.
pub fn extract_next_to(zip_path: &Path) -> Result<Extracted, UnzipError> {
    let parent = zip_path.parent().ok_or_else(|| io::Error::other("no folder"))?;
    let stem = zip_path.file_stem().map(|s| s.to_string_lossy().trim().to_string()).unwrap_or_default();
    let stem = if stem.is_empty() { "ZIP".to_string() } else { stem };
    let mut archive = zip::ZipArchive::new(fs::File::open(zip_path)?)?;
    if archive.len() > MAX_ENTRIES {
        return Err(UnzipError::TooBig);
    }
    // What goes where: safe relative paths only, the declared sizes within the limit.
    let mut entries: Vec<(usize, PathBuf, bool)> = Vec::new();
    let mut total: u64 = 0;
    for i in 0..archive.len() {
        let entry = archive.by_index_raw(i)?;
        if entry.encrypted() {
            return Err(UnzipError::Encrypted);
        }
        let Some(path) = entry.enclosed_name() else { continue };
        if entry.is_symlink() || is_junk(&path) || path.as_os_str().is_empty() {
            continue;
        }
        total = total.saturating_add(entry.size());
        if total > MAX_TOTAL_BYTES {
            return Err(UnzipError::TooBig);
        }
        entries.push((i, path, entry.is_dir()));
    }
    // "Blatt3.zip" holding just "Blatt3/…": don't make "Blatt3/Blatt3/…".
    let first = |p: &Path| p.components().next().map(|c| c.as_os_str().to_os_string());
    let common = entries.first().and_then(|(_, p, _)| first(p));
    let strip = common.filter(|c| {
        entries.iter().all(|(_, p, dir)| first(p).as_ref() == Some(c) && (p.components().count() > 1 || *dir))
    });

    let folder = new_folder(parent, &stem)?;
    match unpack(&mut archive, &entries, strip.as_deref(), &folder) {
        Ok(files) => Ok(Extracted { folder, files }),
        Err(e) => {
            // Only File Sorter's own, new folder: remove it rather than leave half an archive.
            let _ = fs::remove_dir_all(&folder);
            Err(e)
        }
    }
}

fn unpack(
    archive: &mut zip::ZipArchive<fs::File>,
    entries: &[(usize, PathBuf, bool)],
    strip: Option<&std::ffi::OsStr>,
    folder: &Path,
) -> Result<Vec<(PathBuf, u64)>, UnzipError> {
    let mut files = Vec::new();
    let mut written: u64 = 0;
    for (index, path, dir) in entries {
        let relative = match strip {
            Some(prefix) => path.strip_prefix(prefix).unwrap_or(path).to_path_buf(),
            None => path.clone(),
        };
        if relative.as_os_str().is_empty() {
            continue;
        }
        let target = folder.join(&relative);
        if *dir {
            fs::create_dir_all(&target)?;
            continue;
        }
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut entry = archive.by_index(*index)?;
        // The folder is new, so the file can't exist yet; `create_new` makes sure.
        let mut out = fs::OpenOptions::new().write(true).create_new(true).open(&target)?;
        // The declared size can lie: stop at the limit whatever the archive claims.
        let limit = MAX_TOTAL_BYTES - written;
        let copied = io::copy(&mut entry.by_ref().take(limit + 1), &mut out)?;
        written += copied;
        if written > MAX_TOTAL_BYTES {
            return Err(UnzipError::TooBig);
        }
        out.flush()?;
        files.push((relative, copied));
    }
    files.sort();
    Ok(files)
}

/// The files now in `extracted.folder` with their sizes, in the same form as recorded.
fn current_files(folder: &Path) -> Vec<(PathBuf, u64)> {
    fn walk(root: &Path, dir: &Path, out: &mut Vec<(PathBuf, u64)>) {
        let Ok(entries) = fs::read_dir(dir) else { return };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(meta) = fs::symlink_metadata(&path) else { continue };
            if meta.is_dir() {
                walk(root, &path, out);
            } else if let Ok(relative) = path.strip_prefix(root) {
                out.push((relative.to_path_buf(), meta.len()));
            }
        }
    }
    let mut files = Vec::new();
    walk(folder, folder, &mut files);
    files.sort();
    files
}

/// True if the unpacked folder still holds exactly what was unpacked (nothing added,
/// removed or changed in size), so removing it loses nothing of the user's.
pub fn unchanged(extracted: &Extracted) -> bool {
    let mut recorded = extracted.files.clone();
    recorded.sort();
    current_files(&extracted.folder) == recorded
}

#[cfg(test)]
mod tests {
    use super::*;
    use zip::write::SimpleFileOptions;

    fn make_zip(path: &Path, entries: &[(&str, &[u8])]) {
        let mut zip = zip::ZipWriter::new(fs::File::create(path).unwrap());
        for (name, data) in entries {
            if name.ends_with('/') {
                zip.add_directory(*name, SimpleFileOptions::default()).unwrap();
            } else {
                zip.start_file(*name, SimpleFileOptions::default()).unwrap();
                zip.write_all(data).unwrap();
            }
        }
        zip.finish().unwrap();
    }

    #[test]
    fn unpacks_into_a_new_folder_next_to_it() {
        let tmp = tempfile::tempdir().unwrap();
        let zip = tmp.path().join("Blatt3.zip");
        make_zip(&zip, &[("aufgabe1.py", b"print(1)"), ("daten/werte.csv", b"1,2"), ("__MACOSX/._x", b"junk")]);
        fs::create_dir(tmp.path().join("Blatt3")).unwrap();
        fs::write(tmp.path().join("Blatt3").join("aufgabe1.py"), "mine").unwrap();
        let done = extract_next_to(&zip).unwrap();
        assert_eq!(done.folder, tmp.path().join("Blatt3 (1)"), "the existing folder is not touched");
        assert_eq!(fs::read_to_string(tmp.path().join("Blatt3").join("aufgabe1.py")).unwrap(), "mine");
        assert_eq!(fs::read_to_string(done.folder.join("daten").join("werte.csv")).unwrap(), "1,2");
        assert!(!done.folder.join("__MACOSX").exists());
        assert_eq!(done.files.len(), 2);
        assert!(unchanged(&done));
        fs::write(done.folder.join("notizen.txt"), "x").unwrap();
        assert!(!unchanged(&done), "something was added");
    }

    #[test]
    fn a_single_top_folder_is_not_doubled() {
        let tmp = tempfile::tempdir().unwrap();
        let zip = tmp.path().join("Projekt.zip");
        make_zip(&zip, &[("Projekt/", b""), ("Projekt/src/main.py", b"x"), ("Projekt/README.md", b"y")]);
        let done = extract_next_to(&zip).unwrap();
        assert!(done.folder.join("src").join("main.py").is_file());
        assert!(done.folder.join("README.md").is_file());
    }

    #[test]
    fn entries_outside_the_folder_are_skipped() {
        let tmp = tempfile::tempdir().unwrap();
        let inner = tmp.path().join("inner");
        fs::create_dir(&inner).unwrap();
        let zip = inner.join("evil.zip");
        make_zip(&zip, &[("../escaped.txt", b"x"), ("/abs.txt", b"y"), ("ok.txt", b"z")]);
        let done = extract_next_to(&zip).unwrap();
        assert!(!tmp.path().join("escaped.txt").exists() && !Path::new("/abs.txt").exists());
        // "/abs.txt" lands inside the new folder, "../escaped.txt" nowhere.
        let names: Vec<PathBuf> = done.files.iter().map(|(p, _)| p.clone()).collect();
        assert_eq!(names, [PathBuf::from("abs.txt"), PathBuf::from("ok.txt")]);
    }

    #[test]
    fn too_many_entries_and_broken_archives_unpack_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let zip = tmp.path().join("many.zip");
        let names: Vec<String> = (0..=MAX_ENTRIES).map(|i| format!("f{i}.txt")).collect();
        let entries: Vec<(&str, &[u8])> = names.iter().map(|n| (n.as_str(), &b""[..])).collect();
        make_zip(&zip, &entries);
        assert!(matches!(extract_next_to(&zip), Err(UnzipError::TooBig)));
        assert!(!tmp.path().join("many").exists());
        let broken = tmp.path().join("broken.zip");
        fs::write(&broken, "not a zip").unwrap();
        assert!(extract_next_to(&broken).is_err());
        assert!(!tmp.path().join("broken").exists());
    }
}
