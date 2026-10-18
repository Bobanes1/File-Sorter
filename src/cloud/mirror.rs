//! A copy of the study folder inside the cloud folder, kept up to date one way
//! (this computer → cloud), without ever destroying a change made in the cloud.
//!
//! CONTRACT (scaffold; to be implemented):
//! - State per file (keyed by its path relative to the study folder, '/'-separated,
//!   lower-case on Windows): the stamp of the local file when it was last copied, the cloud
//!   copy's relative path (may differ, see conflicts), the cloud copy's stamp right after
//!   copying, and the content hash of what was copied (`cloud::content_hash`).
//! - A cloud copy counts as unchanged if its stamp is `roughly_eq` (2 s) to the saved one,
//!   or if size and content hash are those of what was copied.
//! - Per local file (junk and `shared::SHARED_FILE` skipped; files changed in the last 3 s or
//!   open in another program are skipped for now and counted in `in_use`):
//!   - not known, no cloud file → copy (claimed with `sorter::reserve_destination`).
//!   - not known, cloud file with the same content → adopt it (no copy).
//!   - not known, a different cloud file → conflict: copy as "name (<computer>).ext".
//!   - known, local unchanged → nothing (cloud changed: `changed_in_cloud`; cloud deleted:
//!     respected, not copied again).
//!   - known, local changed, cloud unchanged → replace the cloud copy (temp file + rename).
//!   - known, local changed, cloud missing → copy again.
//!   - known, local changed, cloud changed → conflict copy beside it; the cloud version stays.
//! - Deleting a local file never deletes its cloud copy; only [`Mirror::forget`] (undo) does,
//!   and only while nobody changed the cloud copy (to the Recycle Bin).
//! - When the cloud folder (the parent of `cloud`) is missing, nothing happens: `unreachable`.
//! - `sync_all`, `sync_file` and `forget` save the state themselves when they changed it.
//! - `sync_file` copies right away (no 3-second wait: the sorter just finished the file), but
//!   still not while another program has it open.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

/// The state file, in File Sorter's settings folder.
pub const MIRROR_STATE: &str = "cloud-mirror.json";

#[derive(Debug, Clone, Default, PartialEq)]
pub struct MirrorReport {
    /// New in the cloud.
    pub copied: u64,
    /// Cloud copy replaced by this computer's newer version.
    pub updated: u64,
    /// Changed in the cloud and here: this computer's version was saved beside it (cloud path).
    pub conflicts: Vec<PathBuf>,
    /// Changed only in the cloud (on another device): left as it is.
    pub changed_in_cloud: u64,
    /// Open in another program or just changed: tried again next time.
    pub in_use: u64,
    pub errors: Vec<(PathBuf, String)>,
    /// The cloud folder isn't there (drive not connected, cloud app not set up): nothing done.
    pub unreachable: bool,
    pub cancelled: bool,
}

/// The copy's state, loaded from and saved to the state file.
#[derive(Debug)]
pub struct Mirror {
    state_file: PathBuf,
}

impl Mirror {
    /// The state saved in `state_file` (empty if there is none yet or it can't be read).
    pub fn load(state_file: &Path) -> Mirror {
        Mirror { state_file: state_file.to_path_buf() }
    }

    /// Bring the copy in `cloud` up to date with everything below `local`.
    pub fn sync_all(&mut self, local: &Path, cloud: &Path, cancel: &AtomicBool) -> MirrorReport {
        let _ = (local, cloud, cancel);
        todo!("scaffold")
    }

    /// The same for one file below `local` (just sorted into it).
    pub fn sync_file(&mut self, local: &Path, cloud: &Path, file: &Path) -> MirrorReport {
        let _ = (local, cloud, file);
        todo!("scaffold")
    }

    /// `file` (below `local`) was taken out again (undo): put its cloud copy into the
    /// Recycle Bin unless it was changed there, and forget it. Returns whether a copy was removed.
    pub fn forget(&mut self, local: &Path, cloud: &Path, file: &Path) -> io::Result<bool> {
        let _ = (local, cloud, file);
        todo!("scaffold")
    }

    pub fn save(&self) -> io::Result<()> {
        let _ = &self.state_file;
        todo!("scaffold")
    }
}
