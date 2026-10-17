//! Undo a sort: put the file back where it came from, and leave it there from then on.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::config::Config;
use crate::history;
use crate::ledger::{COPIED_LIST, CopyLedger, KEPT_LIST, KeptFiles};
use crate::sorter::{
    IN_USE_RETRY, MoveError, Outcome, SortResult, copy_file_with_patience, discard, ensure_not_open_elsewhere,
    file_name, in_use, move_file_with_patience, release_placeholder, reserve_destination, retry_while_in_use,
};

#[derive(Debug, Clone, PartialEq)]
pub enum UndoError {
    /// The sorted file isn't where File Sorter put it any more.
    Missing,
    /// Another program has the file open.
    InUse,
    /// Nothing to undo (the file wasn't sorted).
    NotSorted,
    Failed(String),
}

impl From<io::Error> for UndoError {
    fn from(e: io::Error) -> Self {
        if in_use(&e) { UndoError::InUse } else { UndoError::Failed(e.to_string()) }
    }
}

impl From<MoveError> for UndoError {
    fn from(e: MoveError) -> Self {
        match e {
            MoveError::InUse(_) => UndoError::InUse,
            MoveError::Failed(e) => UndoError::Failed(e.to_string()),
        }
    }
}

/// Put `result`'s file back. Returns where it is now (its old name, or "name (1)" if a
/// new file took that name meanwhile).
pub fn undo(result: &SortResult, config: &Config) -> Result<PathBuf, UndoError> {
    let source = &result.source;
    let dir = source.parent().ok_or(UndoError::NotSorted)?;
    let name = file_name(source);
    let back = match &result.outcome {
        Outcome::Moved { destination, .. } => {
            if !destination.is_file() {
                return Err(UndoError::Missing);
            }
            retry_while_in_use(IN_USE_RETRY, || ensure_not_open_elsewhere(destination))?;
            fs::create_dir_all(dir)?;
            let back = reserve_destination(dir, &name)?;
            if let Err(e) = move_file_with_patience(destination, &back, IN_USE_RETRY) {
                release_placeholder(&back);
                return Err(e.into());
            }
            back
        }
        Outcome::DuplicateRemoved { existing, .. } => {
            // The removed download was byte for byte the same as this file.
            if !existing.is_file() {
                return Err(UndoError::Missing);
            }
            fs::create_dir_all(dir)?;
            let back = reserve_destination(dir, &name)?;
            if let Err(e) = copy_file_with_patience(existing, &back, IN_USE_RETRY) {
                release_placeholder(&back);
                return Err(e.into());
            }
            back
        }
        Outcome::Copied { destination, .. } => {
            // Only remove the copy while the original still exists, or the file would be gone.
            if !source.is_file() {
                return Err(UndoError::Missing);
            }
            if destination.is_file() {
                retry_while_in_use(IN_USE_RETRY, || ensure_not_open_elsewhere(destination))?;
                retry_while_in_use(IN_USE_RETRY, || discard(destination))?;
            }
            if let Some(file) = config.state_file(COPIED_LIST) {
                let mut ledger = CopyLedger::load(&file);
                if ledger.forget(source) {
                    let _ = ledger.save(&file);
                }
            }
            source.clone()
        }
        Outcome::Left(_) => return Err(UndoError::NotSorted),
    };
    keep(&back, config);
    if let (Some(dir), Some(id)) = (&config.state_dir, result.id)
        && let Err(e) = history::record_undo(dir, id)
    {
        log::info!("Could not note the undo in the history: {e}");
    }
    log::info!("Undid sorting {name}: back at {}", back.display());
    Ok(back)
}

