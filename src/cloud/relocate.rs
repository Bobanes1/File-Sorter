//! Moving the whole study folder into the cloud folder (or back), without losing a file.
//!
//! - Every file below `from` goes to the same relative place below `to`; folders are
//!   created as needed and merged with what is already there. Empty folders come along
//!   too: they may be modules without files yet.
//! - Never overwrites: when `to` already has a different file of that name, the file is
//!   saved as "name (1).ext" (`sorter::reserve_destination`) and listed in `renamed`.
//! - A file whose identical twin (same content) is already there is not copied again; the
//!   one in `from` goes to the Recycle Bin (`sorter::discard`) and counts as a duplicate.
//!   A file reached through a linked folder is never taken for its own twin. A twin that is
//!   only in the cloud is downloaded to compare; if that fails, the file stays.
//! - Files another program has open (a PDF viewer may still save notes into them) or that
//!   fail stay where they are and are listed in `left`; everything else carries on. The
//!   wait for an open file is short, so a few open files don't stall moving thousands.
//!   "Open" is checked again right before the step that can't be undone (the rename, or
//!   removing the original after copying), since hashing or copying a big file takes long.
//! - Entries that can't be read, and (on Windows) names ending in a dot or space, which
//!   Windows programs can't open, stay too and are listed in `left`.
//! - On the same drive a file is simply renamed (instant and atomic). Across drives it is
//!   copied to a temporary file beside its new name, checked, written to the disk and
//!   renamed into place before the original is removed: killed at any moment, no half file
//!   ever has a real name, and every file is complete in at least one place. If the
//!   original can't be removed, the copy is removed again, so a file is not in both places.
//!   (`sorter::move_file_with_patience` copies straight onto the real name, so it isn't used.)
//!   An empty file with a temporary file beside it is the name claimed by a run that was
//!   stopped while copying; the next run fills that name instead of using "name (1)".
//! - The modification time is kept, also where copying would set it to now. A file that is
//!   only in the cloud (moving back out of OneDrive) is downloaded first.
//! - Junk files (`cloud::is_junk`): moved if `to` has none of that name, otherwise put in
//!   the Recycle Bin. They are not counted, and problems with them are only logged.
//! - Symbolic links and junctions are left alone: following one could move files from
//!   outside the study folder.
//! - Afterwards empty folders in `from` are removed, `from` itself too when empty; nothing
//!   outside `from` is ever removed.
//! - `cancel` is checked between files; a cancelled run leaves every file in one piece and
//!   keeps the folders in `from`, so the rest can be moved later. (The settings already
//!   name `to` as the study folder; offering the rest is up to the caller.)

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::cloud::{Stamp, TEMP_SUFFIX, is_inside, is_junk, is_placeholder, stamp};
use crate::sorter::{self, MoveError};

/// How long to wait for a file another program has open before leaving it where it is.
/// Much shorter than the sorter's wait: moving thousands of files must not stall on each.
const PATIENCE: Duration = Duration::from_secs(2);

/// Why a file stays where it is while another program has it open.
const IN_USE: &str = "open in another program";

/// What a move would involve.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Plan {
    pub files: u64,
    pub bytes: u64,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Progress {
    pub done: u64,
    pub total: u64,
    pub bytes_done: u64,
    pub bytes_total: u64,
    /// The file being moved, relative to the study folder.
    pub current: String,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Report {
    /// Moved into `to`, including the ones in `renamed`.
    pub moved: u64,
    /// Already in the target with the same content; the one here went to the Recycle Bin.
    pub duplicates: u64,
    /// Saved under another name because a different file of that name was there: (from, to).
    pub renamed: Vec<(PathBuf, PathBuf)>,
    /// Not moved (open in another program, unreadable, or an error): (file or folder,
    /// reason). They stay put.
    pub left: Vec<(PathBuf, String)>,
    pub cancelled: bool,
}

#[derive(Debug)]
pub enum RelocateError {
    /// `from` and `to` are the same folder.
    Same,
    /// One is inside the other.
    Nested,
    /// There is no folder at `from`.
    SourceMissing,
    Io(io::Error),
}

impl std::fmt::Display for RelocateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RelocateError::Same => write!(f, "the folders are the same"),
            RelocateError::Nested => write!(f, "one folder is inside the other"),
            RelocateError::SourceMissing => write!(f, "the study folder does not exist"),
            RelocateError::Io(e) => write!(f, "{e}"),
        }
    }
}

/// What moving `from` would involve (junk files not counted). Empty if it doesn't exist.
/// Entries that can't be read count as files: they would stay behind, and a check for what
/// is left at the old place must see them.
pub fn plan(from: &Path) -> Plan {
    let Ok(tree) = scan(from) else { return Plan::default() };
    let (files, bytes) = totals(&tree.files);
    Plan { files: files + tree.unreadable.len() as u64, bytes }
}

/// Move everything in `from` into `to` (see the module comment). `progress` is called
/// before each file (junk not counted) and once at the end, with `done == total` unless
/// cancelled.
pub fn relocate(
    from: &Path,
    to: &Path,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(&Progress),
) -> Result<Report, RelocateError> {
    check_apart(from, to)?;
    if !from.is_dir() {
        return Err(RelocateError::SourceMissing);
    }
    let tree = scan(from).map_err(RelocateError::Io)?;
    // The folders `create_dir_all` is about to make, deepest first, to undo it if need be.
    let made: Vec<PathBuf> =
        to.ancestors().take_while(|dir| !dir.as_os_str().is_empty() && !dir.exists()).map(Path::to_path_buf).collect();
    fs::create_dir_all(to).map_err(RelocateError::Io)?;
    // Only now that `to` exists can links and short names in its path be resolved: a `to`
    // reached through a link into `from` looked separate before.
    if let Err(e) = check_apart(from, to) {
        for dir in &made {
            let _ = fs::remove_dir(dir);
        }
        return Err(e);
    }

    let (total, bytes_total) = totals(&tree.files);
    let mut status = Progress { total, bytes_total, ..Progress::default() };
    log::info!("Moving the study folder {} → {} ({total} files)", from.display(), to.display());
    let mut report = Report { left: tree.unreadable, ..Report::default() };
    for (folder, reason) in &report.left {
        log::info!("Left {} in place: {reason}", folder.display());
    }
    for item in &tree.files {
        if !item.junk {
            status.current = slashed(&item.rel);
            progress(&status);
        }
        if cancel.load(Ordering::Relaxed) {
            report.cancelled = true;
            break;
        }
        let dir = below(to, item.rel.parent().unwrap_or(Path::new("")));
        if item.junk {
            if let Err(reason) = move_junk(&item.path, &dir) {
                log::info!("Left {} in place: {reason}", item.path.display());
            }
            continue;
        }
        match move_one(&item.path, &dir) {
            Ok(Moved::Here) => report.moved += 1,
            Ok(Moved::Renamed(target)) => {
                log::info!("Moved {} as {}: another version is already there", item.path.display(), target.display());
                report.moved += 1;
                report.renamed.push((item.path.clone(), target));
            }
            Ok(Moved::Duplicate) => report.duplicates += 1,
            Ok(Moved::Gone) => log::info!("{} is gone, nothing to move", item.path.display()),
            Err(reason) => {
                log::info!("Left {} in place: {reason}", item.path.display());
                report.left.push((item.path.clone(), reason));
            }
        }
        status.done += 1;
        status.bytes_done += item.size;
    }
    status.current.clear();
    progress(&status);
    if !report.cancelled {
        take_empty_folders(from, to, &tree.folders);
    }
    log::info!(
        "Moved the study folder {} → {}: {} moved ({} under another name), {} duplicates to the Recycle Bin, {} left in place{}",
        from.display(),
        to.display(),
        report.moved,
        report.renamed.len(),
        report.duplicates,
        report.left.len(),
        if report.cancelled { ", cancelled" } else { "" },
    );
    Ok(report)
}

