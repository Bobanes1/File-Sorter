//! A copy of the study folder inside the cloud folder, kept up to date one way
//! (this computer → cloud), without ever destroying a change made in the cloud.
//!
//! The cloud app brings the copy to the other devices, where it may be annotated, renamed or
//! deleted. So the copy is only ever written where that can't lose anything:
//! - Per copied file the state keeps (by its path relative to the study folder, '/'-separated,
//!   lower-case on Windows) the local file's stamp when it was copied, where its copy is (another
//!   name after a conflict), the copy's stamp right after copying and the content hash of what
//!   was copied (taken from the local file, so a file that is only in the cloud isn't downloaded).
//! - A cloud copy counts as unchanged if its stamp is within 2 s of the saved one (cloud apps
//!   round and shift times), or if size and content hash are those of what was copied. It is
//!   hashed only when the stamp differs: reading a file that is only in the cloud downloads it.
//! - Per local file (junk and [`SHARED_FILE`] skipped; files changed in the last 3 s or open in
//!   another program are left for the next pass and counted in `in_use`):
//!   - not known, no cloud file → copied.
//!   - not known, a cloud file with the same content → adopted (nothing copied).
//!   - not known, a different cloud file → conflict: copied beside it as "name (<computer>).ext".
//!   - known, unchanged here → nothing (changed in the cloud: counted in `changed_in_cloud`;
//!     deleted in the cloud: respected, not copied again).
//!   - known, but deleted here since (seen gone by a pass) or just sorted into the name (which
//!     was free, so the file there before is gone), and now holding something else → another
//!     file under the old name: its copy, perhaps the only one left of the old file, is never
//!     replaced by it; it goes beside it as a conflict.
//!   - known, changed here, cloud copy unchanged → the cloud copy is replaced.
//!   - known, changed here, cloud copy deleted → copied again.
//!   - known, changed here and in the cloud → conflict copy beside it, followed from then on;
//!     the cloud version stays as it is.
//! - New names are claimed with `sorter::reserve_destination`, so no file is ever overwritten by
//!   accident. Content goes into "<name>.filesorter-tmp" next to it first, is checked and then
//!   flushed to the disk and only then renamed into place, so neither a killed run nor a power
//!   cut leaves half a file under a real name. The temporary file is made writable first:
//!   `fs::copy` carries a read-only attribute over, and a read-only copy could never be replaced.
//! - Deleting a local file never deletes its cloud copy; only [`Mirror::forget`] (undo) does,
//!   only for copies this computer wrote (not adopted ones) and only while nobody changed the
//!   cloud copy (to the Recycle Bin). If that can't be done right away (the cloud app or a virus
//!   scanner has the copy open, the cloud folder is away), it is tried again on every pass.
//! - Only a sharing or lock violation counts as "in use"; "access denied" (Controlled Folder
//!   Access, missing rights) doesn't pass by itself and is reported as an error.
//! - When the cloud folder (the parent of `cloud`) is missing, nothing happens: `unreachable`.
//! - The state belongs to one study folder and one copy. For another pair, when the copy's
//!   folder is gone altogether (switched off, deleted, switched on again), or after the study
//!   folder was moved into or out of the cloud folder ([`Mirror::reset`]), it starts afresh:
//!   files already there with the same content are adopted, everything else is copied.
//! - `sync_all`, `sync_file` and `forget` save the state themselves when they changed it.
//! - `sync_file` copies right away (no 3-second wait: the sorter just finished the file), but
//!   still not while another program has it open.

use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::cloud::shared::SHARED_FILE;
use crate::cloud::{
    Stamp, TEMP_SUFFIX, computer_name, content_hash, is_inside, is_junk, is_placeholder, save_json, stamp,
};
use crate::sorter::{self, MoveError};

/// The state file, in File Sorter's settings folder.
pub const MIRROR_STATE: &str = "cloud-mirror.json";

/// Cloud apps round or shift modification times a little after uploading.
const SLACK_SECS: u64 = 2;
/// A file changed more recently than this may still be being written (`sync_all` only).
const SETTLE: Duration = Duration::from_secs(3);
/// How long to wait for the cloud app or a virus scanner to let go of a file; anything
/// longer is left for the next pass.
const PATIENCE: Duration = Duration::from_secs(2);
/// Temporary files older than this were left behind by a run that was killed.
const STALE: Duration = Duration::from_secs(3600);
/// A long pass saves the state now and then, so a killed run doesn't compare everything again.
const SAVE_EVERY: Duration = Duration::from_secs(20);

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

/// What is known about one copied file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Entry {
    /// The local file when it was last copied.
    local: Stamp,
    /// The copy, relative to the copy's folder ('/'-separated, its own case).
    cloud: String,
    /// The copy right after copying.
    cloud_stamp: Stamp,
    /// `content_hash` of what was copied.
    hash: u64,
    /// The copy was already in the cloud (another computer's, or put there by hand), so it
    /// isn't this computer's to remove on undo.
    #[serde(default)]
    adopted: bool,
    /// The local file was seen gone: a file turning up under its name is another file, unless
    /// it holds what was copied.
    #[serde(default)]
    orphan: bool,
    /// Undo asked for the copy to go, but that couldn't be done yet: tried again every pass.
    #[serde(default)]
    undo: bool,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct State {
    /// The study folder and the copy the entries belong to.
    local: PathBuf,
    cloud: PathBuf,
    /// By path relative to the study folder ('/'-separated, lower-case on Windows).
    files: BTreeMap<String, Entry>,
}

/// The copy's state, loaded from and saved to the state file.
#[derive(Debug)]
pub struct Mirror {
    state_file: PathBuf,
    state: State,
    /// Changed since it was last saved.
    dirty: bool,
    /// The cloud folder was missing last time (logged only when that changes).
    unreachable: bool,
    /// Problems already logged in this run (a pass every minute would repeat them).
    logged: HashSet<String>,
}

/// Why a file was left for now.
#[derive(Debug)]
enum Skip {
    /// Open in another program, still changing, or being written by someone else.
    Busy,
    /// Gone while being looked at.
    Gone,
    Failed(io::Error),
}

impl From<io::Error> for Skip {
    /// Only a sharing or lock violation means another program has the file open. "Access
    /// denied" (Controlled Folder Access, missing rights) won't pass by itself, so it is
    /// reported instead of being counted as "in use" on every pass.
    fn from(e: io::Error) -> Skip {
        if cfg!(windows) && matches!(e.raw_os_error(), Some(32 | 33)) { Skip::Busy } else { Skip::Failed(e) }
    }
}

impl From<MoveError> for Skip {
    fn from(e: MoveError) -> Skip {
        match e {
            MoveError::InUse(e) | MoveError::Failed(e) => e.into(),
        }
    }
}

/// A cloud copy compared with what was copied there.
#[derive(Debug, Clone, Copy, PartialEq)]
enum CloudCopy {
    Missing,
    /// Still what was copied (its stamp now).
    Unchanged(Stamp),
    Changed,
}

/// What the file replaced by a finished temporary copy must still look like.
#[derive(Debug, Clone, Copy)]
enum Expect {
    /// A name claimed by `reserve_destination` (or by a killed run): empty.
    Empty,
    /// The cloud copy as just checked: anything else would be someone's change.
    Stamp(Stamp),
}

/// The local file being looked at.
struct Local<'a> {
    path: &'a Path,
    /// Below the study folder.
    rel: &'a Path,
    key: String,
    now: Stamp,
}

/// One run over some files, and what it did.
struct Pass<'a> {
    local: &'a Path,
    cloud: &'a Path,
    report: MirrorReport,
    /// Cloud folders already cleared of stale temporary files.
    swept: HashSet<PathBuf>,
}

impl Mirror {
    /// The state saved in `state_file` (empty if there is none yet or it can't be read).
    pub fn load(state_file: &Path) -> Mirror {
        let state =
            fs::read_to_string(state_file).ok().and_then(|text| serde_json::from_str(&text).ok()).unwrap_or_default();
        Mirror { state_file: state_file.to_path_buf(), state, dirty: false, unreachable: false, logged: HashSet::new() }
    }

    /// Bring the copy in `cloud` up to date with everything below `local`.
    pub fn sync_all(&mut self, local: &Path, cloud: &Path, cancel: &AtomicBool) -> MirrorReport {
        let mut pass = Pass::new(local, cloud);
        if self.prepare(&mut pass) {
            let files = walk(local, &mut pass.report.errors);
            let mut saved = Instant::now();
            for file in &files {
                if cancel.load(Ordering::Relaxed) {
                    pass.report.cancelled = true;
                    break;
                }
                if let Ok(rel) = file.strip_prefix(local) {
                    self.sync_path(&mut pass, file, rel, true);
                }
                if self.dirty && saved.elapsed() >= SAVE_EVERY {
                    self.save_if_dirty();
                    saved = Instant::now();
                }
            }
            if !pass.report.cancelled {
                self.retry_undo(local, cloud);
                self.prune(local, cloud);
            }
        }
        self.save_if_dirty();
        pass.report
    }

    /// The same for one file below `local` (just sorted into it).
    pub fn sync_file(&mut self, local: &Path, cloud: &Path, file: &Path) -> MirrorReport {
        let mut pass = Pass::new(local, cloud);
        // Files outside the study folder (a module with a folder of its own) have no copy.
        if let Some(rel) = relative(local, file).filter(|_| wanted(&sorter::file_name(file)))
            && self.prepare(&mut pass)
        {
            self.sync_path(&mut pass, file, &rel, false);
        }
        self.save_if_dirty();
        pass.report
    }

