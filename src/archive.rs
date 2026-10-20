//! Finish a semester: move modules that are over into `Studium/Archiv/HS2026/…`, so the
//! list of modules and the study folder only show the current ones. An archived module
//! can be brought back, folder included.
//!
//! The folders are moved with `cloud::relocate`, which never overwrites or loses a file
//! (files open in another program stay where they are and are reported).

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use chrono::{Datelike, NaiveDate};
use serde::{Deserialize, Serialize};

use crate::cloud::relocate::{Report, relocate};
use crate::config::{Config, Course, expand_user};

/// The archive's folder directly in the study folder.
pub const ARCHIVE_FOLDER: &str = "Archiv";

/// A module of a finished semester.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Archived {
    pub course: Course,
    /// "HS2026", or what the user called it.
    pub semester: String,
    /// Where its folder is now, and where it was.
    pub folder: PathBuf,
    pub original: PathBuf,
    /// Its folder was moved into the archive (a module without files had none).
    #[serde(default)]
    pub moved: bool,
}

/// The semester in Swiss naming: autumn semester (HS, Herbstsemester) from August to
/// January, spring semester (FS, Frühjahrssemester) from February to July.
pub fn current_semester(today: NaiveDate) -> String {
    match today.month() {
        8..=12 => format!("HS{}", today.year()),
        1 => format!("HS{}", today.year() - 1),
        _ => format!("FS{}", today.year()),
    }
}

/// Where `course`'s folder goes in the archive: `Archiv/<semester>/` plus the same path it
/// had in the study folder ("Informatik/Programmieren"), or just its name if it was elsewhere.
pub fn archive_folder(config: &Config, course: &Course, semester: &str) -> PathBuf {
    let root = expand_user(config.destination_root.trim());
    let folder = config.course_folder(course);
    let relative = folder
        .strip_prefix(&root)
        .map(Path::to_path_buf)
        .unwrap_or_else(|_| PathBuf::from(folder.file_name().unwrap_or_default()));
    let semester = crate::categories::safe_folder_name(semester);
    let semester = if semester.is_empty() { "Semester".to_string() } else { semester };
    root.join(ARCHIVE_FOLDER).join(semester).join(relative)
}

/// What happened when archiving or bringing back a module.
#[derive(Debug)]
pub struct Moved {
    pub from: PathBuf,
    pub to: PathBuf,
    /// `None` when the module had no folder yet.
    pub report: Option<Report>,
    /// The files that were in `from` before (so their copies in the cloud can follow).
    pub files: Vec<PathBuf>,
}

pub(crate) fn files_in(folder: &Path) -> Vec<PathBuf> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        for entry in entries.flatten() {
            let path = entry.path();
            match entry.file_type() {
                Ok(t) if t.is_dir() => walk(&path, out),
                Ok(t) if t.is_file() => out.push(path),
                _ => {}
            }
        }
    }
    let mut out = Vec::new();
    walk(folder, &mut out);
    out
}

/// Move one module's folder into the archive. The module itself is taken out of the
/// list by the caller (with the returned `Archived`).
pub fn archive(config: &Config, course: &Course, semester: &str) -> Result<(Archived, Moved), String> {
    let from = config.course_folder(course);
    // Never into a folder that is there already (another module with the same name).
    let wanted = archive_folder(config, course, semester);
    let mut to = wanted.clone();
    let name = wanted.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let mut n = 1;
    while to.exists() {
        to = wanted.with_file_name(format!("{name} ({n})"));
        n += 1;
    }
    let files = files_in(&from);
    let report = move_folder(&from, &to)?;
    let archived = Archived {
        course: course.clone(),
        semester: semester.trim().to_string(),
        folder: to.clone(),
        original: from.clone(),
        moved: report.is_some(),
    };
    Ok((archived, Moved { from, to, report, files }))
}