/// `from` and `to` must be two separate folders: moving a folder into itself never ends.
fn check_apart(from: &Path, to: &Path) -> Result<(), RelocateError> {
    match (is_inside(to, from), is_inside(from, to)) {
        (true, true) => Err(RelocateError::Same),
        (false, false) => Ok(()),
        _ => Err(RelocateError::Nested),
    }
}

/// A file below the folder being moved.
struct Item {
    path: PathBuf,
    /// Relative to the folder being moved.
    rel: PathBuf,
    size: u64,
    junk: bool,
}

/// What is below a folder.
#[derive(Default)]
struct Tree {
    /// In path order, so the move goes folder by folder.
    files: Vec<Item>,
    /// All subfolders, relative.
    folders: Vec<PathBuf>,
    /// Entries that could not be read, and subfolders whose files could not be listed; they
    /// stay where they are: (path, reason).
    unreadable: Vec<(PathBuf, String)>,
}

/// Everything below `root`, so the totals are known before anything moves. Symbolic links,
/// junctions and special files are skipped. Fails only if `root` itself can't be read.
/// The entry's own metadata is used: on Windows it comes with the listing, while opening the
/// path again fails for names Win32 changes on the way (a trailing dot or space).
fn scan(root: &Path) -> io::Result<Tree> {
    let mut tree = Tree::default();
    let mut pending = vec![PathBuf::new()];
    while let Some(rel) = pending.pop() {
        let folder = below(root, &rel);
        let entries = match fs::read_dir(&folder) {
            Ok(entries) => entries,
            Err(e) if rel.as_os_str().is_empty() => return Err(e),
            Err(e) => {
                tree.unreadable.push((folder, format!("cannot read this folder: {e}")));
                continue;
            }
        };
        for entry in entries {
            let entry = match entry {
                Ok(entry) => entry,
                Err(e) => {
                    tree.unreadable.push((folder.clone(), format!("cannot read an entry in this folder: {e}")));
                    continue;
                }
            };
            let path = entry.path();
            if cfg!(windows) && unusable_on_windows(&entry.file_name().to_string_lossy()) {
                tree.unreadable.push((path, "its name ends in a dot or a space, which Windows can't handle".into()));
                continue;
            }
            let meta = match entry.metadata() {
                Ok(meta) => meta,
                Err(e) => {
                    tree.unreadable.push((path, format!("cannot read it: {e}")));
                    continue;
                }
            };
            let rel = rel.join(entry.file_name());
            if meta.is_dir() {
                tree.folders.push(rel.clone());
                pending.push(rel);
            } else if meta.is_file() {
                let junk = is_junk(&entry.file_name().to_string_lossy());
                tree.files.push(Item { path, rel, size: meta.len(), junk });
            }
        }
    }
    tree.files.sort_by(|a, b| a.rel.as_path().cmp(b.rel.as_path()));
    Ok(tree)
}

/// Win32 drops a trailing dot or space from a name (made by WSL, git or a Mac), so such a
/// file can't be opened, moved or removed by its path, nor synced by OneDrive.
fn unusable_on_windows(name: &str) -> bool {
    name.ends_with('.') || name.ends_with(' ')
}

/// Number and size of the files that count (junk doesn't).
fn totals(files: &[Item]) -> (u64, u64) {
    files.iter().filter(|f| !f.junk).fold((0, 0), |(n, bytes), f| (n + 1, bytes + f.size))
}

/// `base` joined with `rel`, without a trailing separator when `rel` is empty.
fn below(base: &Path, rel: &Path) -> PathBuf {
    if rel.as_os_str().is_empty() { base.to_path_buf() } else { base.join(rel) }
}

/// `rel` with '/' between its parts, as shown while moving.
fn slashed(rel: &Path) -> String {
    rel.components().map(|c| c.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/")
}

/// How a file that didn't have to stay where it is was dealt with.
enum Moved {
    /// Under its own name.
    Here,
    /// Under another name, because a different file of that name was there.
    Renamed(PathBuf),
    /// The same file was there already; this one went to the Recycle Bin.
    Duplicate,
    /// It disappeared before it was its turn.
    Gone,
}

/// Move `source` into `dir` without replacing anything there. `Err` is the reason it stays
/// where it is.
fn move_one(source: &Path, dir: &Path) -> Result<Moved, String> {
    fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    // A PDF viewer that still shows the file may save annotations into it later; moving it
    // away from under the viewer would lose them.
    match sorter::retry_while_in_use(PATIENCE, || sorter::ensure_not_open_elsewhere(source)) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Moved::Gone),
        Err(e) if sorter::in_use(&e) => return Err(IN_USE.to_string()),
        Err(e) => return Err(e.to_string()),
    }
    let name = file_name(source)?;
    let existing = dir.join(name);
    if let Ok(meta) = fs::symlink_metadata(&existing) {
        if same_file(source, &existing) {
            return Err(format!("{} is this very file (the folders are linked)", existing.display()));
        }
        if meta.is_file() {
            // Not downloadable (offline, a second computer): unread, it would look different
            // and the same file would be saved again as "name (1)" on every device.
            let same_size = fs::metadata(source).is_ok_and(|m| m.len() == meta.len());
            if same_size && fetch_if_cloud_only(&existing).is_err() {
                return Err(format!(
                    "{} is only in the cloud and could not be downloaded to compare",
                    existing.display()
                ));
            }
            if sorter::same_content(source, &existing) {
                return match sorter::retry_while_in_use(PATIENCE, || sorter::discard(source)) {
                    Ok(()) => Ok(Moved::Duplicate),
                    Err(e) if sorter::in_use(&e) => Err(IN_USE.to_string()),
                    Err(e) => Err(e.to_string()),
                };
            }
            if meta.len() == 0 && fs::symlink_metadata(temp_beside(&existing)).is_ok() {
                // The name an earlier run claimed before it was stopped while copying (quit,
                // crash): filled now, so the file doesn't end up as "name (1)" beside an empty one.
                log::info!("Filling {}, claimed by a move that was stopped", existing.display());
                let _ = fs::remove_file(temp_beside(&existing));
                move_onto(source, &existing, PATIENCE)?;
                return Ok(Moved::Here);
            }
        }
    }
    let target =
        sorter::reserve_destination(dir, name).map_err(|e| format!("cannot write to {}: {e}", dir.display()))?;
    move_onto(source, &target, PATIENCE)?;
    Ok(if target == existing { Moved::Here } else { Moved::Renamed(target) })
}

