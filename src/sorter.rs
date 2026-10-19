//! Match a single file to a module and move it into that module's folder.

use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

use serde::{Deserialize, Serialize};

use crate::categories;
use crate::config::{Config, ExistingScope, FileAction};
use crate::extract::extract_text;
use crate::ledger::{COPIED_LIST, CopyLedger, KEPT_LIST, KeptFiles};
use crate::matcher::{Match, Reason, match_file};
use crate::origin;
use crate::unzip::Extracted;

/// Partial downloads written by browsers before the file is complete.
pub const TEMP_SUFFIXES: &[&str] = &[".crdownload", ".part", ".partial", ".download", ".tmp", ".temp", ".opdownload"];

/// How long to keep retrying a file that another program (browser, PDF preview,
/// virus scanner) still has open, before giving up for now.
pub(crate) const IN_USE_RETRY: Duration = Duration::from_secs(8);

/// What happened to a file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Outcome {
    /// Moved into the module folder.
    Moved { course: String, destination: PathBuf, reason: Reason },
    /// Copied into the module folder (copy mode); the original stays where it was.
    Copied { course: String, destination: PathBuf, reason: Reason },
    /// The very same file was already in the module folder, so the download was removed.
    DuplicateRemoved { course: String, existing: PathBuf },
    /// The file stays where it is.
    Left(LeftReason),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
    /// Copy mode: this file was copied before (or the module already has the same file).
    AlreadyCopied {
        course: String,
        copy: PathBuf,
    },
    /// Sorted from the right-click menu while another program had it open (no later retry).
    StillOpen {
        course: String,
    },
    /// The user undid sorting this file, so it stays where it is.
    Kept,
    Error(String),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SortResult {
    pub source: PathBuf,
    pub outcome: Outcome,
    /// Its entry in the history, once recorded there (needed to undo it).
    #[serde(skip)]
    pub id: Option<u64>,
    /// What else happened along with it.
    #[serde(default, skip_serializing_if = "Extras::is_empty")]
    pub extras: Box<Extras>,
}

/// Steps taken along with sorting a file, which undo reverses too.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Extras {
    /// Older versions of the file moved into the versions folder: (from, to).
    pub older_versions: Vec<(PathBuf, PathBuf)>,
    /// The ZIP file was unpacked here.
    pub extracted: Option<Extracted>,
    /// The ZIP file could not be unpacked (why).
    pub unzip_failed: Option<String>,
}

impl Extras {
    pub fn is_empty(&self) -> bool {
        *self == Extras::default()
    }
}

impl SortResult {
    pub fn new(source: PathBuf, outcome: Outcome) -> Self {
        SortResult { source, outcome, id: None, extras: Box::default() }
    }

    /// Every path in the study folder this result refers to, to follow the folder when it moves.
    pub fn study_paths_mut(&mut self) -> Vec<&mut PathBuf> {
        let mut paths = Vec::new();
        match &mut self.outcome {
            Outcome::Moved { destination, .. } | Outcome::Copied { destination, .. } => paths.push(destination),
            Outcome::DuplicateRemoved { existing, .. } => paths.push(existing),
            Outcome::Left(_) => {}
        }
        for (from, to) in &mut self.extras.older_versions {
            paths.push(from);
            paths.push(to);
        }
        if let Some(extracted) = &mut self.extras.extracted {
            paths.push(&mut extracted.folder);
        }
        paths
    }

    fn left(source: &Path, reason: LeftReason) -> Self {
        SortResult::new(source.to_path_buf(), Outcome::Left(reason))
    }

    /// The file's new name, when it had to be saved under another one ("x (1).pdf").
    pub fn renamed_to(&self) -> Option<String> {
        let destination = match &self.outcome {
            Outcome::Moved { destination, .. } | Outcome::Copied { destination, .. } => destination,
            _ => return None,
        };
        let new = file_name(destination);
        (new != self.file_name()).then_some(new)
    }

    pub fn file_name(&self) -> String {
        file_name(&self.source)
    }

    /// True when the file is now in its module folder (moved, copied, or was there already as a duplicate).
    pub fn sorted(&self) -> bool {
        !matches!(self.outcome, Outcome::Left(_))
    }

    /// The module the file went into (`None` while it stayed where it was).
    pub fn sorted_course(&self) -> Option<&str> {
        match &self.outcome {
            Outcome::Moved { course, .. }
            | Outcome::Copied { course, .. }
            | Outcome::DuplicateRemoved { course, .. } => Some(course),
            Outcome::Left(_) => None,
        }
    }

