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
    same_content,
};

#[derive(Debug, Clone, PartialEq)]
pub enum UndoError {
    /// The sorted file isn't where File Sorter put it any more.
    Missing,
    /// Another program has the file open.
    InUse,
    /// Nothing to undo (the file wasn't sorted).
    NotSorted,
    /// Copy mode: the copy was changed (notes added?), so it isn't removed.
    CopyChanged,
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
                // Only a copy that is still the same as the original: notes added to it stay.
                if !same_content(source, destination) {
                    return Err(UndoError::CopyChanged);
                }
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
    undo_extras(result, config);
    keep(&back, config);
    if let (Some(dir), Some(id)) = (&config.state_dir, result.id)
        && let Err(e) = history::record_undo(dir, id)
    {
        log::info!("Could not note the undo in the history: {e}");
    }
    log::info!("Undid sorting {name}: back at {}", back.display());
    Ok(back)
}

/// Reverse what happened along with sorting: older versions go back, and the unpacked
/// folder of a ZIP file is removed, but only if nobody changed anything in it.
fn undo_extras(result: &SortResult, config: &Config) {
    let (stuck, moved) = crate::sorter::put_back_moved(&result.extras.older_versions);
    for path in stuck {
        log::info!("Could not put the older version {} back", path.display());
    }
    if let Some(dir) = &config.state_dir {
        for (from, to) in &moved {
            crate::sorter::note_moved(dir, from, to);
        }
    }
    if let Some(extracted) = &result.extras.extracted
        && extracted.folder.is_dir()
    {
        if crate::unzip::unchanged(extracted) {
            if let Err(e) = crate::sorter::discard_folder(&extracted.folder) {
                log::info!("Could not remove the unpacked folder {}: {e}", extracted.folder.display());
            }
        } else {
            log::info!("Kept {}: files in it were changed or added", extracted.folder.display());
        }
    }
}