    /// `file` (below `local`) was taken out again (undo): put its cloud copy into the
    /// Recycle Bin unless it was changed there or isn't this computer's, and forget it. Returns
    /// whether a copy was removed. On an error the entry is kept and tried again every pass.
    pub fn forget(&mut self, local: &Path, cloud: &Path, file: &Path) -> io::Result<bool> {
        if !self.bound(local, cloud) {
            return Ok(false);
        }
        let Some(key) = relative(local, file).and_then(|rel| key(&rel)) else { return Ok(false) };
        let Some(entry) = self.state.files.get(&key).cloned() else { return Ok(false) };
        let result = remove_copy(cloud, &entry, PATIENCE);
        if result.is_ok() {
            self.state.files.remove(&key);
            self.dirty = true;
        } else if let Some(entry) = self.state.files.get_mut(&key)
            && !entry.undo
        {
            entry.undo = true;
            self.dirty = true;
        }
        self.save_if_dirty();
        result
    }

    /// The study folder was moved into or out of the cloud folder: the entries no longer say
    /// what is where (a copy moved away would look deleted in the cloud and never be made
    /// again), so the copy starts afresh, adopting the files that are the same.
    pub fn reset(&mut self) {
        if !self.state.files.is_empty() {
            log::info!("The study folder was moved; the cloud copy is compared afresh");
            self.state.files.clear();
            self.dirty = true;
        }
        self.save_if_dirty();
    }

    /// Undo that couldn't remove a copy at the time: try again (while the file is still gone here).
    fn retry_undo(&mut self, local: &Path, cloud: &Path) {
        let pending: Vec<(String, Entry)> = self
            .state
            .files
            .iter()
            .filter(|(key, entry)| entry.undo && below(local, key).is_some_and(|p| gone(&p)))
            .map(|(key, entry)| (key.clone(), entry.clone()))
            .collect();
        for (key, entry) in pending {
            match remove_copy(cloud, &entry, Duration::ZERO) {
                Ok(removed) => {
                    if removed {
                        log::info!("Removed the cloud copy {} (sorting it was undone)", entry.cloud);
                    }
                    self.state.files.remove(&key);
                    self.dirty = true;
                }
                Err(e) => self.log_once(format!("Could not remove the cloud copy {} yet: {e}", entry.cloud)),
            }
        }
    }

    pub fn save(&self) -> io::Result<()> {
        save_json(&self.state, &self.state_file)
    }

    fn save_if_dirty(&mut self) {
        if !self.dirty {
            return;
        }
        match self.save() {
            Ok(()) => self.dirty = false,
            Err(e) => self.log_once(format!("Could not save the state of the cloud copy: {e}")),
        }
    }

    fn log_once(&mut self, message: String) {
        if self.logged.insert(message.clone()) {
            log::info!("{message}");
        }
    }

    /// Checks before copying anything; false when nothing can be done now.
    fn prepare(&mut self, pass: &mut Pass) -> bool {
        let (local, cloud) = (pass.local, pass.cloud);
        if !reachable(cloud) {
            if !self.unreachable {
                log::info!("{} can't be reached; the copy is updated later", cloud.display());
            }
            self.unreachable = true;
            pass.report.unreachable = true;
            return false;
        }
        self.unreachable = false;
        if is_inside(local, cloud) || is_inside(cloud, local) {
            let e = "the study folder and its copy are the same folder or inside each other";
            self.log_once(format!("Not copying {} to {}: {e}", local.display(), cloud.display()));
            pass.report.errors.push((cloud.to_path_buf(), e.into()));
            return false;
        }
        if !local.is_dir() {
            pass.report.errors.push((local.to_path_buf(), "the study folder isn't there".into()));
            return false;
        }
        self.bind(local, cloud);
        if gone(cloud) && !self.state.files.is_empty() {
            log::info!("{} is gone; copying the study folder there again", cloud.display());
            self.state.files.clear();
            self.dirty = true;
        }
        if let Err(e) = fs::create_dir_all(cloud) {
            self.log_once(format!("Could not create {}: {e}", cloud.display()));
            pass.report.errors.push((cloud.to_path_buf(), e.to_string()));
            return false;
        }
        true
    }

    /// True if the entries belong to this study folder and copy (however they are spelled).
    fn bound(&self, local: &Path, cloud: &Path) -> bool {
        let same = |saved: &Path, given: &Path| {
            saved == given || (!saved.as_os_str().is_empty() && is_inside(saved, given) && is_inside(given, saved))
        };
        same(&self.state.local, local) && same(&self.state.cloud, cloud)
    }

    /// The entries are for another study folder or copy: start afresh for these.
    fn bind(&mut self, local: &Path, cloud: &Path) {
        if self.bound(local, cloud) {
            return;
        }
        if !self.state.files.is_empty() {
            log::info!("The copy is now kept in {} (was {})", cloud.display(), self.state.cloud.display());
        }
        self.state = State { local: local.to_path_buf(), cloud: cloud.to_path_buf(), files: BTreeMap::new() };
        self.dirty = true;
    }

    fn sync_path(&mut self, pass: &mut Pass, path: &Path, rel: &Path, settle: bool) {
        let result = match (key(rel), local_stamp(path)) {
            (None, _) => Err(Skip::Failed(io::Error::other("the name can't be written down"))),
            (_, None) => Ok(()), // gone meanwhile
            (Some(key), Some(now)) => self.sync_one(pass, &Local { path, rel, key, now }, settle),
        };
        match result {
            Ok(()) | Err(Skip::Gone) => {}
            Err(Skip::Busy) => pass.report.in_use += 1,
            // Deleted while being copied: nothing to report.
            Err(Skip::Failed(_)) if gone(path) => {}
            Err(Skip::Failed(e)) => {
                self.log_once(format!("Could not copy {} into the cloud: {e}", path.display()));
                pass.report.errors.push((path.to_path_buf(), e.to_string()));
            }
        }
    }

    fn sync_one(&mut self, pass: &mut Pass, file: &Local, settle: bool) -> Result<(), Skip> {
        if let Some(mut entry) = self.state.files.get(&file.key).cloned() {
            if let Some(copy) = below(pass.cloud, &entry.cloud) {
                // Gone here before, or just sorted into a free name: the file the entry is
                // about was gone, so this is another one unless it holds what was copied.
                if entry.orphan || entry.undo || !settle {
                    if entry.local != file.now && content_hash(file.path)? != entry.hash {
                        return self.sync_successor(pass, file, &copy, settle);
                    }
                    if entry.orphan || entry.undo || entry.local != file.now {
                        (entry.local, entry.orphan, entry.undo) = (file.now, false, false);
                        self.state.files.insert(file.key.clone(), entry.clone());
                        self.dirty = true;
                    }
                }
                return self.sync_known(pass, file, &entry, &copy, settle);
            }
            // Not a path inside the copy (a damaged state file): not trusted.
            self.state.files.remove(&file.key);
            self.dirty = true;
        }
        ready(file, settle)?;
        self.sync_new(pass, file)
    }

    /// Another file under the name of one that was gone here (week 2's "Übung.pdf" after week
    /// 1's was deleted): the old copy may be the only one left of the old file, so it is never
    /// replaced; this file goes beside it.
    fn sync_successor(&mut self, pass: &mut Pass, file: &Local, copy: &Path, settle: bool) -> Result<(), Skip> {
        ready(file, settle)?;
        if !is_file(copy) {
            self.state.files.remove(&file.key);
            self.dirty = true;
            return self.sync_new(pass, file);
        }
        let hash = content_hash(file.path)?;
        if sorter::same_content(file.path, copy) {
            return self.adopt(pass.cloud, file, copy, hash);
        }
        log::info!("{} is another file than the one copied to {} before", file.path.display(), copy.display());
        let folder = copy.parent().unwrap_or(pass.cloud);
        // The old entry still claims the old copy, so the conflict copy can't land on it.
        self.conflict(pass, file, folder, copy, hash)
    }

    fn sync_known(
        &mut self,
        pass: &mut Pass,
        file: &Local,
        entry: &Entry,
        copy: &Path,
        settle: bool,
    ) -> Result<(), Skip> {
        if entry.local == file.now {
            match check_cloud(copy, entry)? {
                CloudCopy::Unchanged(seen) => self.restamp(&file.key, seen),
                CloudCopy::Changed => pass.report.changed_in_cloud += 1,
                CloudCopy::Missing => {} // deleted in the cloud: respected
            }
            return Ok(());
        }
        ready(file, settle)?;
        match check_cloud(copy, entry)? {
            CloudCopy::Unchanged(seen) => {
                let hash = content_hash(file.path)?;
                let placed = pass.fill(file, copy, Expect::Stamp(seen))?;
                // Undo still leaves alone a copy that was adopted (it holds this file now, but
                // it was there before).
                self.record(pass.cloud, file, copy, placed, hash, entry.adopted);
                pass.report.updated += 1;
                Ok(())
            }
            CloudCopy::Missing => {
                // Deleted in the cloud, but changed here since: the new version is wanted there.
                self.state.files.remove(&file.key);
                self.dirty = true;
                self.sync_new(pass, file)
            }
            // The same version arrived both ways (or a killed run had replaced it already).
            CloudCopy::Changed if is_file(copy) && sorter::same_content(file.path, copy) => {
                let hash = content_hash(file.path)?;
                self.adopt(pass.cloud, file, copy, hash)
            }
            CloudCopy::Changed => {
                let hash = content_hash(file.path)?;
                let folder = copy.parent().unwrap_or(pass.cloud);
                self.conflict(pass, file, folder, copy, hash)
            }
        }
    }

