//! Match a single file to a module and move it into that module's folder.

use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

use crate::config::Config;
use crate::extract::extract_text;
use crate::matcher::{Reason, match_file};

/// Partial downloads written by browsers before the file is complete.
pub const TEMP_SUFFIXES: &[&str] = &[".crdownload", ".part", ".partial", ".download", ".tmp", ".temp", ".opdownload"];

/// How long to keep retrying a file that another program (browser, PDF preview,
/// virus scanner) still has open, before giving up for now.
const IN_USE_RETRY: Duration = Duration::from_secs(8);

/// What happened to a file.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    /// Moved into the module folder.
    Moved { course: String, destination: PathBuf, reason: Reason },
    /// The very same file was already in the module folder, so the download was removed.
    DuplicateRemoved { course: String, existing: PathBuf },
    /// The file stays where it is.
    Left(LeftReason),
}

#[derive(Debug, Clone, PartialEq)]
pub enum LeftReason {
    Gone,
    NotDocument,
    NoMatch,
    AlreadyInFolder {
        course: String,
    },
    /// Another program still has the file open; it will be tried again.
    InUse {
        course: String,
    },
    Error(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct SortResult {
    pub source: PathBuf,
    pub outcome: Outcome,
}

impl SortResult {
    fn left(source: &Path, reason: LeftReason) -> Self {
        SortResult { source: source.to_path_buf(), outcome: Outcome::Left(reason) }
    }

    pub fn file_name(&self) -> String {
        file_name(&self.source)
    }

    /// True when the file is gone from the watched folder (moved, or removed as a duplicate).
    pub fn moved(&self) -> bool {
        !matches!(self.outcome, Outcome::Left(_))
    }

    #[cfg(test)]
    pub fn course(&self) -> Option<&str> {
        match &self.outcome {
            Outcome::Moved { course, .. } | Outcome::DuplicateRemoved { course, .. } => Some(course),
            Outcome::Left(LeftReason::AlreadyInFolder { course } | LeftReason::InUse { course }) => Some(course),
            Outcome::Left(_) => None,
        }
    }

    #[cfg(test)]
    pub fn destination(&self) -> Option<&Path> {
        match &self.outcome {
            Outcome::Moved { destination, .. } => Some(destination),
            Outcome::DuplicateRemoved { existing, .. } => Some(existing),
            Outcome::Left(_) => None,
        }
    }

    /// English description, for the log file and the command line.
    pub fn describe(&self) -> String {
        let name = self.file_name();
        match &self.outcome {
            Outcome::Moved { course, reason, .. } => format!("Moved {name} → {course} ({})", reason.describe()),
            Outcome::DuplicateRemoved { course, .. } => {
                format!("Removed {name}: the same file is already in {course}")
            }
            Outcome::Left(reason) => {
                let why = match reason {
                    LeftReason::Gone => "file no longer exists".to_string(),
                    LeftReason::NotDocument => "not a course file type".to_string(),
                    LeftReason::NoMatch => "no matching module".to_string(),
                    LeftReason::AlreadyInFolder { .. } => "already in module folder".to_string(),
                    LeftReason::InUse { course } => {
                        format!("still open in another program, will retry (belongs to {course})")
                    }
                    LeftReason::Error(e) => format!("error: {e}"),
                };
                format!("Left {name} in place – {why}")
            }
        }
    }
}

pub fn file_name(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

/// Lower-cased extension including the dot (".pdf"), or "" if there is none.
fn suffix(path: &Path) -> String {
    path.extension().map(|e| format!(".{}", e.to_string_lossy().to_lowercase())).unwrap_or_default()
}

/// True for finished files with an extension the user wants sorted.
pub fn is_candidate(path: &Path, config: &Config) -> bool {
    let name = file_name(path);
    if name.starts_with('.') || name.starts_with("~$") {
        return false;
    }
    let ext = suffix(path);
    !TEMP_SUFFIXES.contains(&ext.as_str()) && config.extensions.contains(&ext)
}

/// folder/filename, or folder/"name (1).ext" etc. if that already exists.
pub fn unique_destination(folder: &Path, filename: &str) -> PathBuf {
    let target = folder.join(filename);
    if !target.exists() {
        return target;
    }
    let path = Path::new(filename);
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let ext = path.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    (1..)
        .map(|n| folder.join(format!("{stem} ({n}){ext}")))
        .find(|candidate| !candidate.exists())
        .expect("an unused name always exists")
}

/// True if the error means another program still has the file open.
fn in_use(e: &io::Error) -> bool {
    // Windows: ERROR_ACCESS_DENIED (5), ERROR_SHARING_VIOLATION (32), ERROR_LOCK_VIOLATION (33).
    e.kind() == io::ErrorKind::PermissionDenied || matches!(e.raw_os_error(), Some(5 | 32 | 33))
}

#[derive(Debug)]
pub enum MoveError {
    /// Another program has the file open; nothing was changed.
    InUse(io::Error),
    Failed(io::Error),
}

impl std::fmt::Display for MoveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MoveError::InUse(e) => write!(f, "file is in use by another program ({e})"),
            MoveError::Failed(e) => write!(f, "{e}"),
        }
    }
}