/// Junk is moved when `to` has none of that name, otherwise put in the Recycle Bin.
fn move_junk(source: &Path, dir: &Path) -> Result<(), String> {
    // Office lock files belong to documents that are open right now.
    sorter::ensure_not_open_elsewhere(source).map_err(|e| e.to_string())?;
    fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let name = file_name(source)?;
    let existing = dir.join(name);
    if fs::symlink_metadata(&existing).is_ok() {
        if same_file(source, &existing) {
            return Err("it is already there (the folders are linked)".to_string());
        }
        return sorter::discard(source).map_err(|e| e.to_string());
    }
    let target = sorter::reserve_destination(dir, name).map_err(|e| e.to_string())?;
    if target != existing {
        // The name was taken in the meantime; junk isn't worth a second name.
        sorter::release_placeholder(&target);
        return Err("its name was taken in the meantime".to_string());
    }
    move_onto(source, &target, Duration::ZERO)
}

/// The file's name as text (`reserve_destination` needs it). A name that isn't valid
/// Unicode would arrive changed, so such a file stays where it is.
fn file_name(path: &Path) -> Result<&str, String> {
    path.file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| "its name has characters that cannot be handled".to_string())
}

/// Move `source` onto `target`, a name claimed with `reserve_destination` (an empty
/// placeholder), keeping its modification time. On failure `source` is as it was and the
/// name is given up again.
fn move_onto(source: &Path, target: &Path, patience: Duration) -> Result<(), String> {
    let result = transfer(source, target, patience);
    if result.is_err() {
        sorter::release_placeholder(target);
    }
    result
}

fn transfer(source: &Path, target: &Path, patience: Duration) -> Result<(), String> {
    let modified = fs::metadata(source).and_then(|m| m.modified()).ok();
    fetch_if_cloud_only(source).map_err(|e| e.to_string())?;
    still_ours(target)?;
    // Same drive: instant and atomic; it replaces nothing but the empty placeholder. Not
    // from under a viewer that opened the file while it was compared or downloaded.
    let rename = || sorter::ensure_not_open_elsewhere(source).and_then(|()| fs::rename(source, target));
    match sorter::retry_while_in_use(patience, rename) {
        Ok(()) => keep_modified(target, modified),
        Err(e) if sorter::in_use(&e) => return Err(IN_USE.to_string()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Err(e.to_string()),
        // Another drive (or a rename the file system refuses).
        Err(_) => copy_onto(source, target, patience, modified)?,
    }
    Ok(())
}

/// Across drives: copy `source` to a temporary file beside `target`, check it, write it to
/// the disk, rename it onto `target`, then remove `source`. Killed at any moment, the real
/// name holds either the empty placeholder or the complete file, and `source` goes only
/// once its copy is safe. If `source` can't be removed, the copy is removed again.
///
/// From the check that the copy is complete until `source` is removed, `source` is held so
/// that no other program can open it (Windows): a viewer that opened it during a long copy,
/// allowing deletion, would otherwise save its notes into a file about to vanish for good.
fn copy_onto(source: &Path, target: &Path, patience: Duration, modified: Option<SystemTime>) -> Result<(), String> {
    let temp = temp_beside(target);
    // A copy cut off by an earlier crash; Windows won't copy over it if it is read-only.
    let _ = fs::remove_file(&temp);
    let before = stamp(source);
    let unchanged = |held: &fs::File| before.is_some() && held.metadata().ok().and_then(|m| stamp_of(&m)) == before;
    let placed = sorter::copy_file_with_patience(source, &temp, patience)
        .map_err(|e| match e {
            MoveError::InUse(_) => IN_USE.to_string(),
            MoveError::Failed(e) => e.to_string(),
        })
        .and_then(|()| {
            let held = hold(source, patience).map_err(|e| reason_for(&e))?;
            // Written to while it was copied: the copy may be a mix of two versions.
            if !unchanged(&held) {
                return Err(CHANGED.to_string());
            }
            keep_modified(&temp, modified);
            flush(&temp)?;
            still_ours(target)?;
            sorter::retry_while_in_use(patience, || fs::rename(&temp, target)).map_err(|e| e.to_string())?;
            Ok(held)
        });
    let held = match placed {
        Ok(held) => held,
        Err(reason) => {
            let _ = fs::remove_file(&temp);
            return Err(reason);
        }
    };
    keep_name(target);
    // Checked once more for where nothing stops other programs from writing (not Windows).
    let removed = if unchanged(&held) { remove_held(held, source) } else { Err(io::Error::other(CHANGED)) };
    match removed {
        Ok(()) => Ok(()),
        // Removed by someone else in the meantime: the copy is the only one now, so it stays.
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => {
            let reason = reason_for(&e);
            // The cloud app may be reading the new file to upload it.
            match sorter::retry_while_in_use(patience, || fs::remove_file(target)) {
                Ok(()) => Err(reason),
                Err(e) => {
                    log::info!("Could not remove the copy {} again: {e}", target.display());
                    Err(format!("{reason}; it is also at {}", target.display()))
                }
            }
        }
    }
}

/// Why a copy isn't used: the original was written to while it was copied.
const CHANGED: &str = "it changed while it was being copied";

/// The reason a file stays, for an error.
fn reason_for(e: &io::Error) -> String {
    if sorter::in_use(e) { IN_USE.to_string() } else { e.to_string() }
}

/// Like `cloud::stamp`, from metadata already read.
fn stamp_of(meta: &fs::Metadata) -> Option<Stamp> {
    let modified = meta.modified().ok()?.duration_since(UNIX_EPOCH).ok()?.as_nanos() as u64;
    meta.is_file().then_some(Stamp { size: meta.len(), modified })
}

/// Open `path` so that no other program can open it until the handle is closed (Windows;
/// elsewhere files can't be locked, so it is only opened). Waits up to `patience` for
/// programs that have it open now.
fn hold(path: &Path, patience: Duration) -> io::Result<fs::File> {
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        use windows_sys::Win32::Storage::FileSystem::{DELETE, FILE_READ_ATTRIBUTES};
        let open = || fs::OpenOptions::new().access_mode(DELETE | FILE_READ_ATTRIBUTES).share_mode(0).open(path);
        sorter::retry_while_in_use(patience, open)
    }
    #[cfg(not(windows))]
    {
        let _ = patience;
        fs::File::open(path)
    }
}