    /// A file without a copy (yet, or any more).
    fn sync_new(&mut self, pass: &mut Pass, file: &Local) -> Result<(), Skip> {
        let target = pass.cloud.join(file.rel);
        let (Some(folder), Some(name)) = (target.parent(), file.rel.file_name()) else { return Err(Skip::Gone) };
        let name = name.to_string_lossy();
        fs::create_dir_all(folder)?;
        let hash = content_hash(file.path)?;
        let existing = match fs::symlink_metadata(&target) {
            Ok(meta) => meta,
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                let (copy, placed) = pass.copy_new(file, folder, &name)?;
                self.record(pass.cloud, file, &copy, placed, hash, false);
                pass.report.copied += 1;
                return Ok(());
            }
            Err(e) => return Err(e.into()),
        };
        let free = existing.is_file() && !self.claimed(pass.cloud, &target);
        if free && sorter::same_content(file.path, &target) {
            // Already there: copied by hand, or by a run that was killed before saving.
            return self.adopt(pass.cloud, file, &target, hash);
        }
        if free && existing.len() == 0 && file.now.size > 0 {
            // Claimed by a run that was killed before filling it: it holds nothing.
            let placed = pass.fill(file, &target, Expect::Empty)?;
            self.record(pass.cloud, file, &target, placed, hash, false);
            pass.report.copied += 1;
            return Ok(());
        }
        self.conflict(pass, file, folder, &target, hash)
    }

    /// This computer's version beside a different `other` in the cloud, as "name (<computer>).ext".
    fn conflict(&mut self, pass: &mut Pass, file: &Local, folder: &Path, other: &Path, hash: u64) -> Result<(), Skip> {
        let name = conflict_name(&sorter::file_name(file.path));
        let candidate = folder.join(&name);
        if let Ok(meta) = fs::symlink_metadata(&candidate)
            && meta.is_file()
            && !self.claimed(pass.cloud, &candidate)
        {
            if sorter::same_content(file.path, &candidate) {
                // Saved beside it before, by a run whose state was lost.
                return self.adopt(pass.cloud, file, &candidate, hash);
            }
            if meta.len() == 0 && file.now.size > 0 {
                let placed = pass.fill(file, &candidate, Expect::Empty)?;
                self.conflicted(pass, file, other, &candidate, placed, hash);
                return Ok(());
            }
        }
        let (copy, placed) = pass.copy_new(file, folder, &name)?;
        self.conflicted(pass, file, other, &copy, placed, hash);
        Ok(())
    }

    fn conflicted(&mut self, pass: &mut Pass, file: &Local, other: &Path, copy: &Path, placed: Stamp, hash: u64) {
        log::info!(
            "{} is a different version in the cloud; this computer's version is beside it as {}",
            other.display(),
            sorter::file_name(copy)
        );
        self.record(pass.cloud, file, copy, placed, hash, false);
        pass.report.conflicts.push(copy.to_path_buf());
    }

    /// Follow `copy`, which already holds what `file` holds (not this computer's to remove).
    fn adopt(&mut self, cloud: &Path, file: &Local, copy: &Path, hash: u64) -> Result<(), Skip> {
        let seen = stamp(copy).ok_or(Skip::Gone)?;
        self.record(cloud, file, copy, seen, hash, true);
        Ok(())
    }

    fn record(&mut self, cloud: &Path, file: &Local, copy: &Path, cloud_stamp: Stamp, hash: u64, adopted: bool) {
        let Some(rel) = copy.strip_prefix(cloud).ok().and_then(slashed) else { return };
        let entry = Entry { local: file.now, cloud: rel, cloud_stamp, hash, adopted, orphan: false, undo: false };
        self.state.files.insert(file.key.clone(), entry);
        self.dirty = true;
    }

    /// A copy proven unchanged by its content keeps its new stamp, so it isn't read again.
    fn restamp(&mut self, key: &str, seen: Stamp) {
        if let Some(entry) = self.state.files.get_mut(key)
            && !seen.roughly_eq(&entry.cloud_stamp, SLACK_SECS)
        {
            entry.cloud_stamp = seen;
            self.dirty = true;
        }
    }

    /// True if an entry follows this cloud file already (it is never adopted or filled for another).
    fn claimed(&self, cloud: &Path, path: &Path) -> bool {
        let Some(rel) = path.strip_prefix(cloud).ok().and_then(slashed) else { return false };
        let rel = fold(&rel);
        self.state.files.values().any(|entry| fold(&entry.cloud) == rel)
    }

    /// Forget files gone both here and in the cloud. (Gone only here: the entry stays, marked
    /// as an orphan, so the copy isn't taken for someone else's file, nor replaced by another
    /// file that gets the name later.)
    fn prune(&mut self, local: &Path, cloud: &Path) {
        let mut changed = false;
        self.state.files.retain(|key, entry| {
            let here = below(local, key).is_some_and(|p| !gone(&p));
            let there = below(cloud, &entry.cloud).is_some_and(|p| !gone(&p));
            if !here && there && !entry.orphan {
                entry.orphan = true;
                changed = true;
            }
            changed |= !(here || there);
            here || there
        });
        self.dirty |= changed;
    }
}

impl<'a> Pass<'a> {
    fn new(local: &'a Path, cloud: &'a Path) -> Pass<'a> {
        Pass { local, cloud, report: MirrorReport::default(), swept: HashSet::new() }
    }

    /// Copy `file` into `folder` as `name`, or as "name (1).ext" etc. if that is taken.
    fn copy_new(&mut self, file: &Local, folder: &Path, name: &str) -> Result<(PathBuf, Stamp), Skip> {
        self.sweep(folder);
        let target = sorter::reserve_destination(folder, name)?;
        match self.fill(file, &target, Expect::Empty) {
            Ok(placed) => Ok((target, placed)),
            Err(e) => {
                sorter::release_placeholder(&target);
                Err(e)
            }
        }
    }

    /// Write `file` onto `target` through a temporary file beside it, which is renamed over
    /// `target` once complete and only while `target` is still as expected. Returns the new stamp.
    fn fill(&mut self, file: &Local, target: &Path, expect: Expect) -> Result<Stamp, Skip> {
        if let Some(folder) = target.parent() {
            self.sweep(folder);
        }
        let tmp = temp_path(target);
        if fs::symlink_metadata(&tmp).is_ok() {
            // Being written right now (by another computer that keeps a copy here too).
            return Err(Skip::Busy);
        }
        sorter::copy_file_with_patience(file.path, &tmp, PATIENCE)?;
        let placed = make_writable(&tmp).map_err(Skip::from).and_then(|()| finish(file, &tmp, target, expect));
        if placed.is_err() {
            let _ = remove_temp(&tmp);
        }
        placed
    }

    /// Remove temporary files left in `folder` by killed runs (once per pass).
    fn sweep(&mut self, folder: &Path) {
        if !self.swept.insert(folder.to_path_buf()) {
            return;
        }
        let Ok(entries) = fs::read_dir(folder) else { return };
        for entry in entries.flatten() {
            if !entry.file_name().to_string_lossy().to_lowercase().ends_with(TEMP_SUFFIX) {
                continue;
            }
            let stale = entry
                .metadata()
                .ok()
                .filter(fs::Metadata::is_file)
                .and_then(|meta| meta.modified().ok())
                .and_then(|modified| modified.elapsed().ok())
                .is_some_and(|age| age >= STALE);
            if stale && remove_temp(&entry.path()).is_ok() {
                log::info!("Removed {}, left over from an interrupted copy", entry.path().display());
            }
        }
    }
}

/// Put a complete temporary copy in place of `target`, unless something changed meanwhile.
fn finish(file: &Local, tmp: &Path, target: &Path, expect: Expect) -> Result<Stamp, Skip> {
    keep_modified(tmp, file.now.modified);
    if local_stamp(file.path) != Some(file.now) {
        return Err(Skip::Busy); // changed while being copied
    }
    if fs::metadata(tmp)?.len() != file.now.size {
        return Err(Skip::Failed(io::Error::other("the copy is incomplete")));
    }
    // On the disk before it gets the real name: after a power cut that name must not hold
    // zeros, which the cloud app would upload over the good version.
    sorter::retry_while_in_use(PATIENCE, || reopen(tmp).and_then(|f| f.sync_all()))?;
    let still = match expect {
        Expect::Empty => fs::symlink_metadata(target).is_ok_and(|meta| meta.is_file() && meta.len() == 0),
        // Not from under a program showing it either: notes may still be saved into it.
        Expect::Stamp(seen) => {
            (is_placeholder(target)
                || sorter::retry_while_in_use(PATIENCE, || sorter::ensure_not_open_elsewhere(target)).is_ok())
                && stamp(target) == Some(seen)
        }
    };
    if !still {
        return Err(Skip::Busy); // decided again next time
    }
    match sorter::retry_while_in_use(PATIENCE, || fs::rename(tmp, target)) {
        Ok(()) => stamp(target).ok_or(Skip::Gone),
        Err(e) if fs::symlink_metadata(target).is_ok_and(|meta| meta.permissions().readonly()) => {
            Err(Skip::Failed(io::Error::new(e.kind(), format!("{} is read-only", target.display()))))
        }
        // Windows refuses to replace a file another program has open with "access denied" too.
        Err(e) if cfg!(windows) && e.raw_os_error() == Some(5) => Err(Skip::Busy),
        Err(e) => Err(e.into()),
    }
}