    #[cfg(test)]
    pub fn course(&self) -> Option<&str> {
        match &self.outcome {
            Outcome::Moved { course, .. }
            | Outcome::Copied { course, .. }
            | Outcome::DuplicateRemoved { course, .. } => Some(course),
            Outcome::Left(
                LeftReason::AlreadyInFolder { course }
                | LeftReason::InUse { course }
                | LeftReason::StillOpen { course }
                | LeftReason::AlreadyCopied { course, .. },
            ) => Some(course),
            Outcome::Left(_) => None,
        }
    }

    #[cfg(test)]
    pub fn destination(&self) -> Option<&Path> {
        match &self.outcome {
            Outcome::Moved { destination, .. } | Outcome::Copied { destination, .. } => Some(destination),
            Outcome::DuplicateRemoved { existing, .. } => Some(existing),
            Outcome::Left(_) => None,
        }
    }

    /// English description, for the log file and the command line.
    pub fn describe(&self) -> String {
        let name = self.file_name();
        let saved_as =
            self.renamed_to().map(|n| format!(" as {n}, another version is already there")).unwrap_or_default();
        match &self.outcome {
            Outcome::Moved { course, reason, .. } => {
                format!("Moved {name} → {course}{saved_as} ({})", reason.describe())
            }
            Outcome::Copied { course, reason, .. } => {
                format!("Copied {name} → {course}{saved_as} ({})", reason.describe())
            }
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
                    LeftReason::AlreadyCopied { course, .. } => format!("already copied to {course}"),
                    LeftReason::StillOpen { course } => {
                        format!("still open in another program; close it and try again (belongs to {course})")
                    }
                    LeftReason::Kept => "sorting was undone, so it stays here".to_string(),
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
    !TEMP_SUFFIXES.contains(&ext.as_str())
        && (config.extensions.contains(&ext) || ext == ".zip" && config.features.unzip)
}

/// Claim a free name in `folder`: `filename`, or "name (1).ext", "name (2).ext", …
///
/// The name is taken by creating an empty placeholder file that didn't exist before, so
/// the file moved or copied there afterwards can never replace one of the user's files
/// (on Windows, `fs::rename` would silently overwrite an existing file).
pub fn reserve_destination(folder: &Path, filename: &str) -> io::Result<PathBuf> {
    let path = Path::new(filename);
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let ext = path.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    for n in 0.. {
        let candidate = if n == 0 { folder.join(filename) } else { folder.join(format!("{stem} ({n}){ext}")) };
        match fs::OpenOptions::new().write(true).create_new(true).open(&candidate) {
            Ok(_) => return Ok(candidate),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            // Windows says "access denied" instead when a folder has that name.
            Err(_) if fs::symlink_metadata(&candidate).is_ok() => continue,
            Err(e) => return Err(e),
        }
    }
    unreachable!("an unused name always exists")
}

/// Remove a placeholder from `reserve_destination` that wasn't filled after all.
pub(crate) fn release_placeholder(path: &Path) {
    if fs::metadata(path).is_ok_and(|m| m.len() == 0) {
        let _ = fs::remove_file(path);
    }
}

/// Fails with a sharing violation while another program has the file open (Windows).
///
/// PDF viewers like Edge or Acrobat may still show the file and save annotations into it
/// later; moving or removing it now would take it away from under them.
pub(crate) fn ensure_not_open_elsewhere(path: &Path) -> io::Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        fs::OpenOptions::new().read(true).share_mode(0).open(path).map(drop)
    }
    #[cfg(not(windows))]
    {
        fs::metadata(path).map(drop)
    }
}

/// Put a file into the Recycle Bin (Windows), so it can be restored from there.
/// Where that isn't possible it is deleted (only used for files that exist elsewhere too).
pub fn discard(path: &Path) -> io::Result<()> {
    #[cfg(windows)]
    match recycle(path) {
        Ok(()) => return Ok(()),
        Err(e) if !path.exists() => {
            log::info!("{}: {e}, but it is gone", path.display());
            return Ok(());
        }
        Err(e) => log::info!("Could not put {} into the Recycle Bin ({e}); deleting it instead", path.display()),
    }
    fs::remove_file(path)
}

/// Put a folder File Sorter made (an unpacked ZIP file) into the Recycle Bin (Windows), or
/// delete it elsewhere.
pub fn discard_folder(path: &Path) -> io::Result<()> {
    #[cfg(windows)]
    match recycle(path) {
        Ok(()) => return Ok(()),
        Err(e) if !path.exists() => {
            log::info!("{}: {e}, but it is gone", path.display());
            return Ok(());
        }
        Err(e) => log::info!("Could not put {} into the Recycle Bin ({e}); deleting it instead", path.display()),
    }
    fs::remove_dir_all(path)
}