/// Remove a file opened with `hold`. On Windows through the handle itself: closing it first
/// would let another program open the file before it is gone.
fn remove_held(held: fs::File, path: &Path) -> io::Result<()> {
    #[cfg(windows)]
    {
        let _ = path;
        delete_through(&held)
    }
    #[cfg(not(windows))]
    {
        drop(held);
        fs::remove_file(path)
    }
}

/// Mark the file of `file` for deletion; it is gone once `file` is closed. Read-only files
/// too (`fs::remove_file` removes those as well), where the drive supports it.
#[cfg(windows)]
fn delete_through(file: &fs::File) -> io::Result<()> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_DISPOSITION_FLAG_DELETE, FILE_DISPOSITION_FLAG_IGNORE_READONLY_ATTRIBUTE,
        FILE_DISPOSITION_FLAG_POSIX_SEMANTICS, FILE_DISPOSITION_INFO, FILE_DISPOSITION_INFO_EX, FileDispositionInfo,
        FileDispositionInfoEx, SetFileInformationByHandle,
    };
    let handle = file.as_raw_handle();
    let ex = FILE_DISPOSITION_INFO_EX {
        Flags: FILE_DISPOSITION_FLAG_DELETE
            | FILE_DISPOSITION_FLAG_POSIX_SEMANTICS
            | FILE_DISPOSITION_FLAG_IGNORE_READONLY_ATTRIBUTE,
    };
    // SAFETY: `handle` is open for DELETE while `file` lives; `ex` outlives the call.
    let done = unsafe {
        SetFileInformationByHandle(handle, FileDispositionInfoEx, (&raw const ex).cast(), size_of_val(&ex) as u32)
    };
    if done != 0 {
        return Ok(());
    }
    // FAT and exFAT drives (a USB stick) only know the older way.
    let plain = FILE_DISPOSITION_INFO { DeleteFile: true.into() };
    // SAFETY: as above.
    let done = unsafe {
        SetFileInformationByHandle(handle, FileDispositionInfo, (&raw const plain).cast(), size_of_val(&plain) as u32)
    };
    if done != 0 { Ok(()) } else { Err(io::Error::last_os_error()) }
}

/// The temporary name a copy is written to before it gets its real name. It is junk to all
/// cloud code, so a copy cut off half-way is never taken for a real file.
fn temp_beside(target: &Path) -> PathBuf {
    let mut name = target.file_name().unwrap_or_default().to_os_string();
    name.push(TEMP_SUFFIX);
    target.with_file_name(name)
}

/// The claimed name must still hold our empty placeholder: if something else was put there
/// in the meantime (a cloud app downloading a file of that name), it is not replaced.
fn still_ours(target: &Path) -> Result<(), String> {
    match fs::symlink_metadata(target) {
        Ok(meta) if meta.is_file() && meta.len() == 0 => Ok(()),
        _ => Err(format!("{} was taken by another file in the meantime", target.display())),
    }
}

/// Download a file that is only in the cloud (moving the study folder back out of
/// OneDrive): moved as it is, it might arrive without its content.
fn fetch_if_cloud_only(path: &Path) -> io::Result<()> {
    if !is_placeholder(path) {
        return Ok(());
    }
    io::copy(&mut fs::File::open(path)?, &mut io::sink())?;
    if is_placeholder(path) {
        return Err(io::Error::other("it is only in the cloud and could not be downloaded"));
    }
    Ok(())
}

/// A file just copied or moved here, opened for finishing touches: with write access where
/// possible (Windows needs it), else for reading (enough on Linux and macOS).
fn reopen(path: &Path) -> io::Result<fs::File> {
    fs::OpenOptions::new().write(true).open(path).or_else(|_| fs::File::open(path))
}

/// Give a file back the original's modification time where copying set it to now (Linux,
/// macOS): old lecture notes shouldn't look new. Failures are only logged.
fn keep_modified(path: &Path, modified: Option<SystemTime>) {
    let Some(modified) = modified else { return };
    if fs::metadata(path).and_then(|m| m.modified()).is_ok_and(|now| now == modified) {
        return;
    }
    if let Err(e) = reopen(path).and_then(|file| file.set_modified(modified)) {
        log::info!("Could not keep the modification time of {}: {e}", path.display());
    }
}

/// Make sure a copy is on the disk before the original is removed, so that after a power
/// cut the file is still complete in one of the two places.
fn flush(path: &Path) -> Result<(), String> {
    match reopen(path).and_then(|file| file.sync_all()) {
        Ok(()) => Ok(()),
        // Windows flushes only with write access, which a read-only file doesn't give.
        Err(e) if sorter::in_use(&e) => {
            log::info!("Could not make sure {} is on the disk: {e}", path.display());
            Ok(())
        }
        Err(e) => Err(format!("the copy could not be saved: {e}")),
    }
}

/// Make sure the copy's new name is on the disk before the original (on another drive, with
/// its own bookkeeping) is removed; otherwise a power cut could leave only the temporary
/// name. With NTFS flushing the file also writes its name; elsewhere the folder is flushed.
fn keep_name(path: &Path) {
    if let Err(reason) = flush(path) {
        log::info!("{}: {reason}", path.display());
    }
    #[cfg(unix)]
    if let Some(dir) = path.parent()
        && let Err(e) = fs::File::open(dir).and_then(|dir| dir.sync_all())
    {
        log::info!("Could not make sure {} is on the disk: {e}", dir.display());
    }
}

/// True if both paths lead to the very same file, e.g. through a linked folder: discarding
/// one as the other's duplicate would delete the file itself.
fn same_file(a: &Path, b: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if let (Ok(ma), Ok(mb)) = (fs::metadata(a), fs::metadata(b))
            && (ma.dev(), ma.ino()) == (mb.dev(), mb.ino())
        {
            return true;
        }
    }
    matches!((a.canonicalize(), b.canonicalize()), (Ok(x), Ok(y)) if x == y)
}