/// Bring an archived module's folder back to where it was.
pub fn restore(archived: &Archived) -> Result<Moved, String> {
    let (from, to) = (archived.folder.clone(), archived.original.clone());
    if archived.moved && !from.is_dir() {
        return Err(format!("the archived folder {} is not there", from.display()));
    }
    let files = files_in(&from);
    let report = move_folder(&from, &to)?;
    // The semester's folder and "Archiv" itself go once they are empty (only empty folders).
    let in_archive = from.ancestors().any(|d| d.file_name().is_some_and(|n| n == ARCHIVE_FOLDER));
    for dir in from.ancestors().skip(1).filter(|_| in_archive) {
        let name = dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        if std::fs::remove_dir(dir).is_err() || name == ARCHIVE_FOLDER {
            break;
        }
    }
    Ok(Moved { from, to, report, files })
}

pub(crate) fn move_folder(from: &Path, to: &Path) -> Result<Option<Report>, String> {
    if !from.is_dir() {
        return Ok(None);
    }
    let cancel = AtomicBool::new(false);
    relocate(from, to, &cancel, &mut |_| {}).map(Some).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn swiss_semesters() {
        let day = |y, m, d| NaiveDate::from_ymd_opt(y, m, d).unwrap();
        assert_eq!(current_semester(day(2026, 10, 19)), "HS2026");
        assert_eq!(current_semester(day(2027, 1, 20)), "HS2026");
        assert_eq!(current_semester(day(2027, 3, 1)), "FS2027");
        assert_eq!(current_semester(day(2027, 8, 1)), "HS2027");
    }

    #[test]
    fn archive_and_bring_back() {
        let tmp = tempfile::tempdir().unwrap();
        let mut config = crate::sorter::tests::test_config(tmp.path());
        let course = Course::new("Programmieren", &["prog1"]).with_subject("Informatik");
        config.courses = vec![course.clone()];
        let folder = config.course_folder(&course);
        fs::create_dir_all(folder.join("Übungen")).unwrap();
        fs::write(folder.join("Übungen").join("Blatt1.pdf"), "notes").unwrap();

        let (archived, moved) = archive(&config, &course, "HS2026").unwrap();
        let expected =
            tmp.path().join("University").join("Archiv").join("HS2026").join("Informatik").join("Programmieren");
        assert_eq!(archived.folder, expected);
        assert_eq!(moved.report.unwrap().moved, 1);
        assert_eq!(fs::read_to_string(expected.join("Übungen").join("Blatt1.pdf")).unwrap(), "notes");
        assert!(!folder.exists());
        // Scanning the study folder doesn't take the archive for a study program.
        assert!(crate::config::scan_study_folder(&tmp.path().join("University"), false, &[]).is_empty());

        // Meanwhile a new file with the same name appeared in the old place: both are kept.
        fs::create_dir_all(folder.join("Übungen")).unwrap();
        fs::write(folder.join("Übungen").join("Blatt1.pdf"), "new").unwrap();
        let back = restore(&archived).unwrap();
        assert_eq!(back.report.unwrap().renamed.len(), 1);
        assert_eq!(fs::read_to_string(folder.join("Übungen").join("Blatt1.pdf")).unwrap(), "new");
        assert_eq!(fs::read_to_string(folder.join("Übungen").join("Blatt1 (1).pdf")).unwrap(), "notes");
        assert!(!tmp.path().join("University").join(ARCHIVE_FOLDER).exists(), "empty archive folders go");

        // Another module with the same folder name in the same semester gets its own folder.
        let mut other = Course::new("Programmieren", &[]).with_subject("Informatik");
        other.folder = tmp.path().join("elsewhere").join("Programmieren").display().to_string();
        let (a, _) = archive(&config, &course, "FS2027").unwrap();
        fs::create_dir_all(config.course_folder(&other)).unwrap();
        fs::write(config.course_folder(&other).join("x.pdf"), "x").unwrap();
        fs::create_dir_all(&a.folder).unwrap();
        let (b, _) = archive(&config, &other, "FS2027").unwrap();
        assert_ne!(a.folder, b.folder);
        // Gone from the archive (study folder moved, deleted by hand): no false «restored».
        fs::remove_dir_all(&b.folder).unwrap();
        assert!(restore(&b).is_err());

        // A module without a folder is archived without moving anything.
        let empty = Course::new("Leer", &[]);
        assert!(archive(&config, &empty, "HS2026").unwrap().1.report.is_none());
    }
}