#[cfg(windows)]
fn recycle(path: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::UI::Shell::{
        FO_DELETE, FOF_ALLOWUNDO, FOF_NOCONFIRMATION, FOF_NOERRORUI, FOF_SILENT, SHFILEOPSTRUCTW, SHFileOperationW,
    };
    let full = std::path::absolute(path)?;
    // A list of paths, each ending in a 0, and one more 0 at the end.
    let from: Vec<u16> = full.as_os_str().encode_wide().chain([0, 0]).collect();
    let mut op = SHFILEOPSTRUCTW {
        hwnd: std::ptr::null_mut(),
        wFunc: FO_DELETE,
        pFrom: from.as_ptr(),
        pTo: std::ptr::null(),
        fFlags: (FOF_ALLOWUNDO | FOF_NOCONFIRMATION | FOF_NOERRORUI | FOF_SILENT) as u16,
        fAnyOperationsAborted: 0,
        hNameMappings: std::ptr::null_mut(),
        lpszProgressTitle: std::ptr::null(),
    };
    // SAFETY: `op` and the double-0-terminated `from` live until the call returns.
    let code = unsafe { SHFileOperationW(&mut op) };
    if code != 0 || op.fAnyOperationsAborted != 0 {
        return Err(io::Error::other(format!("Recycle Bin error {code:#x}")));
    }
    if path.exists() {
        return Err(io::Error::other("still there after moving it to the Recycle Bin"));
    }
    Ok(())
}

/// True if the error means another program still has the file open.
pub(crate) fn in_use(e: &io::Error) -> bool {
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
pub(crate) fn retry_while_in_use<T>(patience: Duration, mut op: impl FnMut() -> io::Result<T>) -> io::Result<T> {
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

/// Copy a file (copy mode), waiting up to `patience` while another program has it open.
/// A copy that didn't arrive complete is removed again.
pub fn copy_file_with_patience(from: &Path, to: &Path, patience: Duration) -> Result<(), MoveError> {
    let size = fs::metadata(from).map_err(MoveError::Failed)?.len();
    if let Err(e) = retry_while_in_use(patience, || fs::copy(from, to)) {
        let _ = fs::remove_file(to);
        return Err(if in_use(&e) { MoveError::InUse(e) } else { MoveError::Failed(e) });
    }
    if !fs::metadata(to).map(|m| m.len() == size).unwrap_or(false) {
        let _ = fs::remove_file(to);
        return Err(MoveError::Failed(io::Error::other("the file could not be copied completely")));
    }
    Ok(())
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
    if is_kept(path, config) {
        return SortResult::left(path, LeftReason::Kept);
    }
    sort_any_file(path, config, patience, Target::Auto)
}

/// Which module a file goes to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    /// The one its name, content or origin points to.
    Auto,
    /// This one (index into `config.courses`), chosen by the user.
    Course(usize),
}

/// Put a file into the module the user chose («Assign»), recorded in the history like any sort.
pub fn sort_into(path: &Path, course: usize, config: &Config) -> SortResult {
    stop_keeping(path, config);
    let mut result = sort_any_file(path, config, IN_USE_RETRY, Target::Course(course));
    if let Outcome::Left(LeftReason::InUse { course }) = &result.outcome {
        result.outcome = Outcome::Left(LeftReason::StillOpen { course: course.clone() });
    }
    result
}

/// True if the user undid sorting this file, so it stays where it is.
pub fn is_kept(path: &Path, config: &Config) -> bool {
    config.state_file(KEPT_LIST).is_some_and(|file| KeptFiles::load(&file).contains(path))
}

/// The user sorts this file explicitly, so it is no longer left alone.
fn stop_keeping(path: &Path, config: &Config) {
    if let Some(file) = config.state_file(KEPT_LIST) {
        let mut kept = KeptFiles::load(&file);
        if kept.forget(path) {
            let _ = kept.save(&file);
        }
    }
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
                stop_keeping(path, config);
                sort_any_file(path, config, IN_USE_RETRY, Target::Auto)
            });
        }
    }
    // Nothing retries these later, so say what to do instead of "will be sorted when free".
    for result in &mut results {
        if let Outcome::Left(LeftReason::InUse { course }) = &result.outcome {
            result.outcome = Outcome::Left(LeftReason::StillOpen { course: course.clone() });
        }
    }
    results
}

fn sort_any_file(path: &Path, config: &Config, patience: Duration, target: Target) -> SortResult {
    let mut result = sort_any_file_unrecorded(path, config, patience, target);
    if result.sorted()
        && let Some(dir) = &config.state_dir
    {
        // Earlier entries of an older version follow it into the versions folder (before this
        // entry is written, which keeps the name it freed): undoing them must find that file,
        // not the new one now under its old name.
        for (from, to) in &result.extras.older_versions {
            note_moved(dir, from, to);
        }
        match crate::history::record(dir, &result) {
            Ok(id) => result.id = Some(id),
            Err(e) => log::info!("Could not add {} to the history: {e}", result.file_name()),
        }
    }
    result
}