/// Run `op` until it succeeds, fails for another reason than "in use", or `patience` runs out.
fn retry_while_in_use<T>(patience: Duration, mut op: impl FnMut() -> io::Result<T>) -> io::Result<T> {
    let deadline = Instant::now() + patience;
    let mut pause = Duration::from_millis(100);
    loop {
        match op() {
            Err(e) if in_use(&e) && Instant::now() + pause < deadline => {
                thread::sleep(pause);
                pause = (pause * 2).min(Duration::from_secs(1));
            }
            result => return result,
        }
    }
}

/// Move a file so that it ends up in exactly one place.
///
/// Renames when possible. If another program still has the file open, waits up to
/// `patience` for it to let go. Across drives (Downloads on C:, study folder on D: or
/// OneDrive) it copies and then deletes the original; if the original can't be
/// deleted, the copy is removed again, so a file is never left in both places.
pub fn move_file_with_patience(from: &Path, to: &Path, patience: Duration) -> Result<(), MoveError> {
    let size = fs::metadata(from).map_err(MoveError::Failed)?.len();
    match retry_while_in_use(patience, || fs::rename(from, to)) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Err(MoveError::Failed(e)),
        Err(e) if in_use(&e) => return Err(MoveError::InUse(e)),
        Err(_) => {
            // Different drive (or a rename the file system refuses): copy, then delete.
            fs::copy(from, to).map_err(MoveError::Failed)?;
            if let Err(e) = retry_while_in_use(patience, || fs::remove_file(from)) {
                let _ = fs::remove_file(to);
                return Err(if in_use(&e) { MoveError::InUse(e) } else { MoveError::Failed(e) });
            }
        }
    }
    // Verify: exactly one copy, complete.
    let arrived = fs::metadata(to).map(|m| m.len() == size).unwrap_or(false);
    if from.exists() || !arrived {
        return Err(MoveError::Failed(io::Error::other("the file could not be moved completely")));
    }
    Ok(())
}

#[cfg(test)]
pub fn move_file(from: &Path, to: &Path) -> Result<(), MoveError> {
    move_file_with_patience(from, to, IN_USE_RETRY)
}

/// True if both files have exactly the same content.
pub fn same_content(a: &Path, b: &Path) -> bool {
    let (Ok(ma), Ok(mb)) = (fs::metadata(a), fs::metadata(b)) else { return false };
    if ma.len() != mb.len() {
        return false;
    }
    let (Ok(mut fa), Ok(mut fb)) = (fs::File::open(a), fs::File::open(b)) else { return false };
    let mut ba = vec![0u8; 64 * 1024];
    let mut bb = vec![0u8; 64 * 1024];
    loop {
        let na = match fa.read(&mut ba) {
            Ok(n) => n,
            Err(_) => return false,
        };
        if na == 0 {
            return true;
        }
        if fb.read_exact(&mut bb[..na]).is_err() || ba[..na] != bb[..na] {
            return false;
        }
    }
}

