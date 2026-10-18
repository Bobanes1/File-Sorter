//! Modules, keywords and file types shared between computers through a small file in the
//! study folder in the cloud.
//!
//! - [`SHARED_FILE`] in the cloud study folder holds a [`Shared`] as JSON (pretty printed).
//! - Machine-specific settings (folders, language, startup, …) are never shared; a module's
//!   custom `folder` neither: it is a path on one computer, so it is written empty and each
//!   computer keeps its own when applying.
//! - Modules are cleaned like `Config::clean` (trimmed, nameless ones dropped) whenever they
//!   are written, read or compared, so what one computer writes reads back as equal and
//!   never causes an endless round of rewrites.
//! - [`decide`] picks the step: the newest change wins.
//! - A file written by a newer File Sorter is neither read nor overwritten, so an older
//!   computer can't throw away what the newer format holds.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io;
use std::path::Path;
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

use crate::cloud::{TEMP_SUFFIX, save_json};
use crate::config::{Config, Course, normalise_ext};

/// The file in the cloud study folder.
pub const SHARED_FILE: &str = ".file-sorter.json";
/// Version of the format written; files from newer versions are not touched.
pub const VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Shared {
    pub version: u32,
    /// Milliseconds since 1970.
    pub updated_at: i64,
    /// The computer that wrote it.
    pub updated_by: String,
    pub courses: Vec<Course>,
    pub extensions: Vec<String>,
    pub read_file_contents: bool,
}

impl Shared {
    pub fn from_config(config: &Config, updated_at: i64, updated_by: &str) -> Shared {
        Shared {
            version: VERSION,
            updated_at,
            updated_by: updated_by.to_string(),
            courses: shared_courses(&config.courses),
            extensions: shared_extensions(&config.extensions),
            read_file_contents: config.read_file_contents,
        }
    }

    /// Same modules (in any order), keywords, file types and content reading as `config`.
    pub fn matches(&self, config: &Config) -> bool {
        self.read_file_contents == config.read_file_contents
            && comparable_courses(&self.courses) == comparable_courses(&config.courses)
            && comparable_extensions(&self.extensions) == comparable_extensions(&config.extensions)
    }

    /// Take over modules, keywords and file types; a module keeps its custom folder on this
    /// computer. Returns whether anything changed.
    pub fn apply_to(&self, config: &mut Config) -> bool {
        if self.matches(config) {
            return false;
        }
        let mut folders: HashMap<(String, String), &str> = HashMap::new();
        for course in config.courses.iter().filter(|c| !c.folder.trim().is_empty()) {
            folders.entry(course_key(course)).or_insert(&course.folder);
        }
        let courses: Vec<Course> = shared_courses(&self.courses)
            .into_iter()
            .map(|mut course| {
                if let Some(folder) = folders.get(&course_key(&course)) {
                    course.folder = folder.to_string();
                }
                course
            })
            .collect();

        // Logged so a module that disappeared can be traced back to the computer that removed it.
        let kept: HashSet<(String, String)> = courses.iter().map(course_key).collect();
        let removed: Vec<String> = config
            .courses
            .iter()
            .filter(|c| !c.name.trim().is_empty() && !kept.contains(&course_key(c)))
            .map(Course::label)
            .collect();
        log::info!(
            "Took over the shared modules from {}: {} modules{}",
            self.updated_by,
            courses.len(),
            if removed.is_empty() { String::new() } else { format!(", without {}", removed.join(", ")) }
        );

        config.courses = courses;
        config.extensions = shared_extensions(&self.extensions);
        config.read_file_contents = self.read_file_contents;
        true
    }
}

/// A module as it is shared: trimmed like `Config::clean`, without its custom folder;
/// `None` for a module without a name.
fn shared_course(course: &Course) -> Option<Course> {
    let name = course.name.trim();
    (!name.is_empty()).then(|| Course {
        name: name.to_string(),
        keywords: course.keywords.iter().map(|k| k.trim()).filter(|k| !k.is_empty()).map(str::to_string).collect(),
        folder: String::new(),
        subject: course.subject.trim().to_string(),
    })
}

fn shared_courses(courses: &[Course]) -> Vec<Course> {
    courses.iter().filter_map(shared_course).collect()
}

