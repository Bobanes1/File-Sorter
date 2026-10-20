//! Subfolders for files already in the modules: create the subfolders in every module, and
//! move the files lying directly in a module folder into the subfolder they belong in.
//!
//! Nothing is overwritten (a name that is taken gets "(1)"), files open in another program
//! stay, and every move is noted in the history so earlier entries (undo, deadlines) follow
//! the file. Moving them back is one step too.

use std::fs;
use std::path::PathBuf;
use std::time::Duration;

use crate::config::{Config, Course};
use crate::sorter::{
    ensure_not_open_elsewhere, file_name, is_candidate, move_file_with_patience, note_moved, put_back_moved,
    release_placeholder, reserve_destination, subfolder_for,
};

/// How long to wait for a file another program has open before leaving it where it is.
const PATIENCE: Duration = Duration::from_secs(1);

/// Create the subfolders of the list in `course`'s folder (and that folder). Returns how many
/// folders were new.
pub fn create_for(config: &Config, course: &Course) -> usize {
    let folder = config.course_folder(course);
    let mut made = 0;
    for sub in &config.features.subfolder_list {
        let name = crate::categories::safe_folder_name(&sub.name);
        if name.is_empty() {
            continue;
        }
        let path = folder.join(name);
        if !path.exists() && fs::create_dir_all(&path).is_ok() {
            made += 1;
        }
    }
    made
}

/// Create the subfolders in every module. Returns how many folders were new.
pub fn create_all(config: &Config) -> usize {
    config.courses.iter().map(|course| create_for(config, course)).sum()
}

/// A file that would move into a subfolder.
#[derive(Debug, Clone, PartialEq)]
pub struct Planned {
    pub file: PathBuf,
    /// The subfolder it goes into.
    pub folder: PathBuf,
    /// The module's label ("Informatik › Programmieren").
    pub module: String,
}

/// The files lying directly in a module folder that belong in one of its subfolders.
pub fn plan(config: &Config) -> Vec<Planned> {
    let mut planned = Vec::new();
    for course in &config.courses {
        let folder = config.course_folder(course);
        let Ok(entries) = fs::read_dir(&folder) else { continue };
        let mut files: Vec<PathBuf> = entries
            .flatten()
            .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
            .map(|e| e.path())
            .filter(|p| is_candidate(p, config) && !crate::cloud::is_junk(&file_name(p)))
            .collect();
        files.sort();
        for file in files {
            if let Some(sub) = subfolder_for(config, &folder, &file, None) {
                planned.push(Planned { file, folder: folder.join(sub), module: course.label() });
            }
        }
    }
    planned
}

#[derive(Debug, Default)]
pub struct Report {
    /// (where it was, where it is now).
    pub moved: Vec<(PathBuf, PathBuf)>,
    /// Open in another program: left where it was.
    pub open: Vec<PathBuf>,
    pub failed: Vec<(PathBuf, String)>,
}

/// Move the planned files. Files that are gone or open elsewhere stay; nothing is overwritten.
pub fn apply(planned: &[Planned], config: &Config) -> Report {
    let mut report = Report::default();
    for p in planned {
        if !p.file.is_file() {
            continue;
        }
        if ensure_not_open_elsewhere(&p.file).is_err() {
            report.open.push(p.file.clone());
            continue;
        }
        if let Err(e) = fs::create_dir_all(&p.folder) {
            report.failed.push((p.file.clone(), e.to_string()));
            continue;
        }
        let to = match reserve_destination(&p.folder, &file_name(&p.file)) {
            Ok(to) => to,
            Err(e) => {
                report.failed.push((p.file.clone(), e.to_string()));
                continue;
            }
        };
        match move_file_with_patience(&p.file, &to, PATIENCE) {
            Ok(()) => {
                if let Some(dir) = &config.state_dir {
                    note_moved(dir, &p.file, &to);
                }
                report.moved.push((p.file.clone(), to));
            }
            Err(e) => {
                release_placeholder(&to);
                report.failed.push((p.file.clone(), e.to_string()));
            }
        }
    }
    report
}