/// Bring folders that are empty now (or were all along: modules without files yet) over to
/// `to` and remove them in `from`, deepest first, then `from` itself if empty. A folder that
/// still holds something stays.
fn take_empty_folders(from: &Path, to: &Path, folders: &[PathBuf]) {
    let mut folders: Vec<&PathBuf> = folders.iter().collect();
    folders.sort_by_key(|rel| std::cmp::Reverse(rel.components().count()));
    for rel in folders {
        let here = from.join(rel);
        let empty = fs::read_dir(&here).is_ok_and(|mut entries| entries.next().is_none());
        if empty && fs::create_dir_all(to.join(rel)).is_ok() {
            let _ = fs::remove_dir(&here);
        }
    }
    let _ = fs::remove_dir(from);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, content: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }

    fn read(path: &Path) -> String {
        fs::read_to_string(path).unwrap()
    }

    fn set_time(path: &Path, time: SystemTime) {
        fs::OpenOptions::new().write(true).open(path).unwrap().set_modified(time).unwrap();
    }

    fn time_of(path: &Path) -> SystemTime {
        fs::metadata(path).unwrap().modified().unwrap()
    }

    /// Every file below `root` as ("folder/name", content), sorted; links as "<link>".
    fn files_below(root: &Path) -> Vec<(String, String)> {
        fn walk(dir: &Path, prefix: &str, out: &mut Vec<(String, String)>) {
            for entry in fs::read_dir(dir).unwrap().flatten() {
                let name = format!("{prefix}{}", entry.file_name().to_string_lossy());
                let meta = fs::symlink_metadata(entry.path()).unwrap();
                if meta.is_symlink() {
                    out.push((name, "<link>".into()));
                } else if meta.is_dir() {
                    walk(&entry.path(), &format!("{name}/"), out);
                } else {
                    out.push((name, read(&entry.path())));
                }
            }
        }
        let mut out = Vec::new();
        if root.exists() {
            walk(root, "", &mut out);
        }
        out.sort();
        out
    }

    fn pairs(list: &[(&str, &str)]) -> Vec<(String, String)> {
        list.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect()
    }

    fn folders(tmp: &tempfile::TempDir) -> (PathBuf, PathBuf) {
        (tmp.path().join("Studium"), tmp.path().join("OneDrive").join("Studium"))
    }

    fn run(from: &Path, to: &Path) -> Report {
        relocate(from, to, &AtomicBool::new(false), &mut |_: &Progress| {}).unwrap()
    }

    #[test]
    fn moves_a_nested_tree_and_removes_the_old_folder() {
        let tmp = tempfile::tempdir().unwrap();
        let (from, to) = folders(&tmp);
        let tree = [
            ("Info/Prog 1/blatt1.pdf", "b1"),
            ("Info/Prog 1/skript.pdf", "skript"),
            ("Mathe/Analysis/Übung 3.pdf", "ü3"),
            ("leer.txt", ""),
            ("notes.md", "notes"),
        ];
        for (rel, content) in tree {
            write(&from.join(rel), content);
        }
        fs::create_dir_all(from.join("Physik/Optik")).unwrap(); // a module without files yet

        let report = run(&from, &to);
        assert_eq!(report, Report { moved: 5, ..Report::default() });
        assert_eq!(files_below(&to), pairs(&tree));
        assert!(to.join("Physik/Optik").is_dir(), "empty module folders come along");
        assert!(!from.exists(), "the old study folder is removed");
        assert!(tmp.path().join("OneDrive").is_dir(), "the target is never removed");
    }

    #[test]
    fn merges_into_an_existing_folder_without_touching_its_files() {
        let tmp = tempfile::tempdir().unwrap();
        let (from, to) = folders(&tmp);
        let old = UNIX_EPOCH + Duration::from_secs(1_400_000_000);
        write(&to.join("Mathe/from the laptop.pdf"), "cloud");
        write(&to.join("readme.txt"), "cloud readme");
        set_time(&to.join("readme.txt"), old);
        write(&from.join("Mathe/blatt.pdf"), "local");
        write(&from.join("Info/prog.pdf"), "local prog");

        let report = run(&from, &to);
        assert_eq!(report, Report { moved: 2, ..Report::default() });
        let expected = [
            ("Info/prog.pdf", "local prog"),
            ("Mathe/blatt.pdf", "local"),
            ("Mathe/from the laptop.pdf", "cloud"),
            ("readme.txt", "cloud readme"),
        ];
        assert_eq!(files_below(&to), pairs(&expected));
        assert_eq!(time_of(&to.join("readme.txt")), old, "existing files are not touched");
        assert!(!from.exists());
    }

    #[test]
    fn same_names_are_never_overwritten() {
        let tmp = tempfile::tempdir().unwrap();
        let (from, to) = folders(&tmp);
        // Identical: a duplicate. Different: saved beside it, under the next free number.
        write(&from.join("Mathe/same.pdf"), "same");
        write(&to.join("Mathe/same.pdf"), "same");
        write(&from.join("Mathe/skript.pdf"), "original");
        write(&to.join("Mathe/skript.pdf"), "annotated");
        write(&from.join("Mathe/x.pdf"), "third");
        write(&to.join("Mathe/x.pdf"), "first");
        write(&to.join("Mathe/x (1).pdf"), "second");
        // A folder where the file would go.
        write(&from.join("Info/blatt.pdf"), "file");
        write(&to.join("Info/blatt.pdf/inside.txt"), "folder");

        let report = run(&from, &to);
        let renamed = vec![
            (from.join("Info/blatt.pdf"), to.join("Info/blatt (1).pdf")),
            (from.join("Mathe/skript.pdf"), to.join("Mathe/skript (1).pdf")),
            (from.join("Mathe/x.pdf"), to.join("Mathe/x (2).pdf")),
        ];
        assert_eq!(report, Report { moved: 3, duplicates: 1, renamed, ..Report::default() });
        let expected = [
            ("Info/blatt (1).pdf", "file"),
            ("Info/blatt.pdf/inside.txt", "folder"),
            ("Mathe/same.pdf", "same"),
            ("Mathe/skript (1).pdf", "original"),
            ("Mathe/skript.pdf", "annotated"),
            ("Mathe/x (1).pdf", "second"),
            ("Mathe/x (2).pdf", "third"),
            ("Mathe/x.pdf", "first"),
        ];
        assert_eq!(files_below(&to), pairs(&expected));
        assert!(!from.exists(), "the duplicate went to the Recycle Bin");
    }

    #[test]
    fn refuses_before_anything_moves() {
        let tmp = tempfile::tempdir().unwrap();
        let a = tmp.path().join("Studium");
        write(&a.join("Mathe/blatt.pdf"), "x");
        let file = tmp.path().join("a file.txt");
        write(&file, "not a folder");
        let kind = |result: Result<Report, RelocateError>| match result {
            Ok(_) => "ok",
            Err(RelocateError::Same) => "same",
            Err(RelocateError::Nested) => "nested",
            Err(RelocateError::SourceMissing) => "missing",
            Err(RelocateError::Io(_)) => "io",
        };
        let cases = [
            (a.clone(), a.clone(), "same"),
            (a.clone(), a.join("..").join("Studium"), "same"),
            (a.clone(), a.join("Cloud").join("Studium"), "nested"),
            (a.join("Mathe"), a.clone(), "nested"),
            (tmp.path().join("missing"), tmp.path().join("Cloud").join("Studium"), "missing"),
            (file.clone(), tmp.path().join("Cloud").join("Studium"), "missing"),
            (a.clone(), file.clone(), "io"),
            (a.clone(), file.join("Studium"), "io"),
        ];
        for (from, to, expected) in cases {
            let result = relocate(&from, &to, &AtomicBool::new(false), &mut |_: &Progress| {});
            assert_eq!(kind(result), expected, "{} → {}", from.display(), to.display());
        }
        assert_eq!(files_below(&a), pairs(&[("Mathe/blatt.pdf", "x")]), "nothing moved");
        assert!(!a.join("Cloud").exists() && !tmp.path().join("Cloud").exists(), "no folder made");
        assert_eq!(read(&file), "not a folder");
    }

    /// `to` doesn't exist yet and leads into `from` through a link: only noticed once it is
    /// made, and then made undone.
    #[cfg(unix)]
    #[test]
    fn a_target_inside_through_a_link_is_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let from = tmp.path().join("Studium");
        write(&from.join("blatt.pdf"), "x");
        std::os::unix::fs::symlink(&from, tmp.path().join("alias")).unwrap();
        let to = tmp.path().join("alias/Cloud/Studium");
        let result = relocate(&from, &to, &AtomicBool::new(false), &mut |_: &Progress| {});
        assert!(matches!(result, Err(RelocateError::Nested)), "{result:?}");
        assert_eq!(files_below(&from), pairs(&[("blatt.pdf", "x")]));
        assert!(!from.join("Cloud").exists(), "the folders made for `to` are removed again");
    }

    #[test]
    fn cancelling_leaves_the_rest_untouched() {
        let tmp = tempfile::tempdir().unwrap();
        let (from, to) = folders(&tmp);
        for rel in ["a.pdf", "b.pdf", "sub/c.pdf"] {
            write(&from.join(rel), rel);
        }
        let cancel = AtomicBool::new(false);
        let mut seen = Vec::new();
        let report = relocate(&from, &to, &cancel, &mut |p: &Progress| {
            seen.push(p.clone());
            if p.done == 1 {
                cancel.store(true, Ordering::Relaxed);
            }
        })
        .unwrap();
        assert_eq!(report, Report { moved: 1, cancelled: true, ..Report::default() });
        assert_eq!(files_below(&to), pairs(&[("a.pdf", "a.pdf")]));
        assert_eq!(files_below(&from), pairs(&[("b.pdf", "b.pdf"), ("sub/c.pdf", "sub/c.pdf")]));
        let last = seen.last().unwrap();
        assert_eq!((last.done, last.total, last.current.as_str()), (1, 3, ""));

        // Cancelled before the first file: nothing moves, the (empty) folders stay too.
        let (from, to) = (tmp.path().join("Zwei"), tmp.path().join("Cloud/Zwei"));
        write(&from.join("a.pdf"), "a");
        fs::create_dir_all(from.join("Leer")).unwrap();
        let report = relocate(&from, &to, &AtomicBool::new(true), &mut |_: &Progress| {}).unwrap();
        assert_eq!(report, Report { cancelled: true, ..Report::default() });
        assert_eq!(files_below(&from), pairs(&[("a.pdf", "a")]));
        assert!(from.join("Leer").is_dir());
    }

    #[test]
    fn progress_and_plan_count_files_but_not_junk() {
        let tmp = tempfile::tempdir().unwrap();
        let (from, to) = folders(&tmp);
        write(&from.join("a.txt"), "1");
        write(&from.join("sub/b.txt"), "22");
        write(&from.join("sub/deeper/c.txt"), "333");
        write(&from.join("sub/Thumbs.db"), "junk");
        write(&from.join("sub/blatt.pdf.crdownload"), "junk too");
        assert_eq!(plan(&from), Plan { files: 3, bytes: 6 });
        assert_eq!(plan(&tmp.path().join("missing")), Plan::default());

        let mut seen = Vec::new();
        relocate(&from, &to, &AtomicBool::new(false), &mut |p: &Progress| seen.push(p.clone())).unwrap();
        let step = |done, bytes_done, current: &str| Progress {
            done,
            total: 3,
            bytes_done,
            bytes_total: 6,
            current: current.to_string(),
        };
        let expected =
            vec![step(0, 0, "a.txt"), step(1, 1, "sub/b.txt"), step(2, 3, "sub/deeper/c.txt"), step(3, 6, "")];
        assert_eq!(seen, expected);
    }

    #[test]
    fn junk_is_moved_once_and_otherwise_discarded() {
        let tmp = tempfile::tempdir().unwrap();
        let (from, to) = folders(&tmp);
        write(&from.join("desktop.ini"), "local icon");
        write(&from.join("Mathe/Thumbs.db"), "local thumbs");
        write(&from.join("Mathe/~$skript.docx"), "stale lock");
        write(&from.join("Mathe/skript.docx"), "doc");
        write(&from.join("Mathe/alt.pdf.filesorter-tmp"), "leftover");
        write(&to.join("Mathe/Thumbs.db"), "cloud thumbs");
        write(&to.join("Mathe/alt.pdf.filesorter-tmp"), "cloud leftover");

        let report = run(&from, &to);
        assert_eq!(report, Report { moved: 1, ..Report::default() }, "junk isn't counted");
        let expected = [
            ("Mathe/Thumbs.db", "cloud thumbs"),
            ("Mathe/alt.pdf.filesorter-tmp", "cloud leftover"),
            ("Mathe/skript.docx", "doc"),
            ("Mathe/~$skript.docx", "stale lock"),
            ("desktop.ini", "local icon"),
        ];
        assert_eq!(files_below(&to), pairs(&expected));
        assert!(!from.exists());
    }

    #[test]
    fn only_empty_folders_are_removed() {
        let tmp = tempfile::tempdir().unwrap();
        let (from, to) = folders(&tmp);
        write(&from.join("Mathe/blatt.pdf"), "x");
        write(&from.join("Notizen/n.txt"), "stays");
        write(&to.join("Notizen"), "a file, not a folder");
        fs::create_dir_all(from.join("Leer/Tiefer")).unwrap();

        let report = run(&from, &to);
        assert_eq!(report.moved, 1);
        assert_eq!(report.left.len(), 1, "{report:?}");
        assert_eq!(report.left[0].0, from.join("Notizen").join("n.txt"));
        assert_eq!(files_below(&from), pairs(&[("Notizen/n.txt", "stays")]));
        assert!(!from.join("Mathe").exists() && !from.join("Leer").exists());
        assert!(to.join("Leer/Tiefer").is_dir());
        assert_eq!(read(&to.join("Notizen")), "a file, not a folder");
    }

    #[test]
    fn modification_times_are_kept() {
        let tmp = tempfile::tempdir().unwrap();
        let (from, to) = folders(&tmp);
        let old = UNIX_EPOCH + Duration::from_secs(1_500_000_000);
        let file = from.join("Mathe/alt.pdf");
        write(&file, "old notes");
        set_time(&file, old);
        run(&from, &to);
        assert_eq!(time_of(&to.join("Mathe/alt.pdf")), old);

        // Where copying set the time to now, it is set back.
        let copy = tmp.path().join("copy.pdf");
        write(&copy, "copied just now");
        keep_modified(&copy, Some(old));
        assert_eq!(time_of(&copy), old);
        assert_eq!(read(&copy), "copied just now");
    }

    /// The way across drives, called directly (a temporary folder is on one drive).
    #[test]
    fn copying_across_drives_puts_the_whole_file_in_one_place() {
        let tmp = tempfile::tempdir().unwrap();
        let old = UNIX_EPOCH + Duration::from_secs(1_500_000_000);
        let source = tmp.path().join("here/skript.pdf");
        write(&source, "annotated notes");
        set_time(&source, old);
        let dir = tmp.path().join("cloud");
        write(&dir.join("skript.pdf.filesorter-tmp"), "a copy cut off by a crash before");
        let target = sorter::reserve_destination(&dir, "skript.pdf").unwrap();

        copy_onto(&source, &target, Duration::ZERO, Some(old)).unwrap();
        assert!(!source.exists());
        assert_eq!(files_below(&dir), pairs(&[("skript.pdf", "annotated notes")]), "no temporary file left");
        assert_eq!(time_of(&target), old);
    }

    /// The claimed name was taken by another file (a cloud app downloaded one of that name):
    /// it is not replaced, and the file stays where it is.
    #[test]
    fn a_name_taken_in_the_meantime_is_not_replaced() {
        let tmp = tempfile::tempdir().unwrap();
        let source = tmp.path().join("here/skript.pdf");
        write(&source, "mine");
        let dir = tmp.path().join("cloud");
        let target = dir.join("skript.pdf");
        write(&target, "from the laptop");
        let both = || (files_below(&tmp.path().join("here")), files_below(&dir));
        let unchanged = (pairs(&[("skript.pdf", "mine")]), pairs(&[("skript.pdf", "from the laptop")]));

        assert!(copy_onto(&source, &target, Duration::ZERO, None).is_err());
        assert_eq!(both(), unchanged, "copying across drives");
        assert!(move_onto(&source, &target, Duration::ZERO).is_err());
        assert_eq!(both(), unchanged, "renaming");
    }

    /// A real second drive where there is one (`/dev/shm` is a RAM disk on Linux).
    #[cfg(target_os = "linux")]
    #[test]
    fn moves_to_another_drive() {
        use std::os::unix::fs::MetadataExt;
        let tmp = tempfile::tempdir().unwrap();
        let Ok(other) = tempfile::tempdir_in("/dev/shm") else { return };
        if fs::metadata(tmp.path()).unwrap().dev() == fs::metadata(other.path()).unwrap().dev() {
            return;
        }
        let from = tmp.path().join("Studium");
        let to = other.path().join("Studium");
        let old = UNIX_EPOCH + Duration::from_secs(1_500_000_000);
        write(&from.join("Mathe/alt.pdf"), "old");
        set_time(&from.join("Mathe/alt.pdf"), old);
        write(&from.join("Mathe/same.pdf"), "same");
        write(&to.join("Mathe/same.pdf"), "same");
        write(&from.join("Info/skript.pdf"), "mine");
        write(&to.join("Info/skript.pdf"), "theirs");

        let report = run(&from, &to);
        let renamed = vec![(from.join("Info/skript.pdf"), to.join("Info/skript (1).pdf"))];
        assert_eq!(report, Report { moved: 2, duplicates: 1, renamed, ..Report::default() });
        let expected = [
            ("Info/skript (1).pdf", "mine"),
            ("Info/skript.pdf", "theirs"),
            ("Mathe/alt.pdf", "old"),
            ("Mathe/same.pdf", "same"),
        ];
        assert_eq!(files_below(&to), pairs(&expected));
        assert_eq!(time_of(&to.join("Mathe/alt.pdf")), old);
        assert!(!from.exists());
    }

    /// A run stopped while copying across drives (quit, crash) left the claimed name empty
    /// with half a copy beside it: the next run fills that name, not "name (1)". An empty
    /// file without a temporary file beside it is the user's and stays.
    #[test]
    fn a_name_claimed_by_a_stopped_run_is_filled() {
        let tmp = tempfile::tempdir().unwrap();
        let (from, to) = folders(&tmp);
        write(&from.join("Mathe/Vorlesung.mp4"), "the whole video");
        write(&to.join("Mathe/Vorlesung.mp4"), "");
        write(&to.join("Mathe/Vorlesung.mp4.filesorter-tmp"), "half of it");
        write(&from.join("Mathe/leer.txt"), "notes");
        write(&to.join("Mathe/leer.txt"), "");

        let report = run(&from, &to);
        let renamed = vec![(from.join("Mathe/leer.txt"), to.join("Mathe/leer (1).txt"))];
        assert_eq!(report, Report { moved: 2, renamed, ..Report::default() });
        let expected =
            [("Mathe/Vorlesung.mp4", "the whole video"), ("Mathe/leer (1).txt", "notes"), ("Mathe/leer.txt", "")];
        assert_eq!(files_below(&to), pairs(&expected));
    }

    #[test]
    fn names_windows_cannot_handle() {
        for name in ["Notizen.", "Blatt 3 .pdf ", "Ordner "] {
            assert!(unusable_on_windows(name), "{name}");
        }
        for name in ["Notizen", "Blatt 3 .pdf", ".hidden", "a.b"] {
            assert!(!unusable_on_windows(name), "{name}");
        }
    }

    #[test]
    fn temporary_names_are_junk() {
        let temp = temp_beside(Path::new("cloud/skript (1).pdf"));
        assert_eq!(temp, Path::new("cloud/skript (1).pdf.filesorter-tmp"));
        assert!(is_junk(&temp.file_name().unwrap().to_string_lossy()));
    }

    #[cfg(unix)]
    #[test]
    fn links_are_left_alone() {
        use std::os::unix::fs::symlink;
        let tmp = tempfile::tempdir().unwrap();
        let (from, to) = folders(&tmp);
        let outside = tmp.path().join("Elsewhere");
        write(&outside.join("secret.pdf"), "outside");
        write(&from.join("Mathe/blatt.pdf"), "x");
        symlink(outside.join("secret.pdf"), from.join("Mathe/link.pdf")).unwrap();
        symlink(&outside, from.join("Elsewhere")).unwrap();
        assert_eq!(plan(&from), Plan { files: 1, bytes: 1 });

        let report = run(&from, &to);
        assert_eq!(report, Report { moved: 1, ..Report::default() }, "links are not errors");
        assert_eq!(files_below(&to), pairs(&[("Mathe/blatt.pdf", "x")]));
        assert_eq!(files_below(&from), pairs(&[("Elsewhere", "<link>"), ("Mathe/link.pdf", "<link>")]));
        assert_eq!(files_below(&outside), pairs(&[("secret.pdf", "outside")]));
    }

    /// The cloud folder links back into the study folder (a trick to sync any folder): the
    /// file there is the file itself, never its duplicate.
    #[cfg(unix)]
    #[test]
    fn a_file_reached_through_a_linked_folder_is_never_discarded() {
        let tmp = tempfile::tempdir().unwrap();
        let (from, to) = folders(&tmp);
        write(&from.join("Mathe/skript.pdf"), "my notes");
        write(&from.join("Mathe/Thumbs.db"), "thumbs");
        fs::create_dir_all(&to).unwrap();
        std::os::unix::fs::symlink(from.join("Mathe"), to.join("Mathe")).unwrap();

        let report = run(&from, &to);
        assert_eq!((report.moved, report.duplicates, report.left.len()), (0, 0, 1), "{report:?}");
        assert_eq!(files_below(&from), pairs(&[("Mathe/Thumbs.db", "thumbs"), ("Mathe/skript.pdf", "my notes")]));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_name_that_is_not_unicode_stays() {
        use std::os::unix::ffi::OsStrExt;
        let tmp = tempfile::tempdir().unwrap();
        let (from, to) = folders(&tmp);
        let name = std::ffi::OsStr::from_bytes(b"bl\xe4tt.pdf");
        write(&from.join(name), "latin-1 name");
        let report = run(&from, &to);
        assert_eq!(report.left.len(), 1, "{report:?}");
        assert_eq!(read(&from.join(name)), "latin-1 name");
    }

    /// A file open in another program (a PDF viewer) stays where it is, so notes saved into
    /// it later aren't lost; the rest moves.
    #[cfg(windows)]
    #[test]
    fn a_file_open_elsewhere_stays_and_is_listed() {
        let tmp = tempfile::tempdir().unwrap();
        let (from, to) = folders(&tmp);
        for rel in ["Mathe/a.pdf", "Mathe/open.pdf", "Info/b.pdf"] {
            write(&from.join(rel), rel);
        }
        let open = from.join("Mathe").join("open.pdf");
        let viewer = fs::File::open(&open).unwrap();

        let report = run(&from, &to);
        assert_eq!(report.moved, 2);
        assert_eq!(report.left, vec![(open.clone(), IN_USE.to_string())]);
        assert_eq!(read(&open), "Mathe/open.pdf");
        assert_eq!(files_below(&to), pairs(&[("Info/b.pdf", "Info/b.pdf"), ("Mathe/a.pdf", "Mathe/a.pdf")]));
        assert!(!from.join("Info").exists() && from.join("Mathe").is_dir());
        drop(viewer);
    }

    /// Across drives, the original is open in another program once it is copied: the copy is
    /// removed again, the file is in one place only. (The empty claim is given up by
    /// `move_onto`.)
    #[cfg(windows)]
    #[test]
    fn a_copy_is_removed_again_when_the_original_must_stay() {
        use std::os::windows::fs::OpenOptionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let source = tmp.path().join("here").join("skript.pdf");
        write(&source, "notes");
        let dir = tmp.path().join("cloud");
        fs::create_dir_all(&dir).unwrap();
        let target = sorter::reserve_destination(&dir, "skript.pdf").unwrap();
        let reader = fs::OpenOptions::new().read(true).share_mode(1 /* FILE_SHARE_READ only */).open(&source).unwrap();

        let result = copy_onto(&source, &target, Duration::from_millis(300), None);
        assert_eq!(result, Err(IN_USE.to_string()));
        drop(reader);
        assert_eq!(read(&source), "notes");
        assert_eq!(files_below(&dir), pairs(&[("skript.pdf", "")]), "neither the copy nor a temporary file is left");
    }

    /// A viewer opened the file while it was being moved, allowing others to delete it (as
    /// Edge and Acrobat do). It is neither removed under the viewer after copying, which would
    /// lose the notes it saves later, nor renamed away from under it.
    #[cfg(windows)]
    #[test]
    fn a_file_opened_while_moving_stays_with_its_viewer() {
        use std::os::windows::fs::OpenOptionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let source = tmp.path().join("here").join("skript.pdf");
        write(&source, "notes");
        let dir = tmp.path().join("cloud");
        fs::create_dir_all(&dir).unwrap();
        let viewer = fs::OpenOptions::new().read(true).share_mode(7).open(&source).unwrap();

        let target = sorter::reserve_destination(&dir, "skript.pdf").unwrap();
        assert_eq!(copy_onto(&source, &target, Duration::from_millis(300), None), Err(IN_USE.to_string()));
        assert_eq!(read(&source), "notes", "across drives");
        assert_eq!(files_below(&dir), pairs(&[("skript.pdf", "")]));

        assert_eq!(move_onto(&source, &target, Duration::from_millis(300)), Err(IN_USE.to_string()));
        assert_eq!(read(&source), "notes", "on the same drive");
        assert_eq!(files_below(&dir), Vec::new());
        drop(viewer);

        // Closed again: the file moves, through a handle no one else could open meanwhile.
        let target = sorter::reserve_destination(&dir, "skript.pdf").unwrap();
        copy_onto(&source, &target, Duration::ZERO, None).unwrap();
        assert!(!source.exists());
        assert_eq!(files_below(&dir), pairs(&[("skript.pdf", "notes")]));
    }

    /// Names Win32 can't open (a trailing dot or space, made by WSL or a Mac) stay, are listed
    /// and count as left at the old place.
    #[cfg(windows)]
    #[test]
    fn a_name_windows_cannot_open_stays_and_is_listed() {
        let tmp = tempfile::tempdir().unwrap();
        let (from, to) = folders(&tmp);
        write(&from.join("Mathe").join("blatt.pdf"), "x");
        let odd = from.join("Mathe").join("Notizen.");
        let verbatim = PathBuf::from(format!(r"\\?\{}", std::path::absolute(&odd).unwrap().display()));
        fs::write(&verbatim, "from WSL").unwrap();
        assert_eq!(plan(&from).files, 2);

        let report = run(&from, &to);
        assert_eq!(report.moved, 1);
        assert_eq!(report.left.len(), 1, "{report:?}");
        assert_eq!(report.left[0].0, odd);
        assert_eq!(fs::read_to_string(&verbatim).unwrap(), "from WSL");
        assert_eq!(plan(&from), Plan { files: 1, bytes: 0 }, "still counted as left there");
        fs::remove_file(&verbatim).unwrap();
    }
}