fn shared_extensions(extensions: &[String]) -> Vec<String> {
    extensions.iter().map(|e| normalise_ext(e)).collect()
}

/// Identifies a module across computers (and renames that only change the case).
fn course_key(course: &Course) -> (String, String) {
    (course.subject.trim().to_lowercase(), course.name.trim().to_lowercase())
}

/// The shared modules in a fixed order, so the order in the list doesn't count as a change.
fn comparable_courses(courses: &[Course]) -> Vec<Course> {
    let mut courses = shared_courses(courses);
    // Exact names and keywords break ties, so equal sets always end up in the same order.
    courses.sort_by_cached_key(|c| (course_key(c), c.subject.clone(), c.name.clone(), c.keywords.clone()));
    courses
}

/// File types as a set: order and repeats don't change what is sorted.
fn comparable_extensions(extensions: &[String]) -> Vec<String> {
    let mut extensions = shared_extensions(extensions);
    extensions.sort();
    extensions.dedup();
    extensions
}

/// The shared settings in `file`; `None` if missing, unreadable or from a newer version.
pub fn read(file: &Path) -> Option<Shared> {
    let text = fs::read_to_string(file).ok()?;
    // A byte-order mark may come from editing the file by hand (Notepad, PowerShell).
    let mut shared: Shared = serde_json::from_str(text.trim_start_matches('\u{feff}')).ok()?;
    if shared.version > VERSION {
        return None;
    }
    shared.courses = shared_courses(&shared.courses);
    shared.extensions = shared_extensions(&shared.extensions);
    Some(shared)
}

/// Write `shared` to `file` (through a temporary file).
///
/// Refuses to replace a file written by a newer File Sorter: its format may hold more than
/// this version knows, and the computers using it would take over the poorer copy.
pub fn write(file: &Path, shared: &Shared) -> io::Result<()> {
    if let Some(version) = written_version(file)
        && version > u64::from(VERSION)
    {
        return Err(io::Error::other(format!(
            "{} is from a newer File Sorter (format {version}); it is left as it is",
            file.display()
        )));
    }
    if let Err(e) = save_json(shared, file) {
        // The cloud app would otherwise upload the leftover to every computer.
        let name = file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let _ = fs::remove_file(file.with_file_name(format!("{name}{TEMP_SUFFIX}")));
        return Err(e);
    }
    log::info!("Shared {} modules with other computers in {}", shared.courses.len(), file.display());
    Ok(())
}

/// The format version of an existing shared file, even one this version can't read.
fn written_version(file: &Path) -> Option<u64> {
    #[derive(Deserialize)]
    struct Header {
        version: u64,
    }
    let text = fs::read_to_string(file).ok()?;
    serde_json::from_str::<Header>(text.trim_start_matches('\u{feff}')).ok().map(|h| h.version)
}

/// When `file` last changed, to check for news cheaply.
pub fn modified(file: &Path) -> Option<SystemTime> {
    fs::metadata(file).ok()?.modified().ok()
}

/// What to do to keep this computer and the shared file in step.
#[derive(Debug, Clone, PartialEq)]
pub enum Step {
    Nothing,
    /// Another computer changed them more recently: take theirs.
    Apply(Shared),
    /// This computer has changes (or there is no shared file yet): write ours.
    Write,
}