/// Move files back where they were (each to its old name, or "name (1)" if that is taken
/// now). Returns (from, to) of those moved back, and those that couldn't be.
pub fn undo(moved: &[(PathBuf, PathBuf)], config: &Config) -> (Vec<(PathBuf, PathBuf)>, Vec<PathBuf>) {
    let (stuck, back) = put_back_moved(moved);
    if let Some(dir) = &config.state_dir {
        for (from, to) in &back {
            note_moved(dir, from, to);
        }
    }
    (back, stuck)
}

/// How many planned files go where: (module, subfolder name, count), in order.
pub fn summary(planned: &[Planned]) -> Vec<(String, String, usize)> {
    let mut out: Vec<(String, String, usize)> = Vec::new();
    for p in planned {
        let sub = p.folder.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        match out.iter_mut().find(|(m, s, _)| *m == p.module && *s == sub) {
            Some(entry) => entry.2 += 1,
            None => out.push((p.module.clone(), sub, 1)),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sorter::tests::{make_pdf, test_config};

    #[test]
    fn create_plan_move_and_undo() {
        let tmp = tempfile::tempdir().unwrap();
        let mut config = test_config(tmp.path());
        config.state_dir = Some(tmp.path().join("state"));
        config.features.subfolders = true;
        assert_eq!(create_all(&config), 12, "4 subfolders in 3 modules");
        assert_eq!(create_all(&config), 0, "already there");
        let module = config.course_folder(&config.courses[0]);
        assert!(module.join("Übungen").is_dir());

        make_pdf(&module.join("VL03_Rekursion.pdf"), "slides");
        make_pdf(&module.join("Blatt 2.pdf"), "sheet");
        make_pdf(&module.join("Organisation.pdf"), "nothing");
        fs::create_dir(module.join("Projekt")).unwrap();
        make_pdf(&module.join("Projekt_Anleitung.pdf"), "project");
        // A different file with the same name is already in the subfolder: kept, the new one gets "(1)".
        fs::write(module.join("Übungen").join("Blatt 2.pdf"), "mine").unwrap();

        let planned = plan(&config);
        let names: Vec<_> = planned.iter().map(|p| (file_name(&p.file), file_name(&p.folder))).collect();
        assert_eq!(
            names,
            [
                ("Blatt 2.pdf".to_string(), "Übungen".to_string()),
                ("Projekt_Anleitung.pdf".to_string(), "Projekt".to_string()),
                ("VL03_Rekursion.pdf".to_string(), "Vorlesung".to_string()),
            ]
        );
        assert_eq!(summary(&planned).len(), 3);
        let report = apply(&planned, &config);
        assert_eq!(report.moved.len(), 3);
        assert_eq!(fs::read_to_string(module.join("Übungen").join("Blatt 2.pdf")).unwrap(), "mine");
        assert!(module.join("Übungen").join("Blatt 2 (1).pdf").is_file());
        assert!(module.join("Organisation.pdf").is_file(), "no subfolder fits: stays");
        assert!(plan(&config).is_empty(), "nothing left to do");

        let (back, stuck) = undo(&report.moved, &config);
        assert_eq!((back.len(), stuck.len()), (3, 0));
        assert!(module.join("VL03_Rekursion.pdf").is_file() && module.join("Blatt 2.pdf").is_file());
        assert_eq!(fs::read_to_string(module.join("Übungen").join("Blatt 2.pdf")).unwrap(), "mine");
    }

    #[test]
    fn history_entries_follow_the_moved_file() {
        let tmp = tempfile::tempdir().unwrap();
        let mut config = test_config(tmp.path());
        config.state_dir = Some(tmp.path().join("state"));
        let download = config.watch_path().join("prog1_VL04.pdf");
        make_pdf(&download, "x");
        let sorted = crate::sorter::sort_file(&download, &config);
        let module = config.course_folder(&config.courses[0]);
        assert_eq!(sorted.destination().unwrap(), module.join("prog1_VL04.pdf"));
        // Subfolders switched on later: the file moves, and its undo still works.
        config.features.subfolders = true;
        let report = apply(&plan(&config), &config);
        assert_eq!(report.moved.len(), 1);
        let entries = crate::history::recent(config.state_dir.as_ref().unwrap(), 10);
        let entry = entries.iter().find(|e| e.result.id == sorted.id).unwrap();
        assert_eq!(entry.result.destination().unwrap(), module.join("Vorlesung").join("prog1_VL04.pdf"));
        assert_eq!(crate::undo::undo(&entry.result, &config).unwrap(), download);
    }
}