/// The module for `path` and why, or `None` if it isn't clear. The text read from the file
/// (if it was read) is left in `text`.
fn choose_course(
    path: &Path,
    config: &Config,
    origin: Option<&origin::Source>,
    text: &mut Option<String>,
) -> Option<Match> {
    // A ZIP file's list of files is like a name, so it is always read.
    let zip = suffix(path) == ".zip";
    let read = |p: &Path| {
        let t = if zip && !config.read_file_contents { crate::extract::zip_names(p) } else { extract_text(p) };
        *text = Some(t.clone());
        t
    };
    if let Some(m) = match_file(path, &config.courses, config.read_file_contents || zip, read) {
        return Some(m);
    }
    // Nothing (or a tie) by name and content: files from the same course page go together.
    let source = origin?;
    let course = origin::lookup(config, source)?;
    Some(Match { course, score: 0, reason: Reason::Origin(source.host.clone()) })
}

fn sort_any_file_unrecorded(path: &Path, config: &Config, patience: Duration, target: Target) -> SortResult {
    if !path.is_file() {
        return SortResult::left(path, LeftReason::Gone);
    }
    let copying = config.action == FileAction::Copy;
    let ledger_file = config.state_file(COPIED_LIST);
    let mut ledger = match (&ledger_file, copying) {
        (Some(file), true) => Some((CopyLedger::load(file), file)),
        _ => None,
    };
    if target == Target::Auto
        && let Some(earlier) = ledger.as_ref().and_then(|(l, _)| l.earlier_copy(path))
    {
        return SortResult::left(path, LeftReason::AlreadyCopied { course: earlier.course, copy: earlier.copy });
    }
    // Read before the file moves (the note stays with the file, but only on the same drive).
    let origin = if config.features.origin { origin::source_of(path) } else { None };
    let mut text = None;
    let m = match target {
        Target::Course(i) if i < config.courses.len() => Match { course: i, score: 0, reason: Reason::Chosen },
        Target::Course(_) => return SortResult::left(path, LeftReason::Error("this module no longer exists".into())),
        Target::Auto => match choose_course(path, config, origin.as_ref(), &mut text) {
            Some(m) => m,
            None => return SortResult::left(path, LeftReason::NoMatch),
        },
    };
    let result = place_file(path, config, patience, m, copying, &mut ledger, text);
    if let Some(source) = &origin
        && let Outcome::Moved { course, reason, .. } | Outcome::Copied { course, reason, .. } = &result.outcome
        && !matches!(reason, Reason::Origin(_))
    {
        origin::remember(config, source, course, *reason == Reason::Chosen);
    }
    result
}

/// The folder for `path` inside its module: a subfolder for lectures, exercises, … if
/// that is switched on and the name (or the start of the text) tells the kind.
fn destination_folder(config: &Config, course_folder: &Path, path: &Path, text: Option<String>) -> PathBuf {
    let features = &config.features;
    if !features.subfolders {
        return course_folder.to_path_buf();
    }
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let category = categories::classify(&stem, None).or_else(|| {
        if !config.read_file_contents || suffix(path) == ".zip" {
            return None;
        }
        let text = text.unwrap_or_else(|| extract_text(path));
        categories::classify(&stem, Some(&text))
    });
    let name = category.map(|c| categories::safe_folder_name(&features.subfolder_names[c as usize]));
    match name {
        Some(name) if !name.is_empty() => course_folder.join(name),
        _ => course_folder.to_path_buf(),
    }
}

