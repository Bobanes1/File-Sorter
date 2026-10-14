//! Match a single file to a module and move it into that module's folder.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::config::Config;
use crate::extract::extract_text;
use crate::matcher::match_file;

/// Partial downloads written by browsers before the file is complete.
pub const TEMP_SUFFIXES: &[&str] = &[".crdownload", ".part", ".partial", ".download", ".tmp", ".temp", ".opdownload"];

#[derive(Debug, Clone, PartialEq)]
pub struct SortResult {
    pub source: PathBuf,
    pub moved: bool,
    pub destination: Option<PathBuf>,
    pub course: Option<String>,
    pub reason: String,
}

impl SortResult {
    fn left(source: &Path, reason: impl Into<String>) -> Self {
        SortResult {
            source: source.to_path_buf(),
            moved: false,
            destination: None,
            course: None,
            reason: reason.into(),
        }
    }

    pub fn file_name(&self) -> String {
        file_name(&self.source)
    }

    pub fn describe(&self) -> String {
        if self.moved {
            format!("Moved {} → {} ({})", self.file_name(), self.course.as_deref().unwrap_or(""), self.reason)
        } else {
            format!("Left {} in place – {}", self.file_name(), self.reason)
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
    !TEMP_SUFFIXES.contains(&ext.as_str()) && config.extensions.contains(&ext)
}

/// folder/filename, or folder/"name (1).ext" etc. if that already exists.
pub fn unique_destination(folder: &Path, filename: &str) -> PathBuf {
    let target = folder.join(filename);
    if !target.exists() {
        return target;
    }
    let path = Path::new(filename);
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let ext = path.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    (1..)
        .map(|n| folder.join(format!("{stem} ({n}){ext}")))
        .find(|candidate| !candidate.exists())
        .expect("an unused name always exists")
}

/// Move a file, also across drives (e.g. Downloads on C: and the study folder on D: or OneDrive).
pub fn move_file(from: &Path, to: &Path) -> io::Result<()> {
    match fs::rename(from, to) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Err(e),
        Err(_) => {
            fs::copy(from, to)?;
            if let Err(e) = fs::remove_file(from) {
                // Don't leave the file in two places if the original is locked.
                let _ = fs::remove_file(to);
                return Err(e);
            }
            Ok(())
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
    if !path.is_file() {
        return SortResult::left(path, "file no longer exists");
    }
    if !is_candidate(path, config) {
        return SortResult::left(path, "not a course file type");
    }
    let Some(m) = match_file(path, &config.courses, config.read_file_contents, extract_text) else {
        return SortResult::left(path, "no matching module");
    };
    let course = &config.courses[m.course];
    let folder = config.course_folder(course);
    if let Some(parent) = path.parent()
        && same_dir(&folder, parent)
    {
        return SortResult { course: Some(course.label()), ..SortResult::left(path, "already in module folder") };
    }
    let target = match fs::create_dir_all(&folder) {
        Ok(()) => unique_destination(&folder, &file_name(path)),
        Err(e) => return SortResult::left(path, format!("error: cannot create {}: {e}", folder.display())),
    };
    if let Err(e) = move_file(path, &target) {
        return SortResult::left(path, format!("error: {e}"));
    }
    SortResult {
        source: path.to_path_buf(),
        moved: true,
        destination: Some(target),
        course: Some(course.label()),
        reason: m.reason,
    }
}

/// Sort every file currently sitting in the watch folder.
pub fn sort_existing(config: &Config) -> Vec<SortResult> {
    let Ok(entries) = fs::read_dir(config.watch_path()) else { return Vec::new() };
    let mut files: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
    files.sort();
    files.into_iter().filter(|p| p.is_file() && is_candidate(p, config)).map(|p| sort_file(&p, config)).collect()
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
    fn moves_matching_file() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = test_config(tmp.path());
        let src = cfg.watch_path().join("prog1_slides.pdf");
        make_pdf(&src, "Week 1");
        let result = sort_file(&src, &cfg);
        assert!(result.moved, "{}", result.describe());
        let dest = tmp.path().join("University").join("Programming Course").join("prog1_slides.pdf");
        assert_eq!(result.destination.as_deref(), Some(dest.as_path()));
        assert!(dest.exists() && !src.exists());
    }

    #[test]
    fn sorts_by_content() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = test_config(tmp.path());
        let src = cfg.watch_path().join("lecture5.pdf");
        fs::copy(fixture("linear_algebra.pdf"), &src).unwrap();
        assert_eq!(sort_file(&src, &cfg).course.as_deref(), Some("Linear Algebra"));
        let src = cfg.watch_path().join("week3.pptx");
        fs::copy(fixture("programming.pptx"), &src).unwrap();
        assert_eq!(sort_file(&src, &cfg).course.as_deref(), Some("Programming Course"));
        let src = cfg.watch_path().join("sheet.docx");
        fs::copy(fixture("statistics.docx"), &src).unwrap();
        assert_eq!(sort_file(&src, &cfg).course.as_deref(), Some("Statistics"));
    }

    #[test]
    fn unmatched_file_stays() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = test_config(tmp.path());
        let src = cfg.watch_path().join("holiday.pdf");
        make_pdf(&src, "Beach trip");
        let result = sort_file(&src, &cfg);
        assert!(!result.moved && src.exists());
        assert_eq!(result.describe(), "Left holiday.pdf in place – no matching module");
    }

    #[test]
    fn name_collision_gets_number() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("a.pdf"), "x").unwrap();
        fs::write(tmp.path().join("a (1).pdf"), "x").unwrap();
        assert_eq!(file_name(&unique_destination(tmp.path(), "a.pdf")), "a (2).pdf");
        assert_eq!(file_name(&unique_destination(tmp.path(), "b.pdf")), "b.pdf");
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
        assert_eq!(file_name(result.destination.as_ref().unwrap()), "prog1 (1).pdf");
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
        let results = sort_existing(&cfg);
        let get = |n: &str| results.iter().find(|r| r.file_name() == n).unwrap();
        assert_eq!(get("stats_hw.pdf").course.as_deref(), Some("Statistics"));
        assert!(!get("random.pdf").moved);
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
        assert_eq!(r.course.as_deref(), Some("Mathematik › Linear Algebra"));
        assert_eq!(r.destination.unwrap().parent().unwrap(), tmp.path().join("University/Mathematik/Linear Algebra"));
        let b = cfg.watch_path().join("stats.pdf");
        make_pdf(&b, "x");
        assert_eq!(sort_file(&b, &cfg).destination.unwrap().parent().unwrap(), tmp.path().join("Elsewhere/Stats"));
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
            sort_file(&p, &cfg).destination.map(|d| d.parent().unwrap().to_path_buf())
        };
        assert_eq!(sort("Analysis_II_Blatt3.pdf", "x"), Some(studium.join("Mathematik/Analysis II")));
        assert_eq!(sort("skript.pdf", "Lineare Algebra, Kapitel 2"), Some(studium.join("Mathematik/Lineare Algebra")));
        assert_eq!(sort("Analysis_I_Blatt1.pdf", "x"), Some(studium.join("Informatik/Einführung in die Analysis I")));
        // Content of a real-world PDF (subset fonts, metadata title).
        let p = downloads.join("vl4.pdf");
        fs::copy(fixture("analysis_chromium.pdf"), &p).unwrap();
        assert_eq!(
            sort_file(&p, &cfg).destination.map(|d| d.parent().unwrap().to_path_buf()),
            Some(studium.join("Informatik/Einführung in die Analysis I"))
        );
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