/// Move a sorted file into another module (`course`, an index into `config.courses`), or
/// put a file that stayed in the downloads folder into one. Returns the new result.
///
/// A sorted file is first put back (as undo does) and then sorted into the chosen module,
/// so both steps are as safe as sorting and undoing are on their own. If the second step
/// fails, the file stays in the downloads folder and is left alone there.
pub fn reassign(result: &SortResult, course: usize, config: &Config) -> Result<SortResult, UndoError> {
    let path = if result.sorted() {
        // Undo leaves the file alone in the downloads folder, so the watcher doesn't grab it
        // before it is sorted into the chosen module.
        undo(result, config)?
    } else {
        result.source.clone()
    };
    let new = crate::sorter::sort_into(&path, course, config);
    if !new.sorted() && path.exists() {
        keep(&path, config);
    }
    Ok(new)
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
    use crate::sorter::{LeftReason, sort_chosen, sort_existing, sort_file};

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
        assert!(sort_existing(&cfg, crate::config::ExistingScope::All).iter().all(|r| !r.sorted()));
        assert!(src.exists());
        // Renamed afterwards (the new name would match too): still left alone.
        let renamed = cfg.watch_path().join("prog1 mitschrift.pdf");
        fs::rename(&src, &renamed).unwrap();
        let again = sort_file(&renamed, &cfg);
        assert_eq!(again.outcome, Outcome::Left(LeftReason::Kept), "{}", again.describe());
        fs::rename(&renamed, &src).unwrap();
        // The history knows it was undone.
        let entries = history::recent(cfg.state_dir.as_ref().unwrap(), 10);
        assert!(entries.len() == 1 && entries[0].undone);
        // Sorting it explicitly (right-click) still works.
        let chosen = crate::sorter::sort_chosen(std::slice::from_ref(&src), &cfg);
        assert!(chosen[0].sorted(), "{}", chosen[0].describe());
    }

    #[test]
    fn assign_an_unmatched_file_and_move_a_sorted_one_elsewhere() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = config(tmp.path());
        let lost = cfg.watch_path().join("Blatt 3.pdf");
        make_pdf(&lost, "nothing to see");
        let left = sort_file(&lost, &cfg);
        assert_eq!(left.outcome, Outcome::Left(LeftReason::NoMatch));
        let assigned = reassign(&left, 2, &cfg).unwrap();
        assert!(
            matches!(&assigned.outcome, Outcome::Moved { course, reason: crate::matcher::Reason::Chosen, .. }
            if course == "Statistics"),
            "{}",
            assigned.describe()
        );
        assert!(!lost.exists() && assigned.destination().unwrap().exists() && assigned.id.is_some());

        // Sorted into the wrong module: move it into another one, then undo that.
        let src = cfg.watch_path().join("prog1_woche2.pdf");
        make_pdf(&src, "x");
        let wrong = sort_file(&src, &cfg);
        assert_eq!(wrong.course(), Some("Programming Course"));
        let right = reassign(&wrong, 1, &cfg).unwrap();
        assert_eq!(right.course(), Some("Linear Algebra"));
        assert!(!wrong.destination().unwrap().exists() && right.destination().unwrap().exists());
        assert!(!src.exists(), "not left in the downloads folder");
        let entries = history::recent(cfg.state_dir.as_ref().unwrap(), 10);
        assert!(entries.iter().any(|e| e.result.id == wrong.id && e.undone));
        assert_eq!(undo(&right, &cfg).unwrap(), src);
        assert!(src.exists() && !right.destination().unwrap().exists());
        assert_eq!(reassign(&right, 99, &cfg).map(|r| r.sorted()), Err(UndoError::Missing));
    }

    #[test]
    fn subfolders_older_versions_and_undo() {
        let tmp = tempfile::tempdir().unwrap();
        let mut cfg = config(tmp.path());
        cfg.features.subfolders = true;
        cfg.features.versions = true;
        let module = tmp.path().join("University").join("Programming Course");
        // Lecture slides into "Vorlesung", an exercise sheet into "Übungen".
        let slides = cfg.watch_path().join("prog1_VL03.pdf");
        make_pdf(&slides, "x");
        let sorted = sort_file(&slides, &cfg);
        assert_eq!(sorted.destination().unwrap(), module.join("Vorlesung").join("prog1_VL03.pdf"));
        let other = cfg.watch_path().join("prog1 Organisation.pdf");
        make_pdf(&other, "y");
        assert_eq!(sort_file(&other, &cfg).destination().unwrap(), module.join("prog1 Organisation.pdf"));

        // A new version of the exercise sheet: the old one (with the user's notes) moves aside.
        let first = cfg.watch_path().join("prog1_Blatt3.pdf");
        make_pdf(&first, "Blatt 3");
        let first = sort_file(&first, &cfg).destination().unwrap().to_path_buf();
        fs::write(&first, "with my notes").unwrap();
        let second = cfg.watch_path().join("prog1_Blatt3_v2.pdf");
        make_pdf(&second, "Blatt 3, corrected");
        let result = sort_file(&second, &cfg);
        let aside = module.join("Übungen").join("Ältere Versionen").join("prog1_Blatt3.pdf");
        assert_eq!(result.extras.older_versions, vec![(first.clone(), aside.clone())]);
        assert_eq!(fs::read_to_string(&aside).unwrap(), "with my notes");
        assert!(!first.exists());
        // An even older one arriving later goes straight to the side.
        let old = cfg.watch_path().join("prog1_Blatt3_v1.pdf");
        make_pdf(&old, "Blatt 3, very first");
        let late = sort_file(&old, &cfg);
        assert_eq!(
            late.destination().unwrap(),
            module.join("Übungen").join("Ältere Versionen").join("prog1_Blatt3_v1.pdf")
        );
        assert!(late.extras.older_versions.is_empty());
        // Undo puts the new one back into the downloads folder and the old one where it was.
        assert_eq!(undo(&result, &cfg).unwrap(), second);
        assert_eq!(fs::read_to_string(&first).unwrap(), "with my notes");
        assert!(!aside.exists());
        // A file already in the module (or one of its subfolders) stays.
        let inside = sort_chosen(std::slice::from_ref(&first), &cfg);
        assert!(matches!(inside[0].outcome, Outcome::Left(LeftReason::AlreadyInFolder { .. })));
    }

    #[test]
    fn zip_files_are_sorted_by_their_content_unpacked_and_undone() {
        use std::io::Write;
        let tmp = tempfile::tempdir().unwrap();
        let mut cfg = config(tmp.path());
        let zip_path = cfg.watch_path().join("Download.zip");
        let make = |path: &Path| {
            let mut zip = zip::ZipWriter::new(fs::File::create(path).unwrap());
            let options = zip::write::SimpleFileOptions::default();
            zip.start_file("stats_uebung/aufgabe1.R", options).unwrap();
            zip.write_all(b"mean(x)").unwrap();
            zip.start_file("stats_uebung/daten.csv", options).unwrap();
            zip.write_all(b"1,2,3").unwrap();
            zip.finish().unwrap();
        };
        make(&zip_path);
        // Without unpacking it is still sorted by the files in it, but stays packed.
        let packed = sort_file(&zip_path, &cfg);
        assert_eq!(packed.course(), Some("Statistics"), "{}", packed.describe());
        assert!(packed.extras.extracted.is_none());
        undo(&packed, &cfg).unwrap();
        crate::sorter::tests::forget_kept(&cfg);
        cfg.features.unzip = true;
        let result = sort_file(&zip_path, &cfg);
        assert_eq!(result.course(), Some("Statistics"), "{}", result.describe());
        let extracted = result.extras.extracted.clone().unwrap();
        let folder = tmp.path().join("University").join("Statistics").join("Download");
        assert_eq!(extracted.folder, folder);
        assert_eq!(fs::read_to_string(folder.join("daten.csv")).unwrap(), "1,2,3");
        assert!(result.destination().unwrap().is_file(), "the ZIP file is kept");
        // Undo: the ZIP goes back, the unpacked folder away.
        assert_eq!(undo(&result, &cfg).unwrap(), zip_path);
        assert!(zip_path.is_file() && !folder.exists());

        // Changed after unpacking: the folder stays.
        let again = crate::sorter::sort_chosen(std::slice::from_ref(&zip_path), &cfg);
        let folder = again[0].extras.extracted.as_ref().unwrap().folder.clone();
        fs::write(folder.join("my notes.txt"), "keep me").unwrap();
        undo(&again[0], &cfg).unwrap();
        assert!(folder.join("my notes.txt").exists());
    }

    #[test]
    fn undoing_an_earlier_sort_finds_the_version_that_was_moved_aside() {
        let tmp = tempfile::tempdir().unwrap();
        let mut cfg = config(tmp.path());
        cfg.features.versions = true;
        let module = tmp.path().join("University").join("Programming Course");
        let download = cfg.watch_path().join("prog1_Skript.pdf");
        make_pdf(&download, "first");
        let first = sort_file(&download, &cfg);
        make_pdf(&download, "second, a new version under the same name");
        let second = sort_file(&download, &cfg);
        assert_eq!(second.destination().unwrap(), module.join("prog1_Skript.pdf"));
        // The first entry, read back from the history, points into the versions folder now.
        let entries = history::recent(cfg.state_dir.as_ref().unwrap(), 10);
        let earlier = entries.iter().find(|e| e.result.id == first.id).unwrap().result.clone();
        let aside = module.join("Ältere Versionen").join("prog1_Skript.pdf");
        assert_eq!(earlier.destination().unwrap(), aside);
        // Undoing it puts the first version back, and the new one stays in the module.
        let back = undo(&earlier, &cfg).unwrap();
        assert!(fs::read(&back).unwrap().windows(5).any(|w| w == b"first"));
        assert!(fs::read(module.join("prog1_Skript.pdf")).unwrap().windows(6).any(|w| w == b"second"));
    }

    #[test]
    fn a_changed_copy_is_not_removed() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = Config { action: FileAction::Copy, ..config(tmp.path()) };
        let src = cfg.watch_path().join("stats_blatt2.pdf");
        make_pdf(&src, "Blatt 2");
        let result = sort_file(&src, &cfg);
        let copy = result.destination().unwrap().to_path_buf();
        fs::write(&copy, "with my notes").unwrap();
        assert_eq!(undo(&result, &cfg), Err(UndoError::CopyChanged));
        assert_eq!(reassign(&result, 0, &cfg).map(|r| r.sorted()), Err(UndoError::CopyChanged));
        assert_eq!(fs::read_to_string(&copy).unwrap(), "with my notes");
    }

    #[test]
    fn assign_keeps_a_file_that_cannot_move_now() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = config(tmp.path());
        let lost = cfg.watch_path().join("Blatt 4.pdf");
        make_pdf(&lost, "x");
        let left = sort_file(&lost, &cfg);
        let failed = reassign(&left, 99, &cfg).unwrap();
        assert!(!failed.sorted() && lost.exists());
        assert_eq!(sort_file(&lost, &cfg).outcome, Outcome::Left(LeftReason::Kept), "left alone");
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