fn same_dir(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

pub fn sort_file(path: &Path, config: &Config) -> SortResult {
    sort_file_with_patience(path, config, IN_USE_RETRY)
}

pub fn sort_file_with_patience(path: &Path, config: &Config, patience: Duration) -> SortResult {
    if !is_candidate(path, config) && path.is_file() {
        return SortResult::left(path, LeftReason::NotDocument);
    }
    sort_any_file(path, config, patience)
}

/// Sort files and folders the user picked explicitly (right-click → "Sort with File Sorter").
///
/// A file is sorted whatever its type, since the user chose it; a folder stands for
/// the document files directly inside it.
pub fn sort_chosen(paths: &[PathBuf], config: &Config) -> Vec<SortResult> {
    let mut results = Vec::new();
    for path in paths {
        if path.is_dir() {
            let Ok(entries) = fs::read_dir(path) else { continue };
            let mut files: Vec<PathBuf> =
                entries.flatten().map(|e| e.path()).filter(|p| p.is_file() && is_candidate(p, config)).collect();
            files.sort();
            results.extend(files.iter().map(|f| sort_file(f, config)));
        } else {
            let name = file_name(path);
            let temporary = TEMP_SUFFIXES.contains(&suffix(path).as_str()) || name.starts_with("~$");
            results.push(if temporary {
                SortResult::left(path, LeftReason::NotDocument)
            } else {
                sort_any_file(path, config, IN_USE_RETRY)
            });
        }
    }
    results
}

fn sort_any_file(path: &Path, config: &Config, patience: Duration) -> SortResult {
    if !path.is_file() {
        return SortResult::left(path, LeftReason::Gone);
    }
    let Some(m) = match_file(path, &config.courses, config.read_file_contents, extract_text) else {
        return SortResult::left(path, LeftReason::NoMatch);
    };
    let course = &config.courses[m.course];
    let label = course.label();
    let folder = config.course_folder(course);
    if let Some(parent) = path.parent()
        && same_dir(&folder, parent)
    {
        return SortResult::left(path, LeftReason::AlreadyInFolder { course: label });
    }
    if let Err(e) = fs::create_dir_all(&folder) {
        return SortResult::left(path, LeftReason::Error(format!("cannot create {}: {e}", folder.display())));
    }

    // The very same file is already there (downloaded twice, or copied in again):
    // remove the download instead of keeping a second copy as "name (1)".
    let existing = folder.join(file_name(path));
    if existing.is_file() && same_content(path, &existing) {
        return match retry_while_in_use(patience, || fs::remove_file(path)) {
            Ok(()) => SortResult {
                source: path.to_path_buf(),
                outcome: Outcome::DuplicateRemoved { course: label, existing },
            },
            Err(e) if in_use(&e) => SortResult::left(path, LeftReason::InUse { course: label }),
            Err(e) => SortResult::left(path, LeftReason::Error(e.to_string())),
        };
    }

    let target = unique_destination(&folder, &file_name(path));
    match move_file_with_patience(path, &target, patience) {
        Ok(()) => SortResult {
            source: path.to_path_buf(),
            outcome: Outcome::Moved { course: label, destination: target, reason: m.reason },
        },
        Err(MoveError::InUse(_)) => SortResult::left(path, LeftReason::InUse { course: label }),
        Err(MoveError::Failed(e)) => SortResult::left(path, LeftReason::Error(e.to_string())),
    }
}

/// Sort every file currently sitting in the watch folder.
pub fn sort_existing(config: &Config) -> Vec<SortResult> {
    let Ok(entries) = fs::read_dir(config.watch_path()) else { return Vec::new() };
    let mut files: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
    files.sort();
    files.into_iter().filter(|p| p.is_file() && is_candidate(p, config)).map(|p| sort_file(&p, config)).collect()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::config::Course;

    /// A minimal one-page PDF containing `text`.
    pub fn make_pdf(path: &Path, text: &str) {
        let safe = text.replace('\\', "\\\\").replace('(', "\\(").replace(')', "\\)");
        let stream = format!("BT /F1 24 Tf 72 720 Td ({safe}) Tj ET");
        let objects = [
            "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R \
             /Resources << /Font << /F1 5 0 R >> >> >>"
                .to_string(),
            format!("<< /Length {} >>\nstream\n{stream}\nendstream", stream.len()),
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_string(),
        ];
        let mut out = String::from("%PDF-1.4\n");
        let mut offsets = Vec::new();
        for (i, obj) in objects.iter().enumerate() {
            offsets.push(out.len());
            out += &format!("{} 0 obj\n{obj}\nendobj\n", i + 1);
        }
        let xref = out.len();
        out += &format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1);
        for off in offsets {
            out += &format!("{off:010} 00000 n \n");
        }
        out += &format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objects.len() + 1);
        fs::write(path, out).unwrap();
    }

    pub fn fixture(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join("fixtures").join(name)
    }

    pub fn test_config(root: &Path) -> Config {
        let watch = root.join("Downloads");
        fs::create_dir_all(&watch).unwrap();
        Config {
            watch_folder: watch.to_string_lossy().into(),
            destination_root: root.join("University").to_string_lossy().into(),
            courses: vec![
                Course::new("Programming Course", &["programming", "python", "prog1", "algorithm"]),
                Course::new("Linear Algebra", &["linear algebra", "matrix", "LA1"]),
                Course::new("Statistics", &["statistics", "probability", "stats"]),
            ],
            ..Config::default()
        }
    }

    #[test]
    fn moves_matching_file() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = test_config(tmp.path());
        let src = cfg.watch_path().join("prog1_slides.pdf");
        make_pdf(&src, "Week 1");
        let result = sort_file(&src, &cfg);
        assert!(result.moved(), "{}", result.describe());
        let dest = tmp.path().join("University").join("Programming Course").join("prog1_slides.pdf");
        assert_eq!(result.destination(), Some(dest.as_path()));
        assert!(dest.exists() && !src.exists());
    }

    #[test]
    fn sorts_by_content() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = test_config(tmp.path());
        let src = cfg.watch_path().join("lecture5.pdf");
        fs::copy(fixture("linear_algebra.pdf"), &src).unwrap();
        assert_eq!(sort_file(&src, &cfg).course(), Some("Linear Algebra"));
        let src = cfg.watch_path().join("week3.pptx");
        fs::copy(fixture("programming.pptx"), &src).unwrap();
        assert_eq!(sort_file(&src, &cfg).course(), Some("Programming Course"));
        let src = cfg.watch_path().join("sheet.docx");
        fs::copy(fixture("statistics.docx"), &src).unwrap();
        assert_eq!(sort_file(&src, &cfg).course(), Some("Statistics"));
    }

    #[test]
    fn unmatched_file_stays() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = test_config(tmp.path());
        let src = cfg.watch_path().join("holiday.pdf");
        make_pdf(&src, "Beach trip");
        let result = sort_file(&src, &cfg);
        assert!(!result.moved() && src.exists());
        assert_eq!(result.describe(), "Left holiday.pdf in place – no matching module");
    }

    #[test]
    fn name_collision_gets_number() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("a.pdf"), "x").unwrap();
        fs::write(tmp.path().join("a (1).pdf"), "x").unwrap();
        assert_eq!(file_name(&unique_destination(tmp.path(), "a.pdf")), "a (2).pdf");
        assert_eq!(file_name(&unique_destination(tmp.path(), "b.pdf")), "b.pdf");
    }

    #[test]
    fn duplicate_download_not_overwritten() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = test_config(tmp.path());
        let folder = tmp.path().join("University").join("Programming Course");
        fs::create_dir_all(&folder).unwrap();
        fs::write(folder.join("prog1.pdf"), "old").unwrap();
        let src = cfg.watch_path().join("prog1.pdf");
        make_pdf(&src, "new");
        let result = sort_file(&src, &cfg);
        assert_eq!(file_name(result.destination().unwrap()), "prog1 (1).pdf");
        assert_eq!(fs::read_to_string(folder.join("prog1.pdf")).unwrap(), "old");
    }

    #[test]
    fn temp_and_unwanted_files_ignored() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = test_config(tmp.path());
        let w = cfg.watch_path();
        for name in ["prog1.pdf.crdownload", "prog1.pdf.part", "~$prog1.docx", ".prog1.pdf", "prog1.exe", "prog1"] {
            assert!(!is_candidate(&w.join(name), &cfg), "{name}");
        }
        assert!(is_candidate(&w.join("prog1.PDF"), &cfg));
    }

    #[test]
    fn sort_existing_sorts_folder() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = test_config(tmp.path());
        make_pdf(&cfg.watch_path().join("stats_hw.pdf"), "x");
        make_pdf(&cfg.watch_path().join("random.pdf"), "nothing");
        let results = sort_existing(&cfg);
        let get = |n: &str| results.iter().find(|r| r.file_name() == n).unwrap();
        assert_eq!(get("stats_hw.pdf").course(), Some("Statistics"));
        assert!(!get("random.pdf").moved());
    }

    #[test]
    fn subject_folders_and_custom_folder() {
        let tmp = tempfile::tempdir().unwrap();
        let mut cfg = test_config(tmp.path());
        cfg.courses[1].subject = "Mathematik".into();
        cfg.courses[2].folder = tmp.path().join("Elsewhere").join("Stats").to_string_lossy().into();
        let a = cfg.watch_path().join("matrix_sheet.pdf");
        make_pdf(&a, "x");
        let r = sort_file(&a, &cfg);
        assert_eq!(r.course(), Some("Mathematik › Linear Algebra"));
        assert_eq!(r.destination().unwrap().parent().unwrap(), tmp.path().join("University/Mathematik/Linear Algebra"));
        let b = cfg.watch_path().join("stats.pdf");
        make_pdf(&b, "x");
        assert_eq!(sort_file(&b, &cfg).destination().unwrap().parent().unwrap(), tmp.path().join("Elsewhere/Stats"));
    }

    #[test]
    fn imported_modules_sort_into_subject_folders() {
        let tmp = tempfile::tempdir().unwrap();
        let studium = tmp.path().join("Studium");
        for sub in ["Informatik/Einführung in die Analysis I", "Mathematik/Analysis II", "Mathematik/Lineare Algebra"]
        {
            fs::create_dir_all(studium.join(sub)).unwrap();
        }
        let downloads = tmp.path().join("Downloads");
        fs::create_dir_all(&downloads).unwrap();
        let cfg = Config {
            watch_folder: downloads.to_string_lossy().into(),
            destination_root: studium.to_string_lossy().into(),
            courses: crate::config::scan_study_folder(&studium),
            ..Config::default()
        };
        let sort = |name: &str, text: &str| {
            let p = downloads.join(name);
            make_pdf(&p, text);
            sort_file(&p, &cfg).destination().map(|d| d.parent().unwrap().to_path_buf())
        };
        assert_eq!(sort("Analysis_II_Blatt3.pdf", "x"), Some(studium.join("Mathematik/Analysis II")));
        assert_eq!(sort("skript.pdf", "Lineare Algebra, Kapitel 2"), Some(studium.join("Mathematik/Lineare Algebra")));
        assert_eq!(sort("Analysis_I_Blatt1.pdf", "x"), Some(studium.join("Informatik/Einführung in die Analysis I")));
        // Content of a real-world PDF (subset fonts, metadata title).
        let p = downloads.join("vl4.pdf");
        fs::copy(fixture("analysis_chromium.pdf"), &p).unwrap();
        assert_eq!(
            sort_file(&p, &cfg).destination().map(|d| d.parent().unwrap().to_path_buf()),
            Some(studium.join("Informatik/Einführung in die Analysis I"))
        );
    }

    #[test]
    fn identical_duplicate_is_removed_not_kept_twice() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = test_config(tmp.path());
        let folder = tmp.path().join("University").join("Programming Course");
        fs::create_dir_all(&folder).unwrap();
        make_pdf(&folder.join("prog1.pdf"), "same");
        let src = cfg.watch_path().join("prog1.pdf");
        make_pdf(&src, "same");
        let result = sort_file(&src, &cfg);
        assert!(matches!(result.outcome, Outcome::DuplicateRemoved { .. }), "{}", result.describe());
        assert!(result.moved());
        assert!(!src.exists(), "download must be gone");
        let names: Vec<_> = fs::read_dir(&folder).unwrap().flatten().map(|e| file_name(&e.path())).collect();
        assert_eq!(names, vec!["prog1.pdf"], "no second copy");
    }

    #[test]
    fn same_content_compares_bytes() {
        let tmp = tempfile::tempdir().unwrap();
        let (a, b, c) = (tmp.path().join("a"), tmp.path().join("b"), tmp.path().join("c"));
        fs::write(&a, vec![7u8; 200_000]).unwrap();
        fs::write(&b, vec![7u8; 200_000]).unwrap();
        let mut other = vec![7u8; 200_000];
        other[150_000] = 8;
        fs::write(&c, other).unwrap();
        assert!(same_content(&a, &b));
        assert!(!same_content(&a, &c));
        assert!(!same_content(&a, &tmp.path().join("missing")));
    }

    #[test]
    fn a_moved_file_exists_exactly_once() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = test_config(tmp.path());
        for i in 0..5 {
            make_pdf(&cfg.watch_path().join(format!("python_{i}.pdf")), "x");
        }
        for r in sort_existing(&cfg) {
            assert!(r.moved(), "{}", r.describe());
            assert!(!r.source.exists(), "{} still in the watch folder", r.file_name());
            assert!(r.destination().unwrap().exists());
        }
        assert_eq!(fs::read_dir(cfg.watch_path()).unwrap().count(), 0);
    }

    /// Another program (browser, PDF preview, virus scanner) holds the file open without
    /// allowing deletion: the sorter waits until it is released, then moves it once.
    #[cfg(windows)]
    #[test]
    fn waits_for_a_file_held_open_by_another_program() {
        use std::os::windows::fs::OpenOptionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let cfg = test_config(tmp.path());
        let src = cfg.watch_path().join("python_locked.pdf");
        make_pdf(&src, "x");
        let handle = fs::OpenOptions::new().read(true).share_mode(1 /* FILE_SHARE_READ only */).open(&src).unwrap();
        let release = thread::spawn(move || {
            thread::sleep(Duration::from_millis(1500));
            drop(handle);
        });
        let result = sort_file_with_patience(&src, &cfg, Duration::from_secs(10));
        release.join().unwrap();
        assert!(result.moved(), "{}", result.describe());
        assert!(!src.exists());
    }

    /// If the file stays locked, nothing is changed and the result says it is in use.
    #[cfg(windows)]
    #[test]
    fn a_file_that_stays_locked_is_left_untouched() {
        use std::os::windows::fs::OpenOptionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let cfg = test_config(tmp.path());
        let src = cfg.watch_path().join("python_locked.pdf");
        make_pdf(&src, "x");
        let _handle = fs::OpenOptions::new().read(true).share_mode(1).open(&src).unwrap();
        let result = sort_file_with_patience(&src, &cfg, Duration::from_millis(600));
        assert!(matches!(result.outcome, Outcome::Left(LeftReason::InUse { .. })), "{}", result.describe());
        assert!(src.exists());
        let folder = tmp.path().join("University").join("Programming Course");
        assert_eq!(fs::read_dir(&folder).map(|d| d.count()).unwrap_or(0), 0, "no copy left behind");
    }

    #[test]
    fn chosen_files_of_any_type_and_folders() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = test_config(tmp.path());
        let elsewhere = tmp.path().join("Desktop");
        fs::create_dir_all(&elsewhere).unwrap();
        // A picture the user right-clicked: not a document type, but sorted anyway.
        let picture = elsewhere.join("prog1_diagram.png");
        fs::write(&picture, b"png").unwrap();
        // A folder: its document files are sorted, not the folder itself.
        let folder = tmp.path().join("Stuff");
        fs::create_dir_all(&folder).unwrap();
        make_pdf(&folder.join("stats_sheet.pdf"), "x");
        fs::write(folder.join("notes.exe"), b"x").unwrap();
        // A browser's unfinished download is never touched.
        let partial = elsewhere.join("python.pdf.crdownload");
        fs::write(&partial, b"x").unwrap();

        let results = sort_chosen(&[picture.clone(), folder.clone(), partial.clone()], &cfg);
        let uni = tmp.path().join("University");
        assert!(uni.join("Programming Course/prog1_diagram.png").exists());
        assert!(uni.join("Statistics/stats_sheet.pdf").exists());
        assert!(folder.join("notes.exe").exists(), "only documents from a folder");
        assert!(partial.exists());
        assert_eq!(results.len(), 3);
        assert!(!picture.exists());
    }

    #[test]
    fn move_file_falls_back_to_copy() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("a.pdf");
        fs::write(&src, "data").unwrap();
        // Moving onto a path inside a missing directory fails for both strategies.
        assert!(move_file(&src, &tmp.path().join("missing").join("a.pdf")).is_err());
        assert!(src.exists(), "source must survive a failed move");
        move_file(&src, &tmp.path().join("b.pdf")).unwrap();
        assert!(!src.exists());
        assert_eq!(fs::read_to_string(tmp.path().join("b.pdf")).unwrap(), "data");
    }
}