/// Move or copy `path` into the module `m` points to.
fn place_file(
    path: &Path,
    config: &Config,
    patience: Duration,
    m: Match,
    copying: bool,
    ledger: &mut Option<(CopyLedger, &PathBuf)>,
    text: Option<String>,
) -> SortResult {
    let course = &config.courses[m.course];
    let label = course.label();
    let course_folder = config.course_folder(course);
    // Anywhere in the module already (also in one of its subfolders): leave it there.
    if let Some(parent) = path.parent()
        && (same_dir(&course_folder, parent)
            || course_folder.is_dir() && crate::cloud::is_inside(parent, &course_folder))
    {
        return SortResult::left(path, LeftReason::AlreadyInFolder { course: label });
    }
    let mut folder = destination_folder(config, &course_folder, path, text);
    if let Err(e) = fs::create_dir_all(&folder) {
        return SortResult::left(path, LeftReason::Error(format!("cannot create {}: {e}", folder.display())));
    }

    // Leave files alone while another program still has them open.
    match retry_while_in_use(patience, || ensure_not_open_elsewhere(path)) {
        Ok(()) => {}
        Err(e) if in_use(&e) => return SortResult::left(path, LeftReason::InUse { course: label }),
        Err(e) if e.kind() == io::ErrorKind::NotFound => return SortResult::left(path, LeftReason::Gone),
        Err(e) => return SortResult::left(path, LeftReason::Error(e.to_string())),
    }

    // The very same file is already there (downloaded twice, or copied in again):
    // put the download into the Recycle Bin instead of keeping a second copy as "name (1)".
    let same = [folder.join(file_name(path)), course_folder.join(file_name(path))]
        .into_iter()
        .find(|existing| existing.is_file() && same_content(path, existing));
    if let Some(existing) = same {
        if copying {
            remember_copy(ledger, path, &label, &existing);
            return SortResult::left(path, LeftReason::AlreadyCopied { course: label, copy: existing });
        }
        return match retry_while_in_use(patience, || discard(path)) {
            Ok(()) => SortResult::new(path.to_path_buf(), Outcome::DuplicateRemoved { course: label, existing }),
            Err(e) if in_use(&e) => SortResult::left(path, LeftReason::InUse { course: label }),
            Err(e) => SortResult::left(path, LeftReason::Error(e.to_string())),
        };
    }

    // Another version of the file is there: the older one goes into the versions folder.
    let mut older_versions = Vec::new();
    if config.features.versions {
        let versions_name = categories::safe_folder_name(&config.features.versions_folder);
        let versions_name = if versions_name.is_empty() { "Ältere Versionen".to_string() } else { versions_name };
        let others = crate::versions::other_versions(&folder, path);
        if others.iter().any(|other| crate::versions::is_older(path, other)) {
            // The file that arrives is the older one ("v1" while "v2" is there).
            folder = folder.join(&versions_name);
            if let Err(e) = fs::create_dir_all(&folder) {
                return SortResult::left(path, LeftReason::Error(format!("cannot create {}: {e}", folder.display())));
            }
        } else if !others.is_empty() {
            older_versions = move_older_versions(&others, &folder.join(&versions_name));
        }
    }

    let target = match reserve_destination(&folder, &file_name(path)) {
        Ok(target) => target,
        Err(e) => {
            put_back(&older_versions);
            return SortResult::left(path, LeftReason::Error(format!("cannot write to {}: {e}", folder.display())));
        }
    };
    let done = if copying {
        copy_file_with_patience(path, &target, patience)
    } else {
        move_file_with_patience(path, &target, patience)
    };
    if done.is_err() {
        release_placeholder(&target);
        put_back(&older_versions);
    }
    let mut result = match done {
        Ok(()) if copying => {
            remember_copy(ledger, path, &label, &target);
            SortResult::new(
                path.to_path_buf(),
                Outcome::Copied { course: label, destination: target.clone(), reason: m.reason },
            )
        }
        Ok(()) => SortResult::new(
            path.to_path_buf(),
            Outcome::Moved { course: label, destination: target.clone(), reason: m.reason },
        ),
        Err(MoveError::InUse(_)) => return SortResult::left(path, LeftReason::InUse { course: label }),
        Err(MoveError::Failed(e)) => return SortResult::left(path, LeftReason::Error(e.to_string())),
    };
    result.extras.older_versions = older_versions;
    if config.features.unzip && suffix(&target) == ".zip" {
        match crate::unzip::extract_next_to(&target) {
            Ok(extracted) => result.extras.extracted = Some(extracted),
            Err(e) => {
                log::info!("Could not unpack {}: {e}", target.display());
                result.extras.unzip_failed = Some(e.to_string());
            }
        }
    }
    result
}

/// Move `others` (older versions of a file that is about to arrive) into `versions`.
/// One that is open in another program, or can't be moved, stays where it is.
fn move_older_versions(others: &[PathBuf], versions: &Path) -> Vec<(PathBuf, PathBuf)> {
    let mut moved = Vec::new();
    for other in others {
        if ensure_not_open_elsewhere(other).is_err() || fs::create_dir_all(versions).is_err() {
            continue;
        }
        let Ok(to) = reserve_destination(versions, &file_name(other)) else { continue };
        match move_file_with_patience(other, &to, Duration::ZERO) {
            Ok(()) => moved.push((other.clone(), to)),
            Err(e) => {
                release_placeholder(&to);
                log::info!("Could not move the older version {}: {e}", other.display());
            }
        }
    }
    moved
}

/// Note in the history that the sorted file `from` is now at `to`.
pub(crate) fn note_moved(dir: &Path, from: &Path, to: &Path) {
    let renamed = [(from.to_path_buf(), to.to_path_buf())];
    if let Err(e) = crate::history::record_relocation(dir, from, to, &renamed) {
        log::info!("Could not note in the history that {} moved: {e}", from.display());
    }
}

/// Undo `move_older_versions` (the new file couldn't be sorted after all, or it is undone):
/// each older version goes back to its old name, or "name (1)" if that is taken now.
/// Returns the ones that couldn't be moved back, and (from, to) of those that were.
pub(crate) fn put_back(older_versions: &[(PathBuf, PathBuf)]) -> Vec<PathBuf> {
    put_back_moved(older_versions).0
}