/// Leave this file where it is from now on.
fn keep(path: &Path, config: &Config) {
    if let Some(file) = config.state_file(KEPT_LIST) {
        let mut kept = KeptFiles::load(&file);
        kept.keep(path);
        if let Err(e) = kept.save(&file) {
            log::info!("Could not update the list of files to leave alone: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::FileAction;
    use crate::sorter::tests::{make_pdf, test_config};
    use crate::sorter::{LeftReason, sort_existing, sort_file};

    fn config(root: &Path) -> Config {
        Config { state_dir: Some(root.join("state")), ..test_config(root) }
    }

    #[test]
    fn undo_a_move_puts_it_back_for_good() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = config(tmp.path());
        let src = cfg.watch_path().join("prog1_vl2.pdf");
        make_pdf(&src, "Week 2");
        let result = sort_file(&src, &cfg);
        assert!(matches!(result.outcome, Outcome::Moved { .. }), "{}", result.describe());
        assert!(result.id.is_some(), "recorded in the history");

        assert_eq!(undo(&result, &cfg).unwrap(), src);
        assert!(src.exists() && !result.destination().unwrap().exists());
        // It stays: the watcher and "sort existing" leave it alone now.
        let again = sort_file(&src, &cfg);
        assert_eq!(again.outcome, Outcome::Left(LeftReason::Kept), "{}", again.describe());
        assert!(sort_existing(&cfg).iter().all(|r| !r.sorted()));
        assert!(src.exists());
        // The history knows it was undone.
        let entries = history::recent(cfg.state_dir.as_ref().unwrap(), 10);
        assert!(entries.len() == 1 && entries[0].undone);
        // Sorting it explicitly (right-click) still works.
        let chosen = crate::sorter::sort_chosen(std::slice::from_ref(&src), &cfg);
        assert!(chosen[0].sorted(), "{}", chosen[0].describe());
    }

    #[test]
    fn undo_when_the_old_name_is_taken_uses_a_new_one() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = config(tmp.path());
        let src = cfg.watch_path().join("prog1_vl3.pdf");
        make_pdf(&src, "Week 3");
        let result = sort_file(&src, &cfg);
        fs::write(&src, "a new file with the same name").unwrap();
        let back = undo(&result, &cfg).unwrap();
        assert_eq!(file_name(&back), "prog1_vl3 (1).pdf");
        assert_eq!(fs::read_to_string(&src).unwrap(), "a new file with the same name");
    }

    #[test]
    fn undo_a_removed_duplicate_restores_it() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = config(tmp.path());
        let folder = tmp.path().join("University").join("Programming Course");
        fs::create_dir_all(&folder).unwrap();
        make_pdf(&folder.join("python_vl1.pdf"), "same");
        let src = cfg.watch_path().join("python_vl1.pdf");
        make_pdf(&src, "same");
        let result = sort_file(&src, &cfg);
        assert!(matches!(result.outcome, Outcome::DuplicateRemoved { .. }), "{}", result.describe());
        assert!(!src.exists());
        assert_eq!(undo(&result, &cfg).unwrap(), src);
        assert_eq!(fs::read(&src).unwrap(), fs::read(folder.join("python_vl1.pdf")).unwrap());
        assert_eq!(sort_file(&src, &cfg).outcome, Outcome::Left(LeftReason::Kept));
    }

    #[test]
    fn undo_a_copy_removes_it_and_keeps_the_original() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = Config { action: FileAction::Copy, ..config(tmp.path()) };
        let src = cfg.watch_path().join("stats_blatt1.pdf");
        make_pdf(&src, "Blatt 1");
        let result = sort_file(&src, &cfg);
        let copy = result.destination().unwrap().to_path_buf();
        assert!(copy.exists());
        assert_eq!(undo(&result, &cfg).unwrap(), src);
        assert!(src.exists() && !copy.exists());
        assert_eq!(sort_file(&src, &cfg).outcome, Outcome::Left(LeftReason::Kept));
    }

    #[test]
    fn missing_files_and_unsorted_results_are_reported() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = config(tmp.path());
        let src = cfg.watch_path().join("prog1_gone.pdf");
        make_pdf(&src, "x");
        let result = sort_file(&src, &cfg);
        fs::remove_file(result.destination().unwrap()).unwrap();
        assert_eq!(undo(&result, &cfg), Err(UndoError::Missing));
        assert!(!src.exists(), "nothing was invented");

        let other = cfg.watch_path().join("holiday.pdf");
        make_pdf(&other, "Beach");
        assert_eq!(undo(&sort_file(&other, &cfg), &cfg), Err(UndoError::NotSorted));
    }
}
