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
//! - [`decide`] picks the step. It goes by which file a computer has seen (`updated_at` as an
//!   id, `based_on` for what the writer had seen), never by comparing clocks of two computers:
//!   a clock running ahead must not make another computer's change look old. Only when both
//!   changed the same module does the newer change of that module win.
//! - A file written by a newer File Sorter is neither read nor overwritten, so an older
//!   computer can't throw away what the newer format holds.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::fs;
use std::io;
use std::path::Path;
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

use crate::cloud::{TEMP_SUFFIX, save_json};
use crate::config::{Config, Course, normalise_ext};
use crate::sorter::{release_placeholder, reserve_destination};

/// The file in the cloud study folder.
pub const SHARED_FILE: &str = ".file-sorter.json";
/// Where a damaged shared file is kept before it is replaced (it may be a hand edit with a typo).
pub const DAMAGED_FILE: &str = ".file-sorter.damaged.json";
/// Version of the format written; files from newer versions are not touched.
pub const VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Shared {
    pub version: u32,
    /// Milliseconds since 1970 on the writer's clock; also tells one written file from another.
    pub updated_at: i64,
    /// The computer that wrote it.
    pub updated_by: String,
    /// `updated_at` of the file the writer had last read or written (0: none), so a reader can
    /// tell a change built on what it knows from one made without having seen its own.
    #[serde(default)]
    pub based_on: i64,
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
            based_on: 0,
            courses: shared_courses(&config.courses),
            extensions: shared_extensions(&config.extensions),
            read_file_contents: config.read_file_contents,
        }
    }

    /// Same modules (in any order), keywords, file types and content reading as `config`.
    pub fn matches(&self, config: &Config) -> bool {
        self.same_as(&config.courses, &config.extensions, config.read_file_contents)
    }

    fn same_as(&self, courses: &[Course], extensions: &[String], read_file_contents: bool) -> bool {
        self.read_file_contents == read_file_contents
            && comparable_courses(&self.courses) == comparable_courses(courses)
            && comparable_extensions(&self.extensions) == comparable_extensions(extensions)
    }

    /// Take over modules, keywords and file types; a module keeps its custom folder on this
    /// computer (also when it was renamed elsewhere, if that can be told). Returns whether
    /// anything changed.
    pub fn apply_to(&self, config: &mut Config) -> bool {
        if self.matches(config) {
            return false;
        }
        let mut folders: HashMap<(String, String), &str> = HashMap::new();
        for course in config.courses.iter().filter(|c| !c.folder.trim().is_empty()) {
            folders.entry(course_key(course)).or_insert(&course.folder);
        }
        let mut courses: Vec<Course> = shared_courses(&self.courses)
            .into_iter()
            .map(|mut course| {
                if let Some(folder) = folders.get(&course_key(&course)) {
                    course.folder = folder.to_string();
                }
                course
            })
            .collect();

        // A module renamed (or moved to another subject) elsewhere would otherwise lose its
        // folder here, and its new files would land apart from the old ones.
        let local: HashSet<(String, String)> = config.courses.iter().map(course_key).collect();
        let kept: HashSet<(String, String)> = courses.iter().map(course_key).collect();
        for old in config.courses.iter().filter(|c| !c.folder.trim().is_empty() && !kept.contains(&course_key(c))) {
            let renamed: Vec<usize> = (0..courses.len())
                .filter(|&i| courses[i].folder.is_empty() && !local.contains(&course_key(&courses[i])))
                .filter(|&i| renamed_from(old, &courses[i]))
                .collect();
            if let [i] = renamed[..] {
                log::info!("{} is now {}: it keeps the folder {}", old.label(), courses[i].label(), old.folder);
                courses[i].folder = old.folder.clone();
            } else {
                log::info!(
                    "The folder {} is no longer used: {} is gone from the shared modules",
                    old.folder,
                    old.label()
                );
            }
        }

        // Logged so a module that disappeared can be traced back to the computer that removed it.
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

/// `new` looks like `old` renamed: the same name in another subject, or the same keywords.
fn renamed_from(old: &Course, new: &Course) -> bool {
    let keywords = |c: &Course| -> BTreeSet<String> {
        c.keywords.iter().map(|k| k.trim().to_lowercase()).filter(|k| !k.is_empty()).collect()
    };
    old.name.trim().to_lowercase() == new.name.trim().to_lowercase()
        || (!keywords(old).is_empty() && keywords(old) == keywords(new))
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
        ..course.clone()
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

/// What the shared file holds right now.
#[derive(Debug, Clone, PartialEq)]
pub enum Remote {
    Missing,
    Ok(Shared),
    /// Written by a newer File Sorter: neither read nor overwritten.
    NewerFormat,
    /// Not a valid shared file: cut off, damaged, or a typo from editing it by hand.
    Damaged,
    /// Couldn't be read (a cloud-only file while offline, locked, a folder of that name). It
    /// may hold another computer's change, so it is neither taken nor overwritten.
    Unreadable,
}

/// Read the shared file; reading a cloud-only file makes the cloud app download it.
pub fn load(file: &Path) -> Remote {
    match fs::read(file) {
        Ok(bytes) => parse(&bytes),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Remote::Missing,
        Err(_) => Remote::Unreadable,
    }
}

fn parse(bytes: &[u8]) -> Remote {
    #[derive(Deserialize)]
    struct Header {
        version: u64,
    }
    let Ok(text) = std::str::from_utf8(bytes) else { return Remote::Damaged };
    // A byte-order mark may come from editing the file by hand (Notepad, PowerShell).
    let text = text.trim_start_matches('\u{feff}');
    if serde_json::from_str::<Header>(text).is_ok_and(|h| h.version > u64::from(VERSION)) {
        return Remote::NewerFormat;
    }
    match serde_json::from_str::<Shared>(text) {
        Ok(mut shared) => {
            shared.courses = shared_courses(&shared.courses);
            shared.extensions = shared_extensions(&shared.extensions);
            Remote::Ok(shared)
        }
        Err(_) => Remote::Damaged,
    }
}

/// The shared settings in `file`; `None` if missing, unreadable, damaged or from a newer version.
#[cfg(test)]
fn read(file: &Path) -> Option<Shared> {
    match load(file) {
        Remote::Ok(shared) => Some(shared),
        _ => None,
    }
}

/// Write `shared` to `file` (through a temporary file).
///
/// Refuses to replace a file written by a newer File Sorter (its format may hold more than
/// this version knows, and the computers using it would take over the poorer copy) or one
/// that can't be read (it may hold a change not seen yet). A damaged one is kept as
/// [`DAMAGED_FILE`] first, unless it is empty.
pub fn write(file: &Path, shared: &Shared) -> io::Result<()> {
    match fs::read(file) {
        Ok(bytes) => match parse(&bytes) {
            Remote::NewerFormat => {
                return Err(io::Error::other(format!(
                    "{} is from a newer File Sorter; it is left as it is",
                    file.display()
                )));
            }
            Remote::Damaged if !bytes.iter().all(|b| b.is_ascii_whitespace() || *b == 0) => keep_damaged(file, &bytes)?,
            _ => {}
        },
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => {
            return Err(io::Error::new(
                e.kind(),
                format!("{} can't be read ({e}); it is left as it is", file.display()),
            ));
        }
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

/// Copy a damaged shared file next to it under a name of its own (never replacing an earlier copy).
fn keep_damaged(file: &Path, bytes: &[u8]) -> io::Result<()> {
    let copy = reserve_destination(file.parent().unwrap_or(Path::new(".")), DAMAGED_FILE)?;
    if let Err(e) = fs::write(&copy, bytes) {
        release_placeholder(&copy);
        return Err(e);
    }
    log::info!("{} could not be read; it is kept as {} and replaced", file.display(), copy.display());
    Ok(())
}

/// When `file` last changed, to check for news cheaply.
pub fn modified(file: &Path) -> Option<SystemTime> {
    fs::metadata(file).ok()?.modified().ok()
}

/// What to do to keep this computer and the shared file in step.
#[derive(Debug, Clone, PartialEq)]
pub enum Step {
    Nothing,
    /// Another computer changed them and this one didn't: take theirs.
    Apply(Shared),
    /// Both changed them: take this combination of the two, then write it.
    Merge(Shared),
    /// This computer has changes (or there is no shared file yet): write ours.
    Write,
}

/// - `seen`: `updated_at` of the shared file this computer last read or wrote (0: never).
/// - `base`: what that file held, if known.
/// - `local_changed_at`: when the modules here were last changed (ms), if not shared yet.
/// - No shared file → `Write` if there are modules, else `Nothing`.
/// - Shared file equal to ours → `Nothing`; still the one we've seen → `Write` (ours changed).
/// - Written elsewhere: `Apply` if nothing changed here (or never shared here: the window
///   asks which to keep), else `Merge`.
///
/// A file written elsewhere without having seen ours (both wrote at about the same time, the
/// cloud app kept the other one of two copies) is merged without a base, so nothing either
/// side has is lost; at worst a module removed on one of them comes back.
///
/// A computer without modules never starts the file: before the cloud app has brought down
/// another computer's file, an empty one written here would replace theirs.
pub fn decide(
    config: &Config,
    seen: i64,
    base: Option<&Shared>,
    local_changed_at: Option<i64>,
    remote: Option<&Shared>,
) -> Step {
    let Some(remote) = remote else {
        let has_modules = config.courses.iter().any(|c| !c.name.trim().is_empty());
        return if has_modules { Step::Write } else { Step::Nothing };
    };
    if remote.matches(config) {
        return Step::Nothing;
    }
    if remote.updated_at == seen {
        return Step::Write;
    }
    let built_on_ours = remote.based_on == seen;
    if seen == 0 || (built_on_ours && local_changed_at.is_none()) {
        return Step::Apply(remote.clone());
    }
    let base = base.filter(|b| built_on_ours && b.updated_at == seen);
    // Their clock may differ from ours: this only settles a module both changed.
    let ours_newer = local_changed_at.is_some_and(|at| at > remote.updated_at);
    let merged = merge(base, config, remote, ours_newer);
    if merged.same_as(&remote.courses, &remote.extensions, remote.read_file_contents) {
        Step::Apply(remote.clone())
    } else {
        Step::Merge(merged)
    }
}

/// Our changes and theirs since `base`, module by module (`None`: unknown, so nothing is
/// dropped). A module removed on one side stays if the other changed it. When both changed
/// one, the newer change gives its name, and keywords added on either side are kept.
fn merge(base: Option<&Shared>, ours: &Config, theirs: &Shared, ours_newer: bool) -> Shared {
    let empty = Vec::new();
    let base_courses = base.map(|b| shared_courses(&b.courses)).unwrap_or_default();
    let ours_courses = shared_courses(&ours.courses);
    let theirs_courses = shared_courses(&theirs.courses);
    let (b, o, t) = (keyed(&base_courses), keyed(&ours_courses), keyed(&theirs_courses));
    let base_map: HashMap<_, _> = b.into_iter().collect();
    let theirs_map: HashMap<_, _> = t.iter().cloned().collect();
    let ours_map: HashMap<_, _> = o.iter().cloned().collect();

    let mut courses = Vec::new();
    for (key, ours) in &o {
        let base = base_map.get(key).copied();
        match theirs_map.get(key) {
            Some(theirs) => courses.push(merge_course(base, ours, theirs, ours_newer)),
            // Removed there: gone here too, unless changed here since.
            None if base == Some(*ours) => {}
            None => courses.push((*ours).clone()),
        }
    }
    for (key, theirs) in &t {
        if !ours_map.contains_key(key) && base_map.get(key).copied() != Some(*theirs) {
            courses.push((*theirs).clone());
        }
    }

    let base_ext = base.map_or(&empty, |b| &b.extensions);
    let ours_ext = shared_extensions(&ours.extensions);
    let set = |e: &[String]| comparable_extensions(e);
    let extensions = if base.is_some() && set(&ours_ext) == set(base_ext) {
        theirs.extensions.clone()
    } else if base.is_some() && set(&theirs.extensions) == set(base_ext) {
        ours_ext
    } else {
        merge_words(base_ext, &ours_ext, &theirs.extensions)
    };
    let read_file_contents = match base {
        Some(base) if base.read_file_contents != ours.read_file_contents => ours.read_file_contents,
        Some(_) => theirs.read_file_contents,
        None if ours_newer => ours.read_file_contents,
        None => theirs.read_file_contents,
    };
    Shared { courses, extensions, read_file_contents, ..theirs.clone() }
}

/// Each module under its key; a second module with the same key gets the next number.
fn keyed(courses: &[Course]) -> Vec<((String, String, usize), &Course)> {
    let mut count: HashMap<(String, String), usize> = HashMap::new();
    courses
        .iter()
        .map(|c| {
            let (subject, name) = course_key(c);
            let n = count.entry((subject.clone(), name.clone())).or_default();
            *n += 1;
            ((subject, name, *n), c)
        })
        .collect()
}

fn merge_course(base: Option<&Course>, ours: &Course, theirs: &Course, ours_newer: bool) -> Course {
    if ours == theirs || base == Some(theirs) {
        return ours.clone();
    }
    if base == Some(ours) {
        return theirs.clone();
    }
    let (newer, older) = if ours_newer { (ours, theirs) } else { (theirs, ours) };
    let base_keywords = base.map_or(&[][..], |b| &b.keywords[..]);
    Course { keywords: merge_words(base_keywords, &newer.keywords, &older.keywords), ..newer.clone() }
}

/// Words in `first` or `second`, without those one side removed since `base` (ignoring case).
fn merge_words(base: &[String], first: &[String], second: &[String]) -> Vec<String> {
    let set = |words: &[String]| -> HashSet<String> { words.iter().map(|w| w.to_lowercase()).collect() };
    let (base, in_first, in_second) = (set(base), set(first), set(second));
    let mut taken = HashSet::new();
    first
        .iter()
        .filter(|w| in_second.contains(&w.to_lowercase()) || !base.contains(&w.to_lowercase()))
        .chain(second.iter().filter(|w| !in_first.contains(&w.to_lowercase()) && !base.contains(&w.to_lowercase())))
        .filter(|w| taken.insert(w.to_lowercase()))
        .cloned()
        .collect()
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
        let ours = Shared::from_config(&sample(), 9, "OLD-PC");
        for (existing, overwritten) in [
            ("{\"version\": 2, \"everything\": \"different\"}", false),
            ("\u{feff}{\"version\": 3}", false),
            ("{\"version\": 1, \"updated_at\": \"broken\"}", true),
            ("{\"version\": 0}", true),
            ("garbage", true),
        ] {
            let tmp = tempfile::tempdir().unwrap();
            let file = tmp.path().join(SHARED_FILE);
            fs::write(&file, existing).unwrap();
            assert_eq!(write(&file, &ours).is_ok(), overwritten, "{existing}");
            let now = fs::read_to_string(&file).unwrap();
            assert_eq!(now == existing, !overwritten, "{existing}");
            let damaged = fs::read_to_string(tmp.path().join(DAMAGED_FILE)).ok();
            assert_eq!(damaged.as_deref(), overwritten.then_some(existing), "a damaged file is kept: {existing}");
            assert_eq!(fs::read_dir(tmp.path()).unwrap().count(), 1 + usize::from(overwritten), "no temporary file");
        }
    }

    /// A damaged shared file (a hand edit with a typo, cut off by a power cut) is kept aside
    /// before it is replaced, without replacing a copy kept earlier; an empty one is not kept.
    #[test]
    fn damaged_file_is_kept_before_it_is_replaced() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join(SHARED_FILE);
        let ours = Shared::from_config(&sample(), 9, "PC");
        fs::write(&file, "{\"version\": 1, \"courses\": [ typo").unwrap();
        write(&file, &ours).unwrap();
        fs::write(&file, "second typo").unwrap();
        write(&file, &ours).unwrap();
        fs::write(&file, [0u8; 32]).unwrap();
        write(&file, &ours).unwrap();
        assert_eq!(read(&file), Some(ours));
        assert_eq!(fs::read_to_string(tmp.path().join(DAMAGED_FILE)).unwrap(), "{\"version\": 1, \"courses\": [ typo");
        assert_eq!(fs::read_to_string(tmp.path().join(".file-sorter.damaged (1).json")).unwrap(), "second typo");
        assert_eq!(fs::read_dir(tmp.path()).unwrap().count(), 3);
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
        let theirs = |at: i64, based_on: i64| Shared { based_on, ..Shared::from_config(&theirs_config, at, "OTHER") };
        let equal = |at: i64| Shared::from_config(&ours, at, "OTHER");
        let apply = |at: i64, based_on: i64| Step::Apply(theirs(at, based_on));
        // Each computer stamps with its own clock: "older" stamps from elsewhere are still news.
        #[rustfmt::skip]
        let cases = vec![
            // what                              config       seen changed    remote                  expected
            ("no file, modules here",             &ours,       0,   None,      None,                   Step::Write),
            ("no file, no modules",               &none,       0,   None,      None,                   Step::Nothing),
            ("no file, no modules, a change",     &none,       0,   Some(5),   None,                   Step::Nothing),
            ("no file, only a nameless module",   &half_typed, 0,   None,      None,                   Step::Nothing),
            ("equal",                             &ours,       100, None,      Some(equal(100)),       Step::Nothing),
            ("equal, written elsewhere",          &ours,       100, Some(300), Some(equal(200)),       Step::Nothing),
            ("equal, older",                      &ours,       100, None,      Some(equal(50)),        Step::Nothing),
            ("theirs newer, no change here",      &ours,       100, None,      Some(theirs(200, 100)), apply(200, 100)),
            ("theirs never seen here",            &ours,       0,   None,      Some(theirs(200, 0)),   apply(200, 0)),
            ("never seen, no modules here",       &none,       0,   None,      Some(theirs(200, 7)),   apply(200, 7)),
            ("their clock behind ours",           &ours,       100, None,      Some(theirs(50, 100)),  apply(50, 100)),
            ("their clock behind, same stamp",    &ours,       100, None,      Some(theirs(100, 100)), Step::Write),
            ("still the file seen: ours changed", &ours,       100, Some(150), Some(theirs(100, 0)),   Step::Write),
        ];
        for (what, config, seen, changed, remote, expected) in cases {
            assert_eq!(decide(config, seen, None, changed, remote.as_ref()), expected, "{what}");
        }
    }

    /// Changes made on both computers before they meet again are combined, never replaced.
    #[test]
    fn both_changed_are_merged() {
        let base = Shared::from_config(&sample(), 100, "HERE");
        // Here: a keyword for Analysis, and Statistik added.
        let mut ours = sample();
        ours.courses[0].keywords.push("hörsaal".into());
        ours.courses.push(course("Mathematik", "Statistik", &["stats"], "D:\\Stats"));
        // There: Programmierung removed, Physik added, a file type.
        let mut theirs_config = config(vec![
            course("Mathematik", "Analysis II", &["ana2", "analysis"], ""),
            course("Physik", "Mechanik", &["mech"], ""),
        ]);
        theirs_config.extensions.push(".zip".into());
        let theirs = Shared { based_on: 100, ..Shared::from_config(&theirs_config, 200, "THERE") };

        // Their clock may be far behind (or ahead): the outcome is the same.
        for changed in [Some(150), Some(250), Some(50)] {
            let Step::Merge(merged) = decide(&ours, 100, Some(&base), changed, Some(&theirs)) else {
                panic!("should merge");
            };
            assert_eq!((merged.updated_at, merged.updated_by.as_str()), (200, "THERE"));
            let mut here = ours.clone();
            assert!(merged.apply_to(&mut here));
            assert_eq!(
                here.courses,
                vec![
                    course("Mathematik", "Analysis II", &["ana2", "analysis", "hörsaal"], "D:\\Ana"),
                    course("Mathematik", "Statistik", &["stats"], "D:\\Stats"),
                    course("Physik", "Mechanik", &["mech"], ""),
                ],
                "{changed:?}"
            );
            assert_eq!(here.extensions, vec![".pdf", ".pptx", ".zip"]);
        }

        // Removed there but changed here: kept. Changed there but removed here: kept.
        let mut ours = sample();
        ours.courses[1].keywords.push("java".into());
        ours.courses.remove(0);
        let mut theirs_config = sample();
        theirs_config.courses.remove(1);
        theirs_config.courses[0].keywords.push("neu".into());
        let theirs = Shared { based_on: 100, ..Shared::from_config(&theirs_config, 200, "THERE") };
        let Step::Merge(merged) = decide(&ours, 100, Some(&base), Some(150), Some(&theirs)) else { panic!() };
        assert_eq!(merged.courses.len(), 2, "{merged:?}");

        // Both removed different modules: both gone.
        let mut ours = sample();
        ours.courses.remove(0);
        let mut theirs_config = sample();
        theirs_config.courses.remove(1);
        let theirs = Shared { based_on: 100, ..Shared::from_config(&theirs_config, 200, "THERE") };
        let Step::Merge(merged) = decide(&ours, 100, Some(&base), Some(150), Some(&theirs)) else { panic!() };
        assert!(merged.courses.is_empty());
    }

    /// The same module's keywords changed on both: the newer change wins where they disagree,
    /// keywords added on either side are kept, and those removed on one side stay removed.
    #[test]
    fn both_changed_one_module() {
        let base = Shared::from_config(&sample(), 100, "HERE");
        let mut ours = sample();
        ours.courses[0].keywords = vec!["ana2".into(), "mine".into()]; // removed "analysis"
        let mut theirs_config = sample();
        theirs_config.courses[0].keywords.push("theirs".into());
        theirs_config.courses[0].name = "analysis ii".into();
        let theirs = Shared { based_on: 100, ..Shared::from_config(&theirs_config, 200, "THERE") };

        let Step::Merge(merged) = decide(&ours, 100, Some(&base), Some(300), Some(&theirs)) else { panic!() };
        assert_eq!(merged.courses[0], course("Mathematik", "Analysis II", &["ana2", "mine", "theirs"], ""));
        let Step::Merge(merged) = decide(&ours, 100, Some(&base), Some(150), Some(&theirs)) else { panic!() };
        assert_eq!(merged.courses[0], course("Mathematik", "analysis ii", &["ana2", "theirs", "mine"], ""));
    }

    /// Two computers that each wrote the file before seeing the other's (the cloud app kept
    /// one of the two): both sets of modules survive, on both computers.
    #[test]
    fn first_meeting_race_keeps_both() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join(SHARED_FILE);
        let desktop = config(vec![course("Mathematik", "Analysis II", &["ana2"], "")]);
        let mut laptop = config(vec![course("Informatik", "Programmierung 1", &["prog1"], "")]);

        // The laptop's clock is ahead: its file looks newer than the desktop's.
        let laptop_file = Shared::from_config(&laptop, 9_000, "LAPTOP");
        assert_eq!(decide(&laptop, 0, None, None, None), Step::Write);
        let laptop_seen = laptop_file.updated_at;
        // The cloud app replaces it with the desktop's file, written without seeing the laptop's.
        write(&file, &Shared::from_config(&desktop, 1_000, "DESKTOP")).unwrap();
        let Step::Merge(merged) = decide(&laptop, laptop_seen, Some(&laptop_file), None, read(&file).as_ref()) else {
            panic!("should merge")
        };
        assert!(merged.apply_to(&mut laptop));
        assert_eq!(laptop.courses.len(), 2);
        let written = Shared { based_on: merged.updated_at, ..Shared::from_config(&laptop, 9_100, "LAPTOP") };
        write(&file, &written).unwrap();

        // The desktop takes the combination over: nothing to ask, nothing lost.
        let mut desktop_now = desktop.clone();
        let desktop_base = Shared::from_config(&desktop, 1_000, "DESKTOP");
        let Step::Apply(shared) = decide(&desktop_now, 1_000, Some(&desktop_base), None, read(&file).as_ref()) else {
            panic!("should apply")
        };
        assert!(shared.apply_to(&mut desktop_now));
        assert!(shared.matches(&laptop) && shared.matches(&desktop_now));
    }

    #[test]
    fn renamed_module_keeps_its_folder() {
        let mut local = sample();
        local.courses.push(course("Physik", "Mechanik", &["mech"], "/home/a/mech"));
        local.courses.push(course("Physik", "Optik", &["optik"], "/home/a/optik"));
        let mut remote_config = sample();
        remote_config.courses[0].name = "Analysis 2".into(); // renamed, same keywords
        remote_config.courses.push(course("Mechanik", "Mechanik", &["mech", "kraft"], "")); // other subject
        let remote = Shared::from_config(&remote_config, 5, "OTHER"); // Optik removed
        assert!(remote.apply_to(&mut local));
        assert_eq!(local.courses[0], course("Mathematik", "Analysis 2", &["ana2", "analysis"], "D:\\Ana"));
        assert_eq!(local.courses[2], course("Mechanik", "Mechanik", &["mech", "kraft"], "/home/a/mech"));
        assert_eq!(local.courses.len(), 3);

        // Two candidates: which one is meant can't be told, so neither gets it.
        let mut local = config(vec![course("A", "Statistik", &[], "/stats")]);
        let remote = Shared::from_config(
            &config(vec![course("B", "Statistik", &[], ""), course("C", "Statistik", &[], "")]),
            6,
            "OTHER",
        );
        assert!(remote.apply_to(&mut local));
        assert!(local.courses.iter().all(|c| c.folder.is_empty()));
    }

    #[test]
    fn load_tells_why() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join(SHARED_FILE);
        assert_eq!(load(&file), Remote::Missing);
        fs::write(&file, "{\"version\": 1, \"updated_at\": 7").unwrap();
        assert_eq!(load(&file), Remote::Damaged);
        fs::write(&file, [0u8; 64]).unwrap();
        assert_eq!(load(&file), Remote::Damaged, "zero-filled after a power cut");
        fs::write(&file, "{\"version\": 5, \"anything\": 1}").unwrap();
        assert_eq!(load(&file), Remote::NewerFormat);
        let shared = Shared::from_config(&sample(), 1, "PC");
        fs::remove_file(&file).unwrap();
        write(&file, &shared).unwrap();
        assert_eq!(load(&file), Remote::Ok(shared));
        fs::remove_file(&file).unwrap();
        fs::create_dir(&file).unwrap();
        assert_eq!(load(&file), Remote::Unreadable);
    }

    /// A file that can't be read may hold a change not taken over yet: never replaced.
    #[cfg(unix)]
    #[test]
    fn unreadable_file_is_not_overwritten() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join(SHARED_FILE);
        let theirs = Shared::from_config(&sample(), 1, "OTHER");
        write(&file, &theirs).unwrap();
        fs::set_permissions(&file, fs::Permissions::from_mode(0o000)).unwrap();
        if fs::read(&file).is_ok() {
            return; // running as root: the file can't be made unreadable
        }
        assert_eq!(load(&file), Remote::Unreadable);
        assert!(write(&file, &Shared::from_config(&Config::default(), 2, "HERE")).is_err());
        fs::set_permissions(&file, fs::Permissions::from_mode(0o644)).unwrap();
        assert_eq!(read(&file), Some(theirs));
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
        assert_eq!(decide(&laptop, 0, None, Some(10), read(&file).as_ref()), Step::Write);
        write(&file, &Shared::from_config(&laptop, 20, "LAPTOP")).unwrap();
        let written = read(&file);
        assert_eq!(decide(&laptop, 20, written.as_ref(), None, written.as_ref()), Step::Nothing);

        let mut desktop = Config::default();
        let Step::Apply(shared) = decide(&desktop, 0, None, None, read(&file).as_ref()) else { panic!("should apply") };
        assert!(shared.apply_to(&mut desktop));
        assert_eq!(decide(&desktop, shared.updated_at, Some(&shared), None, read(&file).as_ref()), Step::Nothing);
        let back = Shared { based_on: 20, ..Shared::from_config(&desktop, 30, "DESKTOP") };
        assert_eq!(decide(&laptop, 20, written.as_ref(), None, Some(&back)), Step::Nothing);

        // A merge written back is taken over as it is by the other computer, and settles.
        let mut desktop_change = desktop.clone();
        desktop_change.courses[0].keywords.push("desk".into());
        let mut laptop_change = laptop.clone();
        laptop_change.courses[1].keywords.push("lap".into());
        let theirs = Shared { based_on: 20, ..Shared::from_config(&desktop_change, 40, "DESKTOP") };
        let Step::Merge(merged) = decide(&laptop_change, 20, written.as_ref(), Some(45), Some(&theirs)) else {
            panic!("should merge")
        };
        assert!(merged.apply_to(&mut laptop_change));
        let laptop_wrote = Shared { based_on: 40, ..Shared::from_config(&laptop_change, 50, "LAPTOP") };
        let Step::Apply(back) = decide(&desktop_change, 40, Some(&theirs), None, Some(&laptop_wrote)) else {
            panic!("should apply")
        };
        assert!(back.apply_to(&mut desktop_change));
        assert_eq!(decide(&desktop_change, 50, Some(&back), None, Some(&laptop_wrote)), Step::Nothing);
    }
}