pub(crate) fn put_back_moved(older_versions: &[(PathBuf, PathBuf)]) -> (Vec<PathBuf>, Vec<(PathBuf, PathBuf)>) {
    let mut moved = Vec::new();
    let mut stuck = Vec::new();
    for (from, to) in older_versions {
        let (Some(dir), true) = (from.parent(), to.is_file()) else {
            stuck.push(to.clone());
            continue;
        };
        let back = match reserve_destination(dir, &file_name(from)) {
            Ok(back) => back,
            Err(_) => {
                stuck.push(to.clone());
                continue;
            }
        };
        if move_file_with_patience(to, &back, IN_USE_RETRY).is_err() {
            release_placeholder(&back);
            stuck.push(to.clone());
        } else {
            moved.push((to.clone(), back));
        }
    }
    (stuck, moved)
}

fn remember_copy(ledger: &mut Option<(CopyLedger, &PathBuf)>, original: &Path, course: &str, copy: &Path) {
    if let Some((ledger, file)) = ledger {
        ledger.record(original, course, copy);
        if let Err(e) = ledger.save(file) {
            log::info!("Could not update the list of copied files: {e}");
        }
    }
}

/// Sort the files currently sitting in the watch folder: all, or only the recent ones.
pub fn sort_existing(config: &Config, scope: ExistingScope) -> Vec<SortResult> {
    let Ok(entries) = fs::read_dir(config.watch_path()) else { return Vec::new() };
    let mut files: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
    files.sort();
    files
        .into_iter()
        .filter(|p| p.is_file() && is_candidate(p, config) && in_scope(p, scope))
        .map(|p| sort_file(&p, config))
        .collect()
}

/// True if `path` belongs to `scope`: always for `All`; for `Recent(days)` if it was
/// downloaded or changed within that many days. Windows sets the creation time to the
/// moment of the download, while browsers may keep the server's modification time, so the
/// newer of the two counts.
pub fn in_scope(path: &Path, scope: ExistingScope) -> bool {
    let ExistingScope::Recent(days) = scope else { return true };
    let Ok(meta) = fs::metadata(path) else { return false };
    recent([meta.modified().ok(), meta.created().ok()], SystemTime::now(), days)
}