/// Put `entry`'s copy into the Recycle Bin, unless it was changed in the cloud or this computer
/// didn't write it. Returns whether it was removed.
fn remove_copy(cloud: &Path, entry: &Entry, patience: Duration) -> io::Result<bool> {
    let Some(copy) = below(cloud, &entry.cloud) else { return Ok(false) };
    if entry.adopted {
        log::info!("Keeping {}: it was in the cloud before this computer copied it", copy.display());
        return Ok(false);
    }
    // The entry stays for later rather than the copy being taken for deleted.
    if !reachable(cloud) {
        return Err(io::Error::new(io::ErrorKind::NotFound, "the cloud folder can't be reached"));
    }
    match check_cloud(&copy, entry)? {
        CloudCopy::Unchanged(_) => {
            // Not from under a program showing it (notes may still be saved into it).
            if !is_placeholder(&copy) {
                sorter::retry_while_in_use(patience, || sorter::ensure_not_open_elsewhere(&copy))?;
            }
            sorter::retry_while_in_use(patience, || sorter::discard(&copy))?;
            Ok(true)
        }
        CloudCopy::Changed => {
            log::info!("Keeping {}: it was changed in the cloud", copy.display());
            Ok(false)
        }
        CloudCopy::Missing => Ok(false),
    }
}

/// Whether the copy is still what was copied there. It is hashed only when its stamp differs,
/// since reading a file that is only in the cloud downloads it.
fn check_cloud(copy: &Path, entry: &Entry) -> io::Result<CloudCopy> {
    match fs::symlink_metadata(copy) {
        Ok(meta) if meta.is_file() => {}
        Ok(_) => return Ok(CloudCopy::Changed), // something else in its place
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(CloudCopy::Missing),
        Err(e) => return Err(e),
    }
    let Some(seen) = stamp(copy) else { return Ok(CloudCopy::Missing) };
    let saved = &entry.cloud_stamp;
    if seen.roughly_eq(saved, SLACK_SECS) || (seen.size == saved.size && content_hash(copy)? == entry.hash) {
        Ok(CloudCopy::Unchanged(seen))
    } else {
        Ok(CloudCopy::Changed)
    }
}

/// Not while the file may still be being written (`settle`: changed in the last 3 s), nor
/// while another program has it open (a PDF viewer may still save notes into it).
fn ready(file: &Local, settle: bool) -> Result<(), Skip> {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos() as u64);
    if settle && now.abs_diff(file.now.modified) < SETTLE.as_nanos() as u64 {
        return Err(Skip::Busy);
    }
    sorter::ensure_not_open_elsewhere(file.path)?;
    Ok(())
}

/// The cloud folder the copy goes into is there (`cloud` without `.` and `..`, which
/// `Path::parent` would take literally).
fn reachable(cloud: &Path) -> bool {
    let full = std::path::absolute(cloud).unwrap_or_else(|_| cloud.to_path_buf());
    let mut clean = PathBuf::new();
    for part in full.components() {
        match part {
            Component::CurDir => {}
            Component::ParentDir => {
                clean.pop();
            }
            other => clean.push(other),
        }
    }
    clean.parent().is_some_and(Path::is_dir)
}

/// Every file below `local` worth copying, sorted. Symbolic links and junctions are not followed.
fn walk(local: &Path, errors: &mut Vec<(PathBuf, String)>) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let mut folders = vec![local.to_path_buf()];
    while let Some(folder) = folders.pop() {
        let entries = match fs::read_dir(&folder) {
            Ok(entries) => entries,
            Err(e) => {
                errors.push((folder, e.to_string()));
                continue;
            }
        };
        for entry in entries.flatten() {
            let Ok(kind) = entry.file_type() else { continue };
            if kind.is_dir() {
                folders.push(entry.path());
            } else if kind.is_file() && wanted(&entry.file_name().to_string_lossy()) {
                files.push(entry.path());
            }
        }
    }
    files.sort();
    files
}

/// Not junk, and not the shared settings (which live in the cloud and are written there).
fn wanted(name: &str) -> bool {
    !is_junk(name) && !name.eq_ignore_ascii_case(SHARED_FILE)
}

/// The stamp of a regular file (not of what a symbolic link points to).
fn local_stamp(path: &Path) -> Option<Stamp> {
    fs::symlink_metadata(path).ok().filter(fs::Metadata::is_file).and_then(|_| stamp(path))
}

fn is_file(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|meta| meta.is_file())
}

/// Not there at all (unlike a file that can't be looked at right now).
fn gone(path: &Path) -> bool {
    matches!(fs::symlink_metadata(path), Err(e) if e.kind() == io::ErrorKind::NotFound)
}

/// `file`'s path below `local`, also when the two are spelled differently.
fn relative(local: &Path, file: &Path) -> Option<PathBuf> {
    let plain =
        |rel: &Path| rel.components().next().is_some() && rel.components().all(|c| matches!(c, Component::Normal(_)));
    if let Ok(rel) = file.strip_prefix(local)
        && plain(rel)
    {
        return Some(rel.to_path_buf());
    }
    let (folder, name) = (file.parent()?.canonicalize().ok()?, file.file_name()?);
    let rel = folder.strip_prefix(local.canonicalize().ok()?).ok()?.join(name);
    plain(&rel).then_some(rel)
}

/// A relative path written with '/' (None if it isn't valid Unicode).
fn slashed(rel: &Path) -> Option<String> {
    let parts: Option<Vec<&str>> = rel.components().map(|c| c.as_os_str().to_str()).collect();
    Some(parts?.join("/"))
}

/// Windows doesn't tell `Skript.pdf` from `skript.pdf`.
fn fold(text: &str) -> String {
    if cfg!(windows) { text.to_lowercase() } else { text.to_string() }
}

/// The state's key for a path below the study folder.
fn key(rel: &Path) -> Option<String> {
    slashed(rel).map(|text| fold(&text))
}

/// `rel` ('/'-separated) below `root`, or None if it would lead anywhere else.
fn below(root: &Path, rel: &str) -> Option<PathBuf> {
    let mut path = root.to_path_buf();
    for part in rel.split('/') {
        let mut parts = Path::new(part).components();
        match (parts.next(), parts.next()) {
            (Some(Component::Normal(_)), None) => path.push(part),
            _ => return None,
        }
    }
    Some(path)
}

/// This computer's name as it may appear in a file name.
fn computer() -> String {
    let forbidden = |c: char| c.is_control() || matches!(c, '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|');
    computer_name().chars().map(|c| if forbidden(c) { '-' } else { c }).collect()
}

/// "Skript (DESKTOP-4FJ2K).pdf": this computer's version beside another one.
fn conflict_name(name: &str) -> String {
    let path = Path::new(name);
    match (path.file_stem(), path.extension()) {
        (Some(stem), Some(ext)) => format!("{} ({}).{}", stem.to_string_lossy(), computer(), ext.to_string_lossy()),
        _ => format!("{name} ({})", computer()),
    }
}

/// "<name>.filesorter-tmp" next to `target`.
fn temp_path(target: &Path) -> PathBuf {
    let name = target.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    target.with_file_name(format!("{name}{TEMP_SUFFIX}"))
}

/// The copy keeps the original's modification time, as copies made on Windows do anyway.
fn keep_modified(path: &Path, nanos: u64) {
    if let Ok(file) = reopen(path) {
        let _ = file.set_modified(UNIX_EPOCH + Duration::from_nanos(nanos));
    }
}

/// A temporary copy, opened for finishing touches: with write access where possible (Windows
/// needs it), else for reading (enough on Linux and macOS).
fn reopen(path: &Path) -> io::Result<fs::File> {
    fs::OpenOptions::new().write(true).open(path).or_else(|_| fs::File::open(path))
}

/// Clear the read-only attribute `fs::copy` carried over (Windows): a read-only copy in the
/// cloud could never be replaced, and a read-only temporary file never removed.
fn make_writable(path: &Path) -> io::Result<()> {
    #[cfg(windows)]
    {
        let mut permissions = fs::symlink_metadata(path)?.permissions();
        if permissions.readonly() {
            // Only the attribute on Windows, not Unix mode bits for everyone.
            #[allow(clippy::permissions_set_readonly_false)]
            permissions.set_readonly(false);
            fs::set_permissions(path, permissions)?;
        }
    }
    #[cfg(not(windows))]
    let _ = path;
    Ok(())
}