/// - `seen`: `updated_at` of the shared file this computer last read or wrote.
/// - `local_changed_at`: when the modules here were last changed (ms), if not shared yet.
/// - No shared file → `Write` if there are modules, else `Nothing`.
/// - Shared file equal to ours → `Nothing`.
/// - Written elsewhere since `seen` → `Apply`, unless our unshared change is newer → `Write`.
/// - Otherwise (ours differ, theirs not newer) → `Write`.
///
/// A computer without modules never starts the file: before the cloud app has brought down
/// another computer's file, an empty one written here would be newer and wipe theirs.
pub fn decide(config: &Config, seen: i64, local_changed_at: Option<i64>, remote: Option<&Shared>) -> Step {
    let Some(remote) = remote else {
        let has_modules = config.courses.iter().any(|c| !c.name.trim().is_empty());
        return if has_modules { Step::Write } else { Step::Nothing };
    };
    let ours_newer = local_changed_at.is_some_and(|at| at > remote.updated_at);
    if remote.matches(config) {
        Step::Nothing
    } else if remote.updated_at > seen && !ours_newer {
        Step::Apply(remote.clone())
    } else {
        Step::Write
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn course(subject: &str, name: &str, keywords: &[&str], folder: &str) -> Course {
        Course { folder: folder.to_string(), ..Course::new(name, keywords).with_subject(subject) }
    }

    fn config(courses: Vec<Course>) -> Config {
        Config { courses, extensions: vec![".pdf".into(), ".pptx".into()], ..Config::default() }
    }

    fn sample() -> Config {
        config(vec![
            course("Mathematik", "Analysis II", &["ana2", "analysis"], "D:\\Ana"),
            course("Informatik", "Programmierung 1", &["prog1"], ""),
        ])
    }

    #[test]
    fn round_trip() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("Studium").join(SHARED_FILE);
        let shared = Shared::from_config(&sample(), 1_700_000_000_123, "DESKTOP-4FJ2K");
        write(&file, &shared).unwrap();
        assert_eq!(read(&file), Some(shared.clone()));
        assert!(read(&file).unwrap().matches(&sample()));
        assert!(modified(&file).is_some());
        assert_eq!(fs::read_dir(file.parent().unwrap()).unwrap().count(), 1, "no temporary file is left");

        // Replacing an existing file (on Windows a rename onto it).
        let newer = Shared { updated_at: shared.updated_at + 1, ..shared };
        write(&file, &newer).unwrap();
        assert_eq!(read(&file), Some(newer));
    }

    #[test]
    fn only_shared_settings_are_written() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join(SHARED_FILE);
        let local = Config {
            watch_folder: "C:\\Users\\Anna\\Downloads".into(),
            destination_root: "C:\\Users\\Anna\\Studium".into(),
            language: Some("de".into()),
            ..sample()
        };
        write(&file, &Shared::from_config(&local, 5, "LAPTOP")).unwrap();
        let text = fs::read_to_string(&file).unwrap();
        assert!(text.contains("\"version\": 1") && text.contains("\"updated_by\": \"LAPTOP\""), "{text}");
        for private in ["Downloads", "Anna", "D:\\\\Ana", "language", "watch_folder", "cloud"] {
            assert!(!text.contains(private), "{private} must stay on this computer: {text}");
        }
    }

    #[test]
    fn from_config_strips_folders_and_cleans() {
        let mut local = sample();
        local.courses.push(course(" Physik ", " Mechanik ", &[" mech ", " "], "/home/a/mech"));
        local.courses.push(course("Physik", "  ", &["half typed"], ""));
        local.extensions.push("PDF".into());
        let shared = Shared::from_config(&local, 42, "PC");
        assert_eq!((shared.version, shared.updated_at, shared.updated_by.as_str()), (VERSION, 42, "PC"));
        assert!(shared.courses.iter().all(|c| c.folder.is_empty()));
        assert_eq!(shared.courses.len(), 3, "the nameless module is not shared");
        assert_eq!(shared.courses[2], course("Physik", "Mechanik", &["mech"], ""));
        assert_eq!(shared.extensions, vec![".pdf", ".pptx", ".pdf"]);
        assert!(shared.read_file_contents);
        assert!(shared.matches(&local));
    }

    #[test]
    fn reading_tolerates_hand_edits_and_ignores_the_unreadable() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join(SHARED_FILE);
        let check = |text: &str| {
            fs::write(&file, text).unwrap();
            read(&file)
        };
        let body = r#""updated_at": 7, "updated_by": "PC", "extensions": ["PDF"], "read_file_contents": false,
            "courses": [{"name": " Statistik ", "subject": " Mathe ", "keywords": [" stats ", ""], "folder": "/x"},
                        {"name": ""}, {"name": "Algo", "keywords": null}]"#;

        let shared = check(&format!("\u{feff}{{\"version\": 1, {body}}}")).expect("byte-order mark");
        assert_eq!(shared.courses, vec![course("Mathe", "Statistik", &["stats"], ""), course("", "Algo", &[], "")]);
        assert_eq!(shared.extensions, vec![".pdf"]);
        assert!(!shared.read_file_contents);
        assert!(check(&format!("{{\"version\": 0, {body}}}")).is_some(), "older formats are read");

        assert_eq!(check(&format!("{{\"version\": 2, {body}}}")), None, "newer format");
        assert_eq!(check(&format!("{{\"version\": 2, \"modules\": {{}}, {body}}}")), None, "newer format");
        assert_eq!(check(&format!("{{\"version\": 1, {body}")), None, "cut off");
        assert_eq!(check("{\"version\": 1, \"updated_at\": 7}"), None, "no modules: not taken as \"none\"");
        assert_eq!(check("not json"), None);
        assert_eq!(check(""), None);
        fs::remove_file(&file).unwrap();
        assert_eq!(read(&file), None, "missing");
        assert_eq!(modified(&file), None);
        fs::create_dir(&file).unwrap();
        assert_eq!(read(&file), None, "a folder of that name");
    }

    #[test]
    fn newer_format_is_not_overwritten() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join(SHARED_FILE);
        let ours = Shared::from_config(&sample(), 9, "OLD-PC");
        for (existing, overwritten) in [
            ("{\"version\": 2, \"everything\": \"different\"}", false),
            ("\u{feff}{\"version\": 3}", false),
            ("{\"version\": 1, \"updated_at\": \"broken\"}", true),
            ("{\"version\": 0}", true),
            ("garbage", true),
        ] {
            fs::write(&file, existing).unwrap();
            assert_eq!(write(&file, &ours).is_ok(), overwritten, "{existing}");
            let now = fs::read_to_string(&file).unwrap();
            assert_eq!(now == existing, !overwritten, "{existing}");
            assert_eq!(fs::read_dir(tmp.path()).unwrap().count(), 1, "no temporary file is left");
        }
    }

    #[test]
    fn failed_write_leaves_no_temporary_file() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join(SHARED_FILE);
        fs::create_dir_all(file.join("in the way")).unwrap();
        assert!(write(&file, &Shared::from_config(&sample(), 1, "PC")).is_err());
        let names: Vec<_> = fs::read_dir(tmp.path()).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(names, vec![std::ffi::OsString::from(SHARED_FILE)]);
        assert!(file.join("in the way").is_dir());
    }

    #[test]
    fn matches_ignores_order_folders_and_spacing() {
        let shared = Shared::from_config(&sample(), 1, "PC");
        let mut other = sample();
        other.courses.reverse();
        other.courses[0].folder = "/elsewhere".into();
        other.courses[1].folder.clear();
        other.courses[0].name = format!(" {} ", other.courses[0].name);
        other.courses.push(course("Mathematik", "", &["typing"], ""));
        other.extensions = vec![".pptx".into(), "PDF".into(), ".pdf".into()];
        assert!(shared.matches(&other));

        let differs = |change: fn(&mut Config)| {
            let mut changed = sample();
            change(&mut changed);
            !shared.matches(&changed)
        };
        assert!(differs(|c| c.courses[0].keywords.reverse()), "keyword order is kept");
        assert!(differs(|c| c.courses[0].keywords.push("neu".into())));
        assert!(differs(|c| c.courses[0].name = "analysis ii".into()), "a renamed module");
        assert!(differs(|c| c.courses[0].subject = "Mathe".into()));
        assert!(differs(|c| {
            c.courses.pop();
        }));
        assert!(differs(|c| c.courses.push(course("", "Statistik", &[], ""))));
        assert!(differs(|c| c.extensions.push(".zip".into())));
        assert!(differs(|c| c.read_file_contents = false));
    }

    #[test]
    fn apply_keeps_local_folders() {
        let mut remote_config = config(vec![
            course("mathematik", "ANALYSIS II", &["ana2"], "/their/folder"),
            course("Physik", "Mechanik", &["mech"], ""),
            course("Physik", " ", &[], ""),
        ]);
        remote_config.extensions = vec![".pdf".into(), ".zip".into()];
        remote_config.read_file_contents = false;
        // Not cleaned, as if made by hand: applying must still drop their folders and nameless modules.
        let remote = Shared { courses: remote_config.courses.clone(), ..Shared::from_config(&remote_config, 3, "PC") };

        let mut local = sample();
        local.courses.push(course("Physik", "Mechanik", &[], ""));
        assert!(remote.apply_to(&mut local));
        assert_eq!(
            local.courses,
            vec![
                course("mathematik", "ANALYSIS II", &["ana2"], "D:\\Ana"),
                course("Physik", "Mechanik", &["mech"], "")
            ],
            "the custom folder here stays, theirs and the nameless module don't come along"
        );
        assert_eq!(local.extensions, vec![".pdf", ".zip"]);
        assert!(!local.read_file_contents);
        assert!(remote.matches(&local));
        assert!(!remote.apply_to(&mut local), "nothing left to change");

        // Already the same apart from order: nothing is touched.
        let mut reordered = sample();
        reordered.courses.reverse();
        let before = reordered.clone();
        assert!(!Shared::from_config(&sample(), 4, "PC").apply_to(&mut reordered));
        assert_eq!(reordered, before);
    }

    #[test]
    fn decide_table() {
        let ours = sample();
        let none = Config::default();
        let half_typed = config(vec![course("Mathe", " ", &[], "")]);
        let mut theirs_config = sample();
        theirs_config.courses[1].keywords.push("java".into());
        let theirs = |at: i64| Shared::from_config(&theirs_config, at, "OTHER");
        let equal = |at: i64| Shared::from_config(&ours, at, "OTHER");
        let apply = |at: i64| Step::Apply(theirs(at));

        #[rustfmt::skip]
        let cases = vec![
            // what                            config       seen changed    remote             expected
            ("no file, modules here",           &ours,       0,   None,      None,              Step::Write),
            ("no file, no modules",             &none,       0,   None,      None,              Step::Nothing),
            ("no file, no modules, a change",   &none,       0,   Some(5),   None,              Step::Nothing),
            ("no file, only a nameless module", &half_typed, 0,   None,      None,              Step::Nothing),
            ("equal",                           &ours,       100, None,      Some(equal(100)),  Step::Nothing),
            ("equal, written elsewhere",        &ours,       100, Some(300), Some(equal(200)),  Step::Nothing),
            ("equal, older",                    &ours,       100, None,      Some(equal(50)),   Step::Nothing),
            ("theirs newer, no change here",    &ours,       100, None,      Some(theirs(200)), apply(200)),
            ("theirs newer, never seen",        &ours,       0,   None,      Some(theirs(200)), apply(200)),
            ("theirs newer, no modules here",   &none,       0,   None,      Some(theirs(200)), apply(200)),
            ("theirs newer than our change",    &ours,       100, Some(150), Some(theirs(200)), apply(200)),
            ("our change at the same time",     &ours,       100, Some(200), Some(theirs(200)), apply(200)),
            ("our change newer than theirs",    &ours,       100, Some(250), Some(theirs(200)), Step::Write),
            ("theirs older and different",      &ours,       100, None,      Some(theirs(50)),  Step::Write),
            ("theirs older, our change",        &ours,       100, Some(120), Some(theirs(50)),  Step::Write),
            ("seen and different",              &ours,       100, None,      Some(theirs(100)), Step::Write),
            ("seen, our change",                &ours,       100, Some(150), Some(theirs(100)), Step::Write),
        ];
        for (what, config, seen, changed, remote, expected) in cases {
            assert_eq!(decide(config, seen, changed, remote.as_ref()), expected, "{what}");
        }
    }

    /// Writing, reading back and applying always settle: no endless rewrites between computers.
    #[test]
    fn sync_settles() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join(SHARED_FILE);
        let mut laptop = sample();
        laptop.courses.push(course(" Physik ", "Mechanik ", &[" mech", ""], ""));
        laptop.courses.push(course("Physik", "", &["half typed"], ""));
        laptop.extensions.push("PDF".into());
        assert_eq!(decide(&laptop, 0, Some(10), read(&file).as_ref()), Step::Write);
        write(&file, &Shared::from_config(&laptop, 20, "LAPTOP")).unwrap();
        assert_eq!(decide(&laptop, 20, None, read(&file).as_ref()), Step::Nothing);

        let mut desktop = Config::default();
        let Step::Apply(shared) = decide(&desktop, 0, None, read(&file).as_ref()) else { panic!("should apply") };
        assert!(shared.apply_to(&mut desktop));
        assert_eq!(decide(&desktop, shared.updated_at, None, read(&file).as_ref()), Step::Nothing);
        let back = Shared::from_config(&desktop, 30, "DESKTOP");
        assert_eq!(decide(&laptop, 20, None, Some(&back)), Step::Nothing);
    }
}