/// True if the newest of `times` is at most `days` days before `now` (or in the future).
fn recent(times: [Option<SystemTime>; 2], now: SystemTime, days: u32) -> bool {
    let limit = Duration::from_secs(u64::from(days) * 24 * 60 * 60);
    let newest = times.into_iter().flatten().max();
    newest.is_some_and(|t| now.duration_since(t).map(|age| age <= limit).unwrap_or(true))
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

    /// Forget the files kept after an undo, so they can be sorted again.
    pub fn forget_kept(config: &Config) {
        if let Some(file) = config.state_file(KEPT_LIST) {
            let _ = fs::remove_file(file);
        }
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
    fn recent_means_downloaded_or_changed_in_the_last_days() {
        let now = SystemTime::now();
        let days = |d: u64| Some(now - Duration::from_secs(d * 24 * 60 * 60));
        assert!(recent([days(1), None], now, 7));
        assert!(recent([days(30), days(2)], now, 7), "downloaded recently, old server date");
        assert!(!recent([days(8), days(9)], now, 7));
        assert!(!recent([None, None], now, 7));
        assert!(recent([Some(now + Duration::from_secs(60)), None], now, 7), "clock skew");
    }

    #[test]
    fn sort_existing_last_days_leaves_older_files() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = test_config(tmp.path());
        let new = cfg.watch_path().join("prog1_new.pdf");
        let old = cfg.watch_path().join("prog1_old.pdf");
        make_pdf(&new, "new");
        make_pdf(&old, "old");
        let long_ago = SystemTime::now() - Duration::from_secs(30 * 24 * 60 * 60);
        let file = fs::OpenOptions::new().write(true).open(&old).unwrap();
        let times = fs::FileTimes::new().set_modified(long_ago);
        #[cfg(windows)]
        let times = std::os::windows::fs::FileTimesExt::set_created(times, long_ago);
        file.set_times(times).unwrap();
        drop(file);
        // Elsewhere the creation time can't be set back, so only Windows can test the old file.
        let looks_old = cfg!(windows) || fs::metadata(&old).unwrap().created().is_err();
        let results = sort_existing(&cfg, ExistingScope::Recent(7));
        assert!(results.iter().any(|r| r.file_name() == "prog1_new.pdf" && r.sorted()));
        if looks_old {
            assert!(old.exists(), "older than 7 days: left alone");
            assert_eq!(results.len(), 1);
        }
        let all = sort_existing(&cfg, ExistingScope::All);
        assert!(all.iter().all(|r| r.sorted()) && !old.exists());
    }

    #[test]
    fn origin_decides_when_name_and_content_do_not() {
        let tmp = tempfile::tempdir().unwrap();
        let mut cfg = test_config(tmp.path());
        cfg.state_dir = Some(tmp.path().join("state"));
        let source = origin::Source { key: "moodle.uni.ch/course/4711".into(), host: "moodle.uni.ch".into() };
        let file = cfg.watch_path().join("Blatt3.pdf");
        make_pdf(&file, "nothing");
        assert_eq!(choose_course(&file, &cfg, Some(&source), &mut None), None, "nothing learned yet");
        origin::remember(&cfg, &source, "Statistics", true);
        let m = choose_course(&file, &cfg, Some(&source), &mut None).unwrap();
        assert_eq!((m.course, m.reason), (2, Reason::Origin("moodle.uni.ch".into())));
        // A clear name still wins.
        let named = cfg.watch_path().join("prog1_Blatt3.pdf");
        make_pdf(&named, "x");
        assert_eq!(choose_course(&named, &cfg, Some(&source), &mut None).unwrap().course, 0);
    }

    /// The browser's note on where a download came from (only Windows keeps it).
    #[cfg(windows)]
    #[test]
    fn files_from_the_same_course_page_go_together() {
        let tmp = tempfile::tempdir().unwrap();
        let mut cfg = test_config(tmp.path());
        cfg.state_dir = Some(tmp.path().join("state"));
        let mark = |path: &Path| {
            let mut stream = path.as_os_str().to_os_string();
            stream.push(":Zone.Identifier");
            fs::write(
                stream,
                "[ZoneTransfer]\r\nZoneId=3\r\nReferrerUrl=https://moodle.uni.ch/course/view.php?id=4711\r\n\
                               HostUrl=https://moodle.uni.ch/pluginfile.php/9/x.pdf\r\n",
            )
            .unwrap();
        };
        let first = cfg.watch_path().join("stats_intro.pdf");
        make_pdf(&first, "x");
        mark(&first);
        assert_eq!(sort_file(&first, &cfg).course(), Some("Statistics"));
        let second = cfg.watch_path().join("Blatt3.pdf");
        make_pdf(&second, "nothing");
        mark(&second);
        let result = sort_file(&second, &cfg);
        assert!(
            matches!(&result.outcome, Outcome::Moved { course, reason: Reason::Origin(host), .. }
            if course == "Statistics" && host == "moodle.uni.ch"),
            "{}",
            result.describe()
        );
        // Switched off: left alone.
        cfg.features.origin = false;
        let third = cfg.watch_path().join("Blatt4.pdf");
        make_pdf(&third, "nothing");
        mark(&third);
        assert_eq!(sort_file(&third, &cfg).outcome, Outcome::Left(LeftReason::NoMatch));
    }

    #[test]
    fn a_folder_with_the_files_name_is_skipped() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("blatt.pdf")).unwrap();
        let claimed = reserve_destination(tmp.path(), "blatt.pdf").unwrap();
        assert_eq!(claimed, tmp.path().join("blatt (1).pdf"));
        assert!(tmp.path().join("blatt.pdf").is_dir(), "the folder is left alone");
    }

    #[test]
    fn moves_matching_file() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = test_config(tmp.path());
        let src = cfg.watch_path().join("prog1_slides.pdf");
        make_pdf(&src, "Week 1");
        let result = sort_file(&src, &cfg);
        assert!(result.sorted(), "{}", result.describe());
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
    fn copy_mode_keeps_the_original_and_copies_once() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg =
            Config { action: FileAction::Copy, state_dir: Some(tmp.path().to_path_buf()), ..test_config(tmp.path()) };
        let src = cfg.watch_path().join("prog1_slides.pdf");
        make_pdf(&src, "Week 1");
        let folder = tmp.path().join("University").join("Programming Course");
        let copy = folder.join("prog1_slides.pdf");

        let result = sort_file(&src, &cfg);
        assert!(matches!(result.outcome, Outcome::Copied { .. }), "{}", result.describe());
        assert!(result.sorted() && src.exists());
        assert_eq!(fs::read(&src).unwrap(), fs::read(&copy).unwrap());

        // Sorting again does nothing, even after the copy was annotated.
        fs::write(&copy, "annotated").unwrap();
        let again = sort_file(&src, &cfg);
        assert!(matches!(again.outcome, Outcome::Left(LeftReason::AlreadyCopied { .. })), "{}", again.describe());
        assert_eq!(again.describe(), "Left prog1_slides.pdf in place – already copied to Programming Course");
        assert_eq!(fs::read_dir(&folder).unwrap().count(), 1);

        // A new version under the same name is copied next to the old one.
        make_pdf(&src, "Week 1, corrected");
        let newer = sort_file(&src, &cfg);
        assert_eq!(newer.destination(), Some(folder.join("prog1_slides (1).pdf").as_path()), "{}", newer.describe());
    }

    #[test]
    fn copy_mode_without_list_skips_identical_files() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = Config { action: FileAction::Copy, ..test_config(tmp.path()) };
        let src = cfg.watch_path().join("stats_sheet.pdf");
        make_pdf(&src, "Sheet 1");
        assert!(matches!(sort_file(&src, &cfg).outcome, Outcome::Copied { .. }));
        let again = sort_file(&src, &cfg);
        assert!(matches!(again.outcome, Outcome::Left(LeftReason::AlreadyCopied { .. })), "{}", again.describe());
        assert_eq!(fs::read_dir(tmp.path().join("University").join("Statistics")).unwrap().count(), 1);
    }

    #[test]
    fn unmatched_file_stays() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = test_config(tmp.path());
        let src = cfg.watch_path().join("holiday.pdf");
        make_pdf(&src, "Beach trip");
        let result = sort_file(&src, &cfg);
        assert!(!result.sorted() && src.exists());
        assert_eq!(result.describe(), "Left holiday.pdf in place – no matching module");
    }

    #[test]
    fn name_collision_gets_number_and_is_claimed() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("a.pdf"), "x").unwrap();
        fs::write(tmp.path().join("a (1).pdf"), "x").unwrap();
        let first = reserve_destination(tmp.path(), "a.pdf").unwrap();
        assert_eq!(file_name(&first), "a (2).pdf");
        // The name is taken now, so the next one gets the next number; nothing existing was touched.
        assert_eq!(file_name(&reserve_destination(tmp.path(), "a.pdf").unwrap()), "a (3).pdf");
        assert_eq!(fs::read_to_string(tmp.path().join("a.pdf")).unwrap(), "x");
        assert_eq!(file_name(&reserve_destination(tmp.path(), "b.pdf").unwrap()), "b.pdf");
        release_placeholder(&first);
        assert!(!first.exists());
    }

    #[test]
    fn annotated_copy_in_module_is_never_replaced() {
        // The module already has an annotated version; the original arrives again.
        let tmp = tempfile::tempdir().unwrap();
        let cfg = test_config(tmp.path());
        let folder = tmp.path().join("University").join("Programming Course");
        fs::create_dir_all(&folder).unwrap();
        let src = cfg.watch_path().join("prog1_skript.pdf");
        make_pdf(&src, "Skript");
        let original = fs::read(&src).unwrap();
        let mut annotated = original.clone();
        annotated.extend_from_slice(b"\n% my notes");
        fs::write(folder.join("prog1_skript.pdf"), &annotated).unwrap();

        let result = sort_file(&src, &cfg);
        assert_eq!(file_name(result.destination().unwrap()), "prog1_skript (1).pdf", "{}", result.describe());
        assert_eq!(fs::read(folder.join("prog1_skript.pdf")).unwrap(), annotated, "annotated file changed");
        assert_eq!(fs::read(folder.join("prog1_skript (1).pdf")).unwrap(), original);
        assert!(!src.exists());
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
        let results = sort_existing(&cfg, ExistingScope::All);
        let get = |n: &str| results.iter().find(|r| r.file_name() == n).unwrap();
        assert_eq!(get("stats_hw.pdf").course(), Some("Statistics"));
        assert!(!get("random.pdf").sorted());
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
        assert!(result.sorted());
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
        for r in sort_existing(&cfg, ExistingScope::All) {
            assert!(r.sorted(), "{}", r.describe());
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
        assert!(result.sorted(), "{}", result.describe());
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

    /// A viewer that lets others rename or delete the file (all sharing allowed) still has
    /// it open: it is left alone until it closes, so annotations saved later aren't lost.
    #[cfg(windows)]
    #[test]
    fn a_file_open_in_a_viewer_waits_until_it_is_closed() {
        use std::os::windows::fs::OpenOptionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let cfg = test_config(tmp.path());
        let src = cfg.watch_path().join("python_viewer.pdf");
        make_pdf(&src, "x");
        let viewer = fs::OpenOptions::new().read(true).share_mode(7).open(&src).unwrap();
        let result = sort_file_with_patience(&src, &cfg, Duration::from_millis(600));
        assert!(matches!(result.outcome, Outcome::Left(LeftReason::InUse { .. })), "{}", result.describe());
        assert!(src.exists());
        drop(viewer);
        assert!(sort_file(&src, &cfg).sorted());
    }

    /// Duplicates go to the Recycle Bin, not away for good.
    #[cfg(windows)]
    #[test]
    fn discard_uses_the_recycle_bin() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("file-sorter-recycle-test.txt");
        fs::write(&file, "x").unwrap();
        discard(&file).unwrap();
        assert!(!file.exists());
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