/// Remove one of File Sorter's own temporary files, also when it is read-only.
fn remove_temp(path: &Path) -> io::Result<()> {
    let _ = make_writable(path);
    fs::remove_file(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture {
        _tmp: tempfile::TempDir,
        root: PathBuf,
        local: PathBuf,
        cloud: PathBuf,
        state: PathBuf,
    }

    /// A study folder, a cloud folder (without the copy's folder yet) and a settings folder.
    fn fixture() -> Fixture {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().to_path_buf();
        let (local, cloud) = (root.join("Studium"), root.join("OneDrive").join("Studium"));
        fs::create_dir_all(&local).unwrap();
        fs::create_dir_all(cloud.parent().unwrap()).unwrap();
        let state = root.join("settings").join(MIRROR_STATE);
        Fixture { _tmp: tmp, root, local, cloud, state }
    }

    impl Fixture {
        fn mirror(&self) -> Mirror {
            Mirror::load(&self.state)
        }
        fn sync(&self, mirror: &mut Mirror) -> MirrorReport {
            mirror.sync_all(&self.local, &self.cloud, &AtomicBool::new(false))
        }
        fn sync_file(&self, mirror: &mut Mirror, rel: &str) -> MirrorReport {
            mirror.sync_file(&self.local, &self.cloud, &self.here(rel))
        }
        fn forget(&self, mirror: &mut Mirror, rel: &str) -> io::Result<bool> {
            mirror.forget(&self.local, &self.cloud, &self.here(rel))
        }
        fn here(&self, rel: &str) -> PathBuf {
            below(&self.local, rel).unwrap()
        }
        fn there(&self, rel: &str) -> PathBuf {
            below(&self.cloud, rel).unwrap()
        }
        /// This computer's version beside `rel` in the cloud.
        fn beside(&self, rel: &str) -> PathBuf {
            let there = self.there(rel);
            there.with_file_name(conflict_name(&sorter::file_name(&there)))
        }
    }

    /// Write `text` to `path`, last changed `secs_ago` seconds ago (so the 3-second wait doesn't
    /// matter; each version gets an age of its own, as real edits do).
    fn put(path: &Path, text: &str, secs_ago: u64) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
        set_modified(path, SystemTime::now() - Duration::from_secs(secs_ago));
    }

    fn set_modified(path: &Path, time: SystemTime) {
        fs::OpenOptions::new().write(true).open(path).unwrap().set_modified(time).unwrap();
    }

    /// Move the modification time on by `secs` (as cloud apps do after uploading).
    fn shift(path: &Path, secs: u64) {
        let modified = fs::metadata(path).unwrap().modified().unwrap();
        set_modified(path, modified + Duration::from_secs(secs));
    }

    fn read(path: &Path) -> Option<String> {
        fs::read_to_string(path).ok()
    }

    /// Every file below `dir` ('/'-separated, sorted), temporary files included.
    fn files(dir: &Path) -> Vec<String> {
        let mut found = Vec::new();
        let mut folders = vec![dir.to_path_buf()];
        while let Some(folder) = folders.pop() {
            for entry in fs::read_dir(&folder).into_iter().flatten().flatten() {
                if entry.file_type().unwrap().is_dir() {
                    folders.push(entry.path());
                } else {
                    found.push(slashed(entry.path().strip_prefix(dir).unwrap()).unwrap());
                }
            }
        }
        found.sort();
        found
    }

    fn copied(n: u64) -> MirrorReport {
        MirrorReport { copied: n, ..Default::default() }
    }

    fn updated(n: u64) -> MirrorReport {
        MirrorReport { updated: n, ..Default::default() }
    }

    const NOTHING: Vec<String> = Vec::new();

    #[test]
    fn copies_new_files_and_keeps_working_after_a_restart() {
        let f = fixture();
        put(&f.here("Mathe/Analysis/Skript.pdf"), "slides v1", 300);
        put(&f.here("notes.txt"), "notes", 300);
        let mut m = f.mirror();
        assert_eq!(f.sync(&mut m), copied(2));
        assert_eq!(files(&f.cloud), ["Mathe/Analysis/Skript.pdf", "notes.txt"]);
        assert_eq!(read(&f.there("Mathe/Analysis/Skript.pdf")).as_deref(), Some("slides v1"));
        let modified = |p: &Path| fs::metadata(p).unwrap().modified().unwrap();
        assert_eq!(modified(&f.there("notes.txt")), modified(&f.here("notes.txt")), "the original's time");
        assert!(f.state.is_file());

        // Nothing changed: nothing done, also after a restart.
        assert_eq!(f.sync(&mut m), MirrorReport::default());
        assert_eq!(f.sync(&mut f.mirror()), MirrorReport::default());

        // Edited, then restarted: the copy follows.
        put(&f.here("Mathe/Analysis/Skript.pdf"), "slides v2 with notes", 200);
        let mut m = f.mirror();
        assert_eq!(f.sync(&mut m), updated(1));
        assert_eq!(read(&f.there("Mathe/Analysis/Skript.pdf")).as_deref(), Some("slides v2 with notes"));
        assert_eq!(files(&f.cloud), ["Mathe/Analysis/Skript.pdf", "notes.txt"], "no temporary files left");
    }

    #[test]
    fn files_already_in_the_cloud_are_adopted_or_get_this_computers_version_beside_them() {
        let f = fixture();
        put(&f.here("Mathe/same.pdf"), "same", 300);
        put(&f.there("Mathe/same.pdf"), "same", 50);
        put(&f.here("Mathe/Skript.pdf"), "mine", 300);
        put(&f.there("Mathe/Skript.pdf"), "theirs", 50);
        let beside = f.beside("Mathe/Skript.pdf");
        let mut m = f.mirror();
        assert_eq!(f.sync(&mut m), MirrorReport { conflicts: vec![beside.clone()], ..Default::default() });
        assert_eq!(read(&f.there("Mathe/Skript.pdf")).as_deref(), Some("theirs"));
        assert_eq!(read(&beside).as_deref(), Some("mine"));
        assert_eq!(files(&f.cloud).len(), 3, "the identical file isn't copied twice: {:?}", files(&f.cloud));

        // Both are followed from now on.
        assert_eq!(f.sync(&mut m), MirrorReport::default());
        put(&f.here("Mathe/same.pdf"), "same v2", 200);
        put(&f.here("Mathe/Skript.pdf"), "mine v2", 200);
        assert_eq!(f.sync(&mut m), updated(2));
        assert_eq!(read(&f.there("Mathe/same.pdf")).as_deref(), Some("same v2"));
        assert_eq!(read(&beside).as_deref(), Some("mine v2"));
        assert_eq!(read(&f.there("Mathe/Skript.pdf")).as_deref(), Some("theirs"));
    }

    #[derive(Debug, Clone, Copy)]
    enum InCloud {
        Untouched,
        Edited,
        Deleted,
    }

    /// A copied file after changes here and/or in the cloud: (edited here, in the cloud,
    /// report, the cloud file afterwards, this computer's version beside it).
    #[test]
    fn changes_here_and_in_the_cloud() {
        use InCloud::*;
        let none = MirrorReport::default;
        let in_cloud_only = MirrorReport { changed_in_cloud: 1, ..none() };
        let cases = [
            (false, Untouched, none(), Some("v1"), None),
            (true, Untouched, updated(1), Some("v2 with notes"), None),
            (false, Edited, in_cloud_only, Some("annotated on the tablet"), None),
            (true, Edited, none(), Some("annotated on the tablet"), Some("v2 with notes")),
            (false, Deleted, none(), None, None),
            (true, Deleted, copied(1), Some("v2 with notes"), None),
        ];
        for (edited_here, in_cloud, expected, cloud_text, beside_text) in cases {
            let case = format!("edited here: {edited_here}, in the cloud: {in_cloud:?}");
            let f = fixture();
            put(&f.here("Mathe/Skript.pdf"), "v1", 300);
            let mut m = f.mirror();
            assert_eq!(f.sync(&mut m), copied(1), "{case}");
            if edited_here {
                put(&f.here("Mathe/Skript.pdf"), "v2 with notes", 200);
            }
            match in_cloud {
                Untouched => {}
                Edited => put(&f.there("Mathe/Skript.pdf"), "annotated on the tablet", 100),
                Deleted => fs::remove_file(f.there("Mathe/Skript.pdf")).unwrap(),
            }
            let beside = f.beside("Mathe/Skript.pdf");
            let conflicts = if beside_text.is_some() { vec![beside.clone()] } else { Vec::new() };
            assert_eq!(f.sync(&mut m), MirrorReport { conflicts, ..expected.clone() }, "{case}");
            assert_eq!(read(&f.there("Mathe/Skript.pdf")).as_deref(), cloud_text, "{case}");
            assert_eq!(read(&beside).as_deref(), beside_text, "{case}");

            // Another pass (also after a restart) does nothing more.
            let again = MirrorReport { changed_in_cloud: expected.changed_in_cloud, ..none() };
            assert_eq!(f.sync(&mut m), again, "{case}");
            assert_eq!(f.sync(&mut f.mirror()), again, "{case}");
            assert_eq!(read(&f.there("Mathe/Skript.pdf")).as_deref(), cloud_text, "{case}");
            assert!(!files(&f.cloud).iter().any(|n| is_junk(n)), "{case}");
        }
    }

    #[test]
    fn after_a_conflict_the_copy_beside_it_is_followed() {
        let f = fixture();
        put(&f.here("Skript.pdf"), "v1", 300);
        let mut m = f.mirror();
        f.sync(&mut m);
        put(&f.here("Skript.pdf"), "v2", 200);
        put(&f.there("Skript.pdf"), "theirs", 100);
        let beside = f.beside("Skript.pdf");
        assert_eq!(f.sync(&mut m), MirrorReport { conflicts: vec![beside.clone()], ..Default::default() });

        // Edited here again: the copy beside it is replaced, theirs stays.
        put(&f.here("Skript.pdf"), "v3", 50);
        assert_eq!(f.sync(&mut m), updated(1));
        assert_eq!(read(&beside).as_deref(), Some("v3"));
        assert_eq!(read(&f.there("Skript.pdf")).as_deref(), Some("theirs"));

        // That one edited in the cloud too: yet another one beside it, nothing overwritten.
        put(&beside, "theirs too", 20);
        put(&f.here("Skript.pdf"), "v4", 10);
        let report = f.sync(&mut m);
        assert_eq!(report.conflicts.len(), 1);
        assert_ne!(report.conflicts[0], beside);
        assert_eq!(read(&report.conflicts[0]).as_deref(), Some("v4"));
        assert_eq!(read(&beside).as_deref(), Some("theirs too"));
        assert_eq!(read(&f.there("Skript.pdf")).as_deref(), Some("theirs"));
        assert_eq!(files(&f.cloud).len(), 3);
    }

    #[test]
    fn local_deletions_keep_the_cloud_copy() {
        let f = fixture();
        put(&f.here("Mathe/Skript.pdf"), "v1", 300);
        let mut m = f.mirror();
        f.sync(&mut m);
        fs::remove_file(f.here("Mathe/Skript.pdf")).unwrap();
        assert_eq!(f.sync(&mut m), MirrorReport::default());
        assert_eq!(f.sync(&mut f.mirror()), MirrorReport::default());
        assert_eq!(read(&f.there("Mathe/Skript.pdf")).as_deref(), Some("v1"));
        let key = key(&Path::new("Mathe").join("Skript.pdf")).unwrap();
        assert!(f.mirror().state.files.contains_key(&key), "still known while the copy exists");

        // Gone on both sides: forgotten.
        fs::remove_file(f.there("Mathe/Skript.pdf")).unwrap();
        let mut m = f.mirror();
        assert_eq!(f.sync(&mut m), MirrorReport::default());
        assert!(f.mirror().state.files.is_empty());

        // Back here later: a new file, copied again.
        put(&f.here("Mathe/Skript.pdf"), "v1", 300);
        assert_eq!(f.sync(&mut m), copied(1));
    }

    #[test]
    fn forget_removes_only_unchanged_copies() {
        let f = fixture();
        put(&f.here("a.pdf"), "aaa", 300);
        put(&f.here("b.pdf"), "bbb", 300);
        let mut m = f.mirror();
        assert_eq!(f.sync(&mut m), copied(2));

        // Sorting undone: the file was taken out here, its copy goes too.
        fs::remove_file(f.here("a.pdf")).unwrap();
        assert!(f.forget(&mut m, "a.pdf").unwrap());
        assert!(!f.there("a.pdf").exists());

        // Annotated on the tablet meanwhile: the copy stays.
        put(&f.there("b.pdf"), "bbb with notes", 100);
        fs::remove_file(f.here("b.pdf")).unwrap();
        assert!(!f.forget(&mut m, "b.pdf").unwrap());
        assert_eq!(read(&f.there("b.pdf")).as_deref(), Some("bbb with notes"));

        // Not known (any more): nothing.
        assert!(!f.forget(&mut m, "a.pdf").unwrap());
        assert!(!f.forget(&mut m, "never.pdf").unwrap());
        assert!(f.mirror().state.files.is_empty(), "saved");
        assert_eq!(files(&f.cloud), ["b.pdf"]);
    }

    #[test]
    fn unreachable_cloud_folder_changes_nothing() {
        let f = fixture();
        put(&f.here("a.pdf"), "aaa", 300);
        let disconnected = f.root.join("G").join("Studium");
        let mut m = f.mirror();
        let unreachable = MirrorReport { unreachable: true, ..Default::default() };
        assert_eq!(m.sync_all(&f.local, &disconnected, &AtomicBool::new(false)), unreachable);
        assert_eq!(m.sync_file(&f.local, &disconnected, &f.here("a.pdf")), unreachable);
        assert!(!f.root.join("G").exists() && !f.state.exists());

        // Copied before, then the drive is gone: undo keeps the copy and its entry for later.
        assert_eq!(f.sync(&mut m), copied(1));
        fs::rename(f.root.join("OneDrive"), f.root.join("away")).unwrap();
        assert_eq!(f.sync(&mut m), unreachable);
        assert!(!f.cloud.exists(), "not created again");
        fs::remove_file(f.here("a.pdf")).unwrap();
        assert!(f.forget(&mut m, "a.pdf").is_err());
        fs::rename(f.root.join("away"), f.root.join("OneDrive")).unwrap();
        assert!(f.forget(&mut m, "a.pdf").unwrap());
        assert!(!f.there("a.pdf").exists());
    }

    #[test]
    fn same_or_nested_folders_are_refused() {
        let f = fixture();
        put(&f.here("a.pdf"), "aaa", 300);
        let inner = f.cloud.join("Inner");
        put(&inner.join("b.pdf"), "bbb", 300);
        let cases = [
            (f.local.clone(), f.local.clone()),
            (f.local.clone(), f.local.join("x").join("..")),
            (f.local.clone(), f.local.join("Cloud")),
            (inner.clone(), f.cloud.clone()),
        ];
        for (local, cloud) in cases {
            let mut m = f.mirror();
            let report = m.sync_all(&local, &cloud, &AtomicBool::new(false));
            assert_eq!(report.errors.len(), 1, "{local:?} {cloud:?}");
            assert_eq!(MirrorReport { errors: Vec::new(), ..report }, MirrorReport::default());
            assert_eq!(m.sync_file(&local, &cloud, &local.join("a.pdf")).errors.len(), 1, "{local:?} {cloud:?}");
        }
        assert_eq!(files(&f.local), ["a.pdf"]);
        assert_eq!(files(&f.cloud), ["Inner/b.pdf"]);
        assert!(!f.state.exists());
    }

    #[test]
    fn junk_and_the_shared_settings_are_skipped() {
        let f = fixture();
        let junk = [
            "desktop.ini",
            "Mathe/Thumbs.db",
            "Mathe/~$Notizen.docx",
            "Mathe/.~lock.Notizen.odt#",
            "Mathe/Blatt3.pdf.crdownload",
            "Mathe/Blatt3.pdf.part",
            "Mathe/x.pdf.filesorter-tmp",
            SHARED_FILE,
        ];
        for name in junk {
            put(&f.here(name), "junk", 300);
        }
        put(&f.here("Mathe/Skript.pdf"), "slides", 300);
        let mut m = f.mirror();
        assert_eq!(f.sync(&mut m), copied(1));
        assert_eq!(files(&f.cloud), ["Mathe/Skript.pdf"]);
        for name in junk {
            assert_eq!(f.sync_file(&mut m, name), MirrorReport::default(), "{name}");
        }
        assert_eq!(files(&f.cloud), ["Mathe/Skript.pdf"]);
    }

    #[cfg(unix)]
    #[test]
    fn symbolic_links_are_not_followed() {
        let f = fixture();
        put(&f.root.join("elsewhere").join("big.iso"), "big", 300);
        std::os::unix::fs::symlink(f.root.join("elsewhere"), f.here("linked")).unwrap();
        std::os::unix::fs::symlink(f.root.join("elsewhere").join("big.iso"), f.here("big.iso")).unwrap();
        put(&f.here("Skript.pdf"), "slides", 300);
        let mut m = f.mirror();
        assert_eq!(f.sync(&mut m), copied(1));
        assert_eq!(f.sync_file(&mut m, "big.iso"), MirrorReport::default());
        assert_eq!(files(&f.cloud), ["Skript.pdf"]);
    }

    #[test]
    fn fresh_files_wait_for_a_pass_but_not_when_just_sorted() {
        let f = fixture();
        fs::write(f.here("Blatt1.pdf"), "just downloaded").unwrap();
        let mut m = f.mirror();
        let busy = MirrorReport { in_use: 1, ..Default::default() };
        assert_eq!(f.sync(&mut m), busy);
        assert_eq!(files(&f.cloud), NOTHING);
        assert_eq!(f.sync_file(&mut m, "Blatt1.pdf"), copied(1));
        assert_eq!(read(&f.there("Blatt1.pdf")).as_deref(), Some("just downloaded"));
        assert_eq!(f.sync(&mut m), MirrorReport::default());

        // Changed again just now: waits as well.
        fs::write(f.here("Blatt1.pdf"), "just changed").unwrap();
        assert_eq!(f.sync(&mut m), busy);
        assert_eq!(read(&f.there("Blatt1.pdf")).as_deref(), Some("just downloaded"));
        set_modified(&f.here("Blatt1.pdf"), SystemTime::now() - Duration::from_secs(10));
        assert_eq!(f.sync(&mut m), updated(1));
        assert_eq!(read(&f.there("Blatt1.pdf")).as_deref(), Some("just changed"));
    }

    /// Generic weekly names: week 2's "Übung.pdf" gets the name of week 1's, deleted here.
    #[test]
    fn another_file_under_a_deleted_files_name_never_replaces_its_copy() {
        let f = fixture();
        put(&f.here("Mathe/Übung.pdf"), "week 1", 300);
        let mut m = f.mirror();
        assert_eq!(f.sync(&mut m), copied(1));
        let beside = f.beside("Mathe/Übung.pdf");

        // Deleted here and seen gone by a pass, then another file gets the name.
        fs::remove_file(f.here("Mathe/Übung.pdf")).unwrap();
        assert_eq!(f.sync(&mut m), MirrorReport::default());
        put(&f.here("Mathe/Übung.pdf"), "week 2", 200);
        let conflict = MirrorReport { conflicts: vec![beside.clone()], ..Default::default() };
        assert_eq!(f.sync(&mut f.mirror()), conflict, "also after a restart");
        assert_eq!(read(&f.there("Mathe/Übung.pdf")).as_deref(), Some("week 1"));
        assert_eq!(read(&beside).as_deref(), Some("week 2"));

        // The new file's copy is followed from then on.
        put(&f.here("Mathe/Übung.pdf"), "week 2 with notes", 100);
        let mut m = f.mirror();
        assert_eq!(f.sync(&mut m), updated(1));
        assert_eq!(read(&beside).as_deref(), Some("week 2 with notes"));

        // Deleted and another one sorted there before a pass saw it gone: a file just sorted
        // got a free name, so it is another file too.
        fs::remove_file(f.here("Mathe/Übung.pdf")).unwrap();
        put(&f.here("Mathe/Übung.pdf"), "week 3", 50);
        let report = f.sync_file(&mut m, "Mathe/Übung.pdf");
        assert_eq!(report.conflicts.len(), 1, "{report:?}");
        assert_eq!(read(&report.conflicts[0]).as_deref(), Some("week 3"));
        assert_eq!(read(&beside).as_deref(), Some("week 2 with notes"));
        assert_eq!(read(&f.there("Mathe/Übung.pdf")).as_deref(), Some("week 1"));
        assert_eq!(files(&f.cloud).len(), 3);
        assert_eq!(f.sync(&mut m), MirrorReport::default());
    }

    #[test]
    fn a_deleted_file_that_comes_back_is_followed_again() {
        let f = fixture();
        put(&f.here("Skript.pdf"), "v1", 300);
        let mut m = f.mirror();
        assert_eq!(f.sync(&mut m), copied(1));
        fs::remove_file(f.here("Skript.pdf")).unwrap();
        assert_eq!(f.sync(&mut m), MirrorReport::default());

        // Restored (with another time): the same content, so the same file.
        put(&f.here("Skript.pdf"), "v1", 250);
        assert_eq!(f.sync(&mut m), MirrorReport::default());
        put(&f.here("Skript.pdf"), "v2", 200);
        assert_eq!(f.sync(&mut m), updated(1));
        assert_eq!(files(&f.cloud), ["Skript.pdf"]);

        // Deleted here and in the cloud: another file under the name is simply copied.
        fs::remove_file(f.here("Skript.pdf")).unwrap();
        assert_eq!(f.sync(&mut m), MirrorReport::default());
        fs::remove_file(f.there("Skript.pdf")).unwrap();
        put(&f.here("Skript.pdf"), "something else", 100);
        assert_eq!(f.sync_file(&mut m, "Skript.pdf"), copied(1));
        assert_eq!(read(&f.there("Skript.pdf")).as_deref(), Some("something else"));
    }

    /// Mirror, then Move (the copies became the study folder), then Mirror again (moved back).
    #[test]
    fn after_the_study_folder_was_in_the_cloud_and_back_the_copy_is_made_again() {
        let f = fixture();
        put(&f.here("Mathe/Skript.pdf"), "slides", 300);
        put(&f.here("notes.txt"), "notes", 300);
        let mut m = f.mirror();
        assert_eq!(f.sync(&mut m), copied(2));
        fs::remove_dir_all(&f.local).unwrap();
        fs::rename(&f.cloud, &f.local).unwrap();
        put(&f.cloud.join(SHARED_FILE), "{}", 10); // written there right away again
        assert_eq!(f.sync(&mut f.mirror()), MirrorReport::default(), "the moved copies look deleted in the cloud");

        m.reset();
        assert_eq!(f.sync(&mut m), copied(2));
        assert_eq!(files(&f.cloud), [SHARED_FILE, "Mathe/Skript.pdf", "notes.txt"]);
        assert_eq!(f.mirror().state.files.len(), 2);
    }

    #[test]
    fn undo_leaves_a_copy_that_was_there_before() {
        let f = fixture();
        // Sorted on the laptop and copied there, then sorted here as well.
        put(&f.there("Mathe/Blatt3.pdf"), "sheet 3", 100);
        put(&f.here("Mathe/Blatt3.pdf"), "sheet 3", 300);
        let mut m = f.mirror();
        assert_eq!(f.sync_file(&mut m, "Mathe/Blatt3.pdf"), MirrorReport::default());
        fs::remove_file(f.here("Mathe/Blatt3.pdf")).unwrap();
        assert!(!f.forget(&mut m, "Mathe/Blatt3.pdf").unwrap());
        assert_eq!(read(&f.there("Mathe/Blatt3.pdf")).as_deref(), Some("sheet 3"));
        assert!(f.mirror().state.files.is_empty());
    }

    #[test]
    fn undo_that_could_not_remove_the_copy_is_tried_again() {
        let f = fixture();
        put(&f.here("a.pdf"), "aaa", 300);
        put(&f.here("b.pdf"), "bbb", 300);
        let mut m = f.mirror();
        assert_eq!(f.sync(&mut m), copied(2));
        fs::rename(f.root.join("OneDrive"), f.root.join("away")).unwrap();
        for name in ["a.pdf", "b.pdf"] {
            fs::remove_file(f.here(name)).unwrap();
            assert!(f.forget(&mut m, name).is_err());
        }
        fs::rename(f.root.join("away"), f.root.join("OneDrive")).unwrap();

        // b was sorted again meanwhile: followed again, its copy stays.
        put(&f.here("b.pdf"), "bbb", 200);
        assert_eq!(f.sync_file(&mut m, "b.pdf"), MirrorReport::default());

        // The next pass (also after a restart) removes a's copy.
        assert_eq!(f.sync(&mut f.mirror()), MirrorReport::default());
        assert_eq!(files(&f.cloud), ["b.pdf"]);
        assert_eq!(f.mirror().state.files.len(), 1);
        put(&f.here("b.pdf"), "bbb v2", 100);
        assert_eq!(f.sync(&mut f.mirror()), updated(1));
    }

    #[test]
    fn sync_file_only_for_files_in_the_study_folder() {
        let f = fixture();
        let outside = f.root.join("Eigener Ordner").join("x.pdf");
        put(&outside, "x", 300);
        let mut m = f.mirror();
        assert_eq!(m.sync_file(&f.local, &f.cloud, &outside), MirrorReport::default());
        assert!(!f.cloud.exists());

        // The same folder spelled differently.
        put(&f.here("Mathe/new.pdf"), "new", 300);
        let spelled = f.local.join("Mathe").join("..").join("Mathe").join("new.pdf");
        assert_eq!(m.sync_file(&f.local, &f.cloud, &spelled), copied(1));
        assert_eq!(files(&f.cloud), ["Mathe/new.pdf"]);
        assert_eq!(f.sync(&mut m), MirrorReport::default(), "the same entry");
        assert_eq!(f.sync_file(&mut m, "Mathe/gone.pdf"), MirrorReport::default());
    }

    #[test]
    fn small_time_shifts_and_uploaded_again_count_as_unchanged() {
        let f = fixture();
        put(&f.here("Skript.pdf"), "v1", 300);
        let mut m = f.mirror();
        f.sync(&mut m);
        let key = key(Path::new("Skript.pdf")).unwrap();

        // Moved by a second (the cloud app rounding after uploading): still this computer's copy.
        shift(&f.there("Skript.pdf"), 1);
        assert_eq!(f.sync(&mut m), MirrorReport::default());
        put(&f.here("Skript.pdf"), "v2 with notes", 200);
        assert_eq!(f.sync(&mut m), updated(1));
        assert_eq!(read(&f.there("Skript.pdf")).as_deref(), Some("v2 with notes"));

        // Moved by a minute with the same content (uploaded again): unchanged by its content,
        // and the new time is remembered, so it isn't read again.
        shift(&f.there("Skript.pdf"), 60);
        assert_eq!(f.sync(&mut m), MirrorReport::default());
        assert_eq!(Some(f.mirror().state.files[&key].cloud_stamp), stamp(&f.there("Skript.pdf")));

        // The same size, but other content: changed in the cloud.
        put(&f.there("Skript.pdf"), "v2 with NOTES", 30);
        assert_eq!(f.sync(&mut m), MirrorReport { changed_in_cloud: 1, ..Default::default() });
    }

    #[test]
    fn leftovers_of_a_killed_run() {
        let f = fixture();
        fs::create_dir_all(&f.cloud).unwrap();
        // a: its name claimed, but not filled. b: half a temporary file next to it, long ago.
        // c: a temporary file being written right now (another computer keeping a copy here).
        for (name, text) in [("a.pdf", "aaa"), ("b.pdf", "bbb"), ("c.pdf", "ccc")] {
            put(&f.here(name), text, 300);
        }
        fs::write(f.there("a.pdf"), "").unwrap();
        put(&f.there("b.pdf.filesorter-tmp"), "b", 7200);
        put(&f.there("old.pdf.filesorter-tmp"), "o", 7200);
        put(&f.there("c.pdf.filesorter-tmp"), "c", 10);
        let mut m = f.mirror();
        assert_eq!(f.sync(&mut m), MirrorReport { copied: 2, in_use: 1, ..Default::default() });
        assert_eq!(files(&f.cloud), ["a.pdf", "b.pdf", "c.pdf.filesorter-tmp"]);
        assert_eq!(read(&f.there("a.pdf")).as_deref(), Some("aaa"));
        assert_eq!(read(&f.there("b.pdf")).as_deref(), Some("bbb"));
        assert_eq!(read(&f.there("c.pdf.filesorter-tmp")).as_deref(), Some("c"), "not touched");
        fs::remove_file(f.there("c.pdf.filesorter-tmp")).unwrap();
        assert_eq!(f.sync(&mut m), copied(1));

        // An empty file of its own is no leftover.
        put(&f.here("empty.txt"), "", 300);
        put(&f.there("empty.txt"), "", 100);
        assert_eq!(f.sync(&mut m), MirrorReport::default());
        assert_eq!(files(&f.cloud), ["a.pdf", "b.pdf", "c.pdf", "empty.txt"]);
    }

    #[test]
    fn killed_before_the_state_was_saved() {
        let f = fixture();
        put(&f.here("Skript.pdf"), "v1", 300);
        let mut m = f.mirror();
        f.sync(&mut m);
        // The new version was put in place, but the state still has the old one.
        put(&f.here("Skript.pdf"), "v2 with notes", 200);
        fs::copy(f.here("Skript.pdf"), f.there("Skript.pdf")).unwrap();
        assert_eq!(f.sync(&mut f.mirror()), MirrorReport::default());
        assert_eq!(files(&f.cloud), ["Skript.pdf"]);

        // No state at all, and a conflict copy made before: followed again, not made twice.
        put(&f.here("Mathe/Blatt.pdf"), "mine", 300);
        put(&f.there("Mathe/Blatt.pdf"), "theirs", 100);
        put(&f.beside("Mathe/Blatt.pdf"), "mine", 100);
        fs::remove_file(&f.state).unwrap();
        let mut m = f.mirror();
        assert_eq!(f.sync(&mut m), MirrorReport::default());
        assert_eq!(files(&f.cloud).len(), 3);
        put(&f.here("Mathe/Blatt.pdf"), "mine v2", 200);
        assert_eq!(f.sync(&mut m), updated(1));
        assert_eq!(read(&f.beside("Mathe/Blatt.pdf")).as_deref(), Some("mine v2"));
        assert_eq!(read(&f.there("Mathe/Blatt.pdf")).as_deref(), Some("theirs"));
    }

    #[test]
    fn another_place_or_a_deleted_copy_starts_afresh() {
        let f = fixture();
        put(&f.here("a.pdf"), "aaa", 300);
        let mut m = f.mirror();
        assert_eq!(f.sync(&mut m), copied(1));

        // Another cloud folder: everything is copied there.
        let dropbox = f.root.join("Dropbox").join("Studium");
        fs::create_dir_all(dropbox.parent().unwrap()).unwrap();
        assert_eq!(m.sync_all(&f.local, &dropbox, &AtomicBool::new(false)), copied(1));
        assert_eq!(files(&dropbox), ["a.pdf"]);

        // Back again: the copy there is the same, so it is followed, not copied twice.
        assert_eq!(f.sync(&mut m), MirrorReport::default());
        assert_eq!(files(&f.cloud), ["a.pdf"]);

        // The whole copy deleted (switched off, deleted, switched on again): made again.
        fs::remove_dir_all(&f.cloud).unwrap();
        assert_eq!(f.sync(&mut f.mirror()), copied(1));
        assert_eq!(files(&f.cloud), ["a.pdf"]);
    }

    #[test]
    fn cancel_and_an_unreadable_state() {
        let f = fixture();
        put(&f.here("a.pdf"), "aaa", 300);
        let mut m = f.mirror();
        let report = m.sync_all(&f.local, &f.cloud, &AtomicBool::new(true));
        assert_eq!(report, MirrorReport { cancelled: true, ..Default::default() });
        assert_eq!(files(&f.cloud), NOTHING);

        fs::create_dir_all(f.state.parent().unwrap()).unwrap();
        fs::write(&f.state, "{ not json").unwrap();
        let mut m = f.mirror();
        assert!(m.state.files.is_empty());
        assert_eq!(f.sync(&mut m), copied(1));
        assert_eq!(f.mirror().state.files.len(), 1, "written again");
    }

    #[test]
    fn names_and_paths() {
        let expected = if cfg!(windows) { "mathe/skript.pdf" } else { "Mathe/Skript.pdf" };
        assert_eq!(key(&Path::new("Mathe").join("Skript.pdf")).as_deref(), Some(expected));
        let root = Path::new("cloud");
        assert_eq!(below(root, "Mathe/Skript.pdf"), Some(root.join("Mathe").join("Skript.pdf")));
        for bad in ["", "/x", "../x", "a/../../x", "a//b", "./x", "a/."] {
            assert_eq!(below(root, bad), None, "{bad:?}");
        }
        let pc = computer();
        assert!(!pc.is_empty() && !pc.contains(['/', '\\', ':']));
        assert_eq!(conflict_name("Skript.pdf"), format!("Skript ({pc}).pdf"));
        assert_eq!(conflict_name("notes.tar.gz"), format!("notes.tar ({pc}).gz"));
        assert_eq!(conflict_name("Makefile"), format!("Makefile ({pc})"));
        assert!(is_junk(&sorter::file_name(&temp_path(Path::new("a.pdf")))));
        let s = Path::new("s");
        assert_eq!(relative(s, &s.join("a").join("b.pdf")), Some(Path::new("a").join("b.pdf")));
        assert_eq!(relative(s, s), None);
        assert!(!reachable(Path::new("/")));
    }

    /// PDF viewers keep files open and may still save notes into them.
    #[cfg(windows)]
    #[test]
    fn open_files_wait_until_closed() {
        let f = fixture();
        put(&f.here("a.pdf"), "aaa", 300);
        let open = fs::File::open(f.here("a.pdf")).unwrap();
        let mut m = f.mirror();
        let busy = MirrorReport { in_use: 1, ..Default::default() };
        assert_eq!(f.sync(&mut m), busy);
        assert_eq!(f.sync_file(&mut m, "a.pdf"), busy);
        assert!(!f.there("a.pdf").exists());
        drop(open);
        assert_eq!(f.sync(&mut m), copied(1));

        // Changed while open here: replaced only once closed.
        put(&f.here("a.pdf"), "aaa with notes", 200);
        let open = fs::File::open(f.here("a.pdf")).unwrap();
        assert_eq!(f.sync(&mut m), busy);
        assert_eq!(read(&f.there("a.pdf")).as_deref(), Some("aaa"));
        drop(open);

        // The cloud copy open in a viewer: not replaced from under it.
        let open = fs::File::open(f.there("a.pdf")).unwrap();
        assert_eq!(f.sync(&mut m), busy);
        drop(open);
        assert_eq!(read(&f.there("a.pdf")).as_deref(), Some("aaa"));
        assert_eq!(f.sync(&mut m), updated(1));
        assert_eq!(read(&f.there("a.pdf")).as_deref(), Some("aaa with notes"));
        assert_eq!(files(&f.cloud), ["a.pdf"]);
    }

    /// The cloud app or a virus scanner looking at a brand-new copy right when undo is clicked.
    #[cfg(windows)]
    #[test]
    fn undo_while_the_copy_is_open_is_finished_later() {
        let f = fixture();
        put(&f.here("a.pdf"), "aaa", 300);
        let mut m = f.mirror();
        assert_eq!(f.sync(&mut m), copied(1));
        fs::remove_file(f.here("a.pdf")).unwrap();
        let open = fs::File::open(f.there("a.pdf")).unwrap();
        assert!(f.forget(&mut m, "a.pdf").is_err());
        assert_eq!(f.sync(&mut m), MirrorReport::default());
        assert!(f.there("a.pdf").exists(), "not from under the program");
        drop(open);
        assert_eq!(f.sync(&mut m), MirrorReport::default());
        assert!(!f.there("a.pdf").exists());
        assert!(f.mirror().state.files.is_empty());
    }

    /// Files extracted from archives or copied from a share are often read-only.
    #[cfg(windows)]
    #[test]
    #[allow(clippy::permissions_set_readonly_false)]
    fn read_only_files_get_copies_that_can_be_replaced() {
        let set_readonly = |path: &Path, readonly: bool| {
            let mut permissions = fs::metadata(path).unwrap().permissions();
            permissions.set_readonly(readonly);
            fs::set_permissions(path, permissions).unwrap();
        };
        let f = fixture();
        put(&f.here("a.pdf"), "aaa", 300);
        set_readonly(&f.here("a.pdf"), true);
        let mut m = f.mirror();
        assert_eq!(f.sync(&mut m), copied(1));
        assert!(!fs::metadata(f.there("a.pdf")).unwrap().permissions().readonly());
        set_readonly(&f.here("a.pdf"), false);
        put(&f.here("a.pdf"), "aaa v2", 200);
        assert_eq!(f.sync(&mut m), updated(1));
        assert_eq!(read(&f.there("a.pdf")).as_deref(), Some("aaa v2"));

        // A read-only leftover of a killed run is still cleared away.
        put(&f.there("b.pdf.filesorter-tmp"), "b", 7200);
        set_readonly(&f.there("b.pdf.filesorter-tmp"), true);
        put(&f.here("b.pdf"), "bbb", 300);
        assert_eq!(f.sync(&mut m), copied(1));
        assert_eq!(files(&f.cloud), ["a.pdf", "b.pdf"]);

        // A copy made read-only in the cloud is an error to see, not "in use" for ever.
        set_readonly(&f.there("a.pdf"), true);
        put(&f.here("a.pdf"), "aaa v3", 100);
        let report = f.sync(&mut m);
        assert_eq!(report.in_use, 0, "{report:?}");
        assert!(report.errors.len() == 1 || report.updated == 1, "{report:?}");
        set_readonly(&f.there("a.pdf"), false);
    }

    #[cfg(windows)]
    #[test]
    fn renamed_only_in_case_is_the_same_file() {
        let f = fixture();
        put(&f.here("Mathe/Skript.pdf"), "slides", 300);
        let mut m = f.mirror();
        assert_eq!(f.sync(&mut m), copied(1));
        fs::rename(f.here("Mathe/Skript.pdf"), f.here("Mathe/skript.PDF")).unwrap();
        assert_eq!(f.sync(&mut m), MirrorReport::default());
        assert_eq!(files(&f.cloud), ["Mathe/Skript.pdf"]);
        put(&f.here("Mathe/skript.PDF"), "slides v2", 200);
        assert_eq!(f.sync(&mut m), updated(1));
        assert_eq!(read(&f.there("Mathe/Skript.pdf")).as_deref(), Some("slides v2"));
    }
}
