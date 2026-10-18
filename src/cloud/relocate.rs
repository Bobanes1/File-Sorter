//! Moving the whole study folder into the cloud folder (or back), safely.
//!
//! CONTRACT (scaffold; to be implemented):
//! - Every file below `from` goes to the same relative place below `to`; folders are
//!   created as needed and merged with what is already there.
//! - Never overwrites: when `to` already has a different file of that name, the file is
//!   saved as "name (1).ext" (`sorter::reserve_destination`) and listed in `renamed`.
//! - A file whose identical twin (same content) is already there is not copied again; the
//!   one in `from` goes to the Recycle Bin (`sorter::discard`) and counts as a duplicate.
//! - Files another program has open (`sorter::ensure_not_open_elsewhere`) or that fail stay
//!   where they are and are listed in `left`; everything else carries on.
//! - Moving uses `sorter::move_file_with_patience` (rename, or copy + delete across drives,
//!   verified).
//! - Junk files (`cloud::is_junk`): moved if `to` has none, otherwise put in the Recycle Bin.
//! - Afterwards empty folders in `from` are removed, `from` itself too when empty.
//! - `cancel` is checked between files; a cancelled run leaves every file in one piece.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

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
    pub moved: u64,
    /// Already in the target with the same content; the one here went to the Recycle Bin.
    pub duplicates: u64,
    /// Saved under another name because a different file of that name was there: (from, to).
    pub renamed: Vec<(PathBuf, PathBuf)>,
    /// Not moved (open in another program, or an error): (file, reason). They stay put.
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
pub fn plan(from: &Path) -> Plan {
    let _ = from;
    todo!("scaffold")
}

/// Move everything in `from` into `to` (see the module comment). `progress` is called
/// before each file and once at the end.
pub fn relocate(
    from: &Path,
    to: &Path,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(&Progress),
) -> Result<Report, RelocateError> {
    let _ = (from, to, cancel, progress);
    todo!("scaffold")
}
