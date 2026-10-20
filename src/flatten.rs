//! Modules directly in the study folder: move module folders out of their study program's
//! folder (`Studium/Informatik/Statistik` → `Studium/Statistik`), and back.
//!
//! The folders are moved with `cloud::relocate` like the archive's, which never overwrites
//! or loses a file; a module whose new folder is taken already stays where it is.

use std::path::{Path, PathBuf};

use crate::archive::{Moved, files_in, move_folder};
use crate::config::{Config, Course, expand_user, safe_folder_name};

/// One module to move up.
#[derive(Debug, Clone, PartialEq)]
pub struct Step {
    pub course: Course,
    pub from: PathBuf,
    pub to: PathBuf,
}

/// Why a module stays where it is.
#[derive(Debug, Clone, PartialEq)]
pub enum Kept {
    /// Its folder was chosen by hand.
    OwnFolder,
    /// There is a folder of that name in the study folder already.
    Taken(PathBuf),
}

/// What moving the modules up would do: the modules to move (or only to relabel, when they
/// have no folder yet), and the ones that stay.
pub fn plan(config: &Config) -> (Vec<Step>, Vec<(Course, Kept)>) {
    let root = expand_user(config.destination_root.trim());
    let mut steps: Vec<Step> = Vec::new();
    let mut kept = Vec::new();
    for course in config.courses.iter().filter(|c| !c.subject.is_empty()) {
        if !course.folder.is_empty() {
            kept.push((course.clone(), Kept::OwnFolder));
            continue;
        }
        let from = config.course_folder(course);
        let to = root.join(safe_folder_name(&course.name));
        // Two modules of the same name in different programs: the first one gets the name.
        let claimed = steps.iter().any(|s| s.to == to);
        if claimed || (to.exists() && from.is_dir()) {
            kept.push((course.clone(), Kept::Taken(to)));
            continue;
        }
        steps.push(Step { course: course.clone(), from, to });
    }
    (steps, kept)
}

/// A module that was moved up, for «Undo».
#[derive(Debug, Clone, PartialEq)]
pub struct Done {
    pub name: String,
    pub subject: String,
    pub from: PathBuf,
    pub to: PathBuf,
}

/// Move one module's folder up; its program's folder goes once it is empty.
pub fn move_up(step: &Step) -> Result<Moved, String> {
    let files = files_in(&step.from);
    let report = move_folder(&step.from, &step.to)?;
    if let Some(program) = step.from.parent() {
        remove_if_empty(program);
    }
    Ok(Moved { from: step.from.clone(), to: step.to.clone(), report, files })
}

/// Put a moved module's folder back into its program's folder.
pub fn move_back(from: &Path, to: &Path) -> Result<Moved, String> {
    if from.exists() && !from.is_dir() {
        return Err(format!("{} is not a folder", from.display()));
    }
    let files = files_in(from);
    let report = move_folder(from, to)?;
    Ok(Moved { from: from.to_path_buf(), to: to.to_path_buf(), report, files })
}

/// Remove `dir` if nothing is in it (also no hidden files: a folder with something in it stays).
fn remove_if_empty(dir: &Path) {
    if std::fs::read_dir(dir).is_ok_and(|mut entries| entries.next().is_none()) {
        let _ = std::fs::remove_dir(dir);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn modules_move_up_and_back_without_losing_or_overwriting() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("Studium");
        for dir in ["Informatik/Programmieren/Vorlesung", "Mathematik/Statistik", "Mathematik/Analysis", "Analysis"] {
            fs::create_dir_all(root.join(dir)).unwrap();
        }
        fs::write(root.join("Informatik/Programmieren/Vorlesung/vl1.pdf"), "a").unwrap();
        fs::write(root.join("Mathematik/Statistik/s.pdf"), "b").unwrap();
        fs::write(root.join("Analysis/mine.txt"), "c").unwrap();
        let mut config = Config { destination_root: root.to_string_lossy().into(), ..Config::default() };
        config.courses = vec![
            Course::new("Programmieren", &[]).with_subject("Informatik"),
            Course::new("Statistik", &[]).with_subject("Mathematik"),
            Course::new("Analysis", &[]).with_subject("Mathematik"),
            Course::new("Neu", &[]).with_subject("Mathematik"),
            Course::new("Direkt", &[]),
        ];
        let (steps, kept) = plan(&config);
        let names: Vec<&str> = steps.iter().map(|s| s.course.name.as_str()).collect();
        assert_eq!(names, ["Programmieren", "Statistik", "Neu"], "Analysis is taken, Direkt is up already");
        assert_eq!(kept, [(config.courses[2].clone(), Kept::Taken(root.join("Analysis")))]);
        let moved: Vec<Moved> = steps.iter().map(|s| move_up(s).unwrap()).collect();
        assert_eq!(fs::read_to_string(root.join("Programmieren/Vorlesung/vl1.pdf")).unwrap(), "a");
        assert_eq!(fs::read_to_string(root.join("Statistik/s.pdf")).unwrap(), "b");
        assert_eq!(fs::read_to_string(root.join("Analysis/mine.txt")).unwrap(), "c", "not touched");
        assert!(!root.join("Informatik").exists(), "the empty program folder is gone");
        assert!(root.join("Mathematik/Analysis").is_dir(), "Mathematik still has a module");
        assert!(!root.join("Neu").exists(), "a module without a folder gets none");
        // And back.
        for m in moved.iter().rev() {
            move_back(&m.to, &m.from).unwrap();
        }
        assert_eq!(fs::read_to_string(root.join("Informatik/Programmieren/Vorlesung/vl1.pdf")).unwrap(), "a");
        assert_eq!(fs::read_to_string(root.join("Mathematik/Statistik/s.pdf")).unwrap(), "b");
        assert!(!root.join("Programmieren").exists() && !root.join("Statistik").exists());
    }
}
