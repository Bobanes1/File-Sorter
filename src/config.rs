//! Loading and saving the user's settings (watch folder, study folder, modules, keywords).

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Deserializer, Serialize};

use crate::matcher::normalise;

pub const DEFAULT_EXTENSIONS: &[&str] =
    &[".pdf", ".pptx", ".ppt", ".docx", ".doc", ".key", ".odp", ".odt", ".txt", ".md", ".zip", ".ipynb", ".xlsx"];
/// More types offered in the settings, off unless chosen.
pub const EXTRA_EXTENSIONS: &[&str] =
    &[".xls", ".csv", ".rtf", ".epub", ".tex", ".png", ".jpg", ".py", ".java", ".rar", ".7z", ".mp4"];

/// Per-user location of config.json, following each OS's conventions.
pub fn default_config_path() -> PathBuf {
    let base = dirs::config_dir().unwrap_or_else(|| home().join(".config"));
    let app = if cfg!(any(target_os = "windows", target_os = "macos")) { "FileSorter" } else { "file-sorter" };
    base.join(app).join("config.json")
}

fn home() -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| PathBuf::from("."))
}

/// Expand a leading `~` to the home directory.
pub fn expand_user(path: &str) -> PathBuf {
    if path == "~" {
        return home();
    }
    if let Some(rest) = path.strip_prefix("~/").or_else(|| path.strip_prefix("~\\")) {
        return home().join(rest);
    }
    PathBuf::from(path)
}

/// Treat `null` like a missing value (older hand-edited configs may contain it).
fn null_as_default<'de, D, T>(d: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: Default + Deserialize<'de>,
{
    Ok(Option::<T>::deserialize(d)?.unwrap_or_default())
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Course {
    pub name: String,
    #[serde(default, deserialize_with = "null_as_default")]
    pub keywords: Vec<String>,
    /// Optional custom folder; when empty the module goes to destination_root/[subject/]name.
    #[serde(default, deserialize_with = "null_as_default")]
    pub folder: String,
    /// Study program the module belongs to (a folder directly under destination_root).
    #[serde(default, deserialize_with = "null_as_default")]
    pub subject: String,
}

impl Course {
    pub fn new(name: impl Into<String>, keywords: &[&str]) -> Self {
        Course {
            name: name.into(),
            keywords: keywords.iter().map(|k| k.to_string()).collect(),
            folder: String::new(),
            subject: String::new(),
        }
    }

    pub fn with_subject(mut self, subject: impl Into<String>) -> Self {
        self.subject = subject.into();
        self
    }

    pub fn label(&self) -> String {
        if self.subject.is_empty() { self.name.clone() } else { format!("{} › {}", self.subject, self.name) }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub watch_folder: String,
    pub destination_root: String,
    pub extensions: Vec<String>,
    pub read_file_contents: bool,
    pub sort_existing_on_start: bool,
    pub courses: Vec<Course>,
    /// "de" or "en"; empty means: follow the system language.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    /// Offer "Sort with File Sorter" in Explorer's right-click menu (Windows).
    pub context_menu: bool,
    /// Move files into the module folder (default), or copy them and leave the original.
    pub action: FileAction,
    pub theme: ThemeChoice,
    /// Flying files, sliding cards and the pulsing status dot.
    pub animations: bool,
    /// Start watching as soon as the window opens.
    pub watch_on_open: bool,
    /// Start File Sorter (minimised, watching) when logging in.
    pub autostart: bool,
    /// Show the small results window after sorting from the right-click menu.
    pub results_window: bool,
    /// Keep watching in the notification area when the window is closed (Windows).
    pub background: bool,
    /// Folder for File Sorter's own lists (copied files, files to leave alone, history):
    /// the folder of config.json. Set at runtime, not saved.
    #[serde(skip)]
    pub state_dir: Option<PathBuf>,
}

/// What happens to a file once its module is known.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FileAction {
    /// The file ends up only in the module folder.
    #[default]
    Move,
    /// The original stays in the watched folder, a copy goes to the module folder.
    Copy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemeChoice {
    /// Follow the system's light/dark setting.
    #[default]
    System,
    Light,
    Dark,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            watch_folder: home().join("Downloads").to_string_lossy().into_owned(),
            destination_root: home().join("University").to_string_lossy().into_owned(),
            extensions: DEFAULT_EXTENSIONS.iter().map(|e| e.to_string()).collect(),
            read_file_contents: true,
            sort_existing_on_start: false,
            courses: Vec::new(),
            language: None,
            context_menu: true,
            action: FileAction::Move,
            theme: ThemeChoice::System,
            animations: true,
            watch_on_open: false,
            autostart: false,
            results_window: true,
            background: false,
            state_dir: None,
        }
    }
}

impl Config {
    /// One of File Sorter's own lists, if the settings were loaded from a file.
    pub fn state_file(&self, name: &str) -> Option<PathBuf> {
        self.state_dir.as_ref().map(|dir| dir.join(name))
    }

    pub fn watch_path(&self) -> PathBuf {
        expand_user(&self.watch_folder)
    }

    pub fn course_folder(&self, course: &Course) -> PathBuf {
        if !course.folder.is_empty() {
            return expand_user(&course.folder);
        }
        let mut root = expand_user(&self.destination_root);
        if !course.subject.is_empty() {
            root = root.join(safe_folder_name(&course.subject));
        }
        root.join(safe_folder_name(&course.name))
    }

    pub fn from_json(text: &str) -> serde_json::Result<Config> {
        // Notepad and PowerShell may add a byte-order mark when editing by hand.
        let mut config: Config = serde_json::from_str(text.trim_start_matches('\u{feff}'))?;
        config.clean();
        Ok(config)
    }

    /// Trim names/keywords, drop nameless modules and normalise extensions.
    fn clean(&mut self) {
        self.courses.retain_mut(|c| {
            c.name = c.name.trim().to_string();
            c.subject = c.subject.trim().to_string();
            c.keywords = c.keywords.iter().map(|k| k.trim().to_string()).filter(|k| !k.is_empty()).collect();
            !c.name.is_empty()
        });
        self.extensions = self.extensions.iter().map(|e| normalise_ext(e)).collect();
    }
}

pub(crate) fn normalise_ext(ext: &str) -> String {
    let ext = ext.trim().to_lowercase();
    if ext.starts_with('.') { ext } else { format!(".{ext}") }
}

/// Strip characters that are not allowed in folder names on Windows/macOS/Linux.
pub fn safe_folder_name(name: &str) -> String {
    let cleaned: String = name.chars().map(|c| if r#"<>:"/\|?*"#.contains(c) { '_' } else { c }).collect();
    let cleaned = cleaned.trim().trim_end_matches('.').to_string();
    if cleaned.is_empty() { "Unnamed Course".to_string() } else { cleaned }
}

pub fn load_config(path: &Path) -> io::Result<Config> {
    let mut config = if path.exists() {
        let text = fs::read_to_string(path)?;
        Config::from_json(&text).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?
    } else {
        Config::default()
    };
    config.state_dir = Some(config_path_dir(path));
    Ok(config)
}

/// The folder holding config.json, the log and File Sorter's lists.
pub fn config_path_dir(config_path: &Path) -> PathBuf {
    config_path.parent().unwrap_or(Path::new(".")).to_path_buf()
}

pub fn save_config(config: &Config, path: &Path) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("tmp");
    let json = serde_json::to_string_pretty(config).map_err(io::Error::other)?;
    fs::write(&tmp, json)?;
    fs::rename(&tmp, path)
}

/// Words that say nothing about which module a file belongs to.
pub(crate) const STOPWORDS: &[&str] = &[
    "und",
    "oder",
    "der",
    "die",
    "das",
    "den",
    "dem",
    "des",
    "ein",
    "eine",
    "fur",
    "mit",
    "von",
    "zur",
    "zum",
    "einfuhrung",
    "grundlagen",
    "vorlesung",
    "ubung",
    "ubungen",
    "seminar",
    "modul",
    "kurs",
    "praktikum",
    "teil",
    "and",
    "the",
    "for",
    "with",
    "from",
    "introduction",
    "intro",
    "basics",
    "fundamentals",
    "lecture",
    "course",
    "module",
    "part",
];

/// Short filler words dropped when building a module's core name ("Einführung in die Analysis I" → "analysis i").
pub(crate) const FILLER_WORDS: &[&str] = &["in", "im", "zu", "an", "am", "auf", "of", "to", "on", "a"];

fn core_name(name: &str) -> String {
    normalise(name)
        .split(' ')
        .filter(|w| !STOPWORDS.contains(w) && !FILLER_WORDS.contains(w))
        .collect::<Vec<_>>()
        .join(" ")
}

fn name_words(name: &str) -> Vec<String> {
    normalise(name)
        .split(' ')
        .filter(|w| w.chars().count() > 3 && !STOPWORDS.contains(w) && !w.chars().all(char::is_numeric))
        .map(str::to_string)
        .collect()
}

fn visible_subdirs(dir: &Path) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = fs::read_dir(dir)
        .map(|entries| {
            entries.flatten().map(|e| e.path()).filter(|p| p.is_dir() && !file_name(p).starts_with('.')).collect()
        })
        .unwrap_or_default();
    dirs.sort();
    dirs
}

fn file_name(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

/// Find module folders laid out as root/<subject>/<module>.
///
/// Each module gets the significant words of its name as starter keywords; words
/// shared by several modules (e.g. "Analysis" in "Analysis I" and "Analysis II")
/// are left out because they can't tell the modules apart. The full module name
/// is matched anyway, which separates those, and so is its core name without
/// filler words ("Einführung in die Analysis I" → "analysis i").
pub fn scan_study_folder(root: &Path) -> Vec<Course> {
    if !root.is_dir() {
        return Vec::new();
    }
    let mut found: Vec<Course> = Vec::new();
    for subject_dir in visible_subdirs(root) {
        for module_dir in visible_subdirs(&subject_dir) {
            found.push(Course::new(file_name(&module_dir), &[]).with_subject(file_name(&subject_dir)));
        }
    }

    let mut counts: HashMap<String, usize> = HashMap::new();
    let mut cores: HashMap<String, usize> = HashMap::new();
    for course in &found {
        for word in name_words(&course.name).into_iter().collect::<HashSet<_>>() {
            *counts.entry(word).or_default() += 1;
        }
        *cores.entry(core_name(&course.name)).or_default() += 1;
    }
    for course in &mut found {
        let mut keywords: Vec<String> = Vec::new();
        for word in name_words(&course.name) {
            if counts[&word] == 1 && !keywords.contains(&word) {
                keywords.push(word);
            }
        }
        // The name without filler words, if that differs from the full name and is unique.
        let core = core_name(&course.name);
        if !core.is_empty() && !keywords.contains(&core) && cores[&core] == 1 && core != normalise(&course.name) {
            keywords.insert(0, core);
        }
        course.keywords = keywords;
    }
    found
}

/// Add modules from `found` that aren't in `existing` yet. Returns how many were added.
pub fn merge_courses(existing: &mut Vec<Course>, found: Vec<Course>) -> usize {
    let key = |c: &Course| (c.subject.to_lowercase(), c.name.to_lowercase());
    let mut known: HashSet<_> = existing.iter().map(key).collect();
    let mut added = 0;
    for course in found {
        if known.insert(key(&course)) {
            existing.push(course);
            added += 1;
        }
    }
    added
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_tree(root: &Path) -> PathBuf {
        for sub in [
            "Informatik/Programmierung 1",
            "Informatik/Einführung in die Analysis I",
            "Mathematik/Analysis II",
            "Mathematik/Lineare Algebra",
            "Mathematik/.hidden",
        ] {
            fs::create_dir_all(root.join("Studium").join(sub)).unwrap();
        }
        fs::write(root.join("Studium").join("notes.txt"), "loose file").unwrap();
        root.join("Studium")
    }

    #[test]
    fn course_folder_with_and_without_subject() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = Config { destination_root: tmp.path().to_string_lossy().into(), ..Config::default() };
        let ana = Course::new("Analysis II", &[]).with_subject("Mathematik");
        assert_eq!(cfg.course_folder(&ana), tmp.path().join("Mathematik").join("Analysis II"));
        assert_eq!(cfg.course_folder(&Course::new("Statistik", &[])), tmp.path().join("Statistik"));
        let mut custom = Course::new("X", &[]).with_subject("M");
        custom.folder = tmp.path().join("custom").to_string_lossy().into();
        assert_eq!(cfg.course_folder(&custom), tmp.path().join("custom"));
    }

    #[test]
    fn label() {
        assert_eq!(Course::new("Analysis II", &[]).with_subject("Mathematik").label(), "Mathematik › Analysis II");
        assert_eq!(Course::new("Statistik", &[]).label(), "Statistik");
    }

    #[test]
    fn safe_names() {
        assert_eq!(safe_folder_name("A/B: C?."), "A_B_ C_");
        assert_eq!(safe_folder_name("  "), "Unnamed Course");
    }

    #[test]
    fn scan_finds_modules_with_starter_keywords() {
        let tmp = tempfile::tempdir().unwrap();
        let found = scan_study_folder(&make_tree(tmp.path()));
        let get = |s: &str, n: &str| found.iter().find(|c| c.subject == s && c.name == n).map(|c| c.keywords.clone());
        assert_eq!(found.len(), 4);
        assert_eq!(get("Informatik", "Programmierung 1").unwrap(), vec!["programmierung"]);
        // "analysis" alone is shared by two modules and "einführung" is a stopword,
        // but the core name "analysis i" identifies the module.
        assert_eq!(get("Informatik", "Einführung in die Analysis I").unwrap(), vec!["analysis i"]);
        assert_eq!(get("Mathematik", "Lineare Algebra").unwrap(), vec!["lineare", "algebra"]);
        assert!(get("Mathematik", ".hidden").is_none());
    }

    #[test]
    fn scan_missing_folder() {
        assert!(scan_study_folder(Path::new("/definitely/not/here")).is_empty());
    }

    #[test]
    fn merge_keeps_existing() {
        let tmp = tempfile::tempdir().unwrap();
        let studium = make_tree(tmp.path());
        let mut existing = vec![Course::new("Programmierung 1", &["prog1", "java"]).with_subject("Informatik")];
        assert_eq!(merge_courses(&mut existing, scan_study_folder(&studium)), 3);
        assert_eq!(existing.len(), 4);
        assert_eq!(existing[0].keywords, vec!["prog1", "java"]);
        assert_eq!(merge_courses(&mut existing, scan_study_folder(&studium)), 0);
    }

    #[test]
    fn roundtrip_and_defaults() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("cfg").join("config.json");
        let cfg = Config {
            courses: vec![Course::new("Statistik", &["stats"]).with_subject("Mathematik")],
            action: FileAction::Copy,
            theme: ThemeChoice::Dark,
            autostart: true,
            state_dir: Some(tmp.path().join("cfg")),
            ..Config::default()
        };
        save_config(&cfg, &path).unwrap();
        let saved = fs::read_to_string(&path).unwrap();
        assert!(saved.contains("\"action\": \"copy\"") && !saved.contains("state_dir"), "{saved}");
        assert_eq!(load_config(&path).unwrap(), cfg);
        let missing = tmp.path().join("nope.json");
        let defaults = Config { state_dir: Some(tmp.path().to_path_buf()), ..Config::default() };
        assert_eq!(load_config(&missing).unwrap(), defaults);
    }

    #[test]
    fn reads_python_era_config_with_bom_and_nulls() {
        let text = "\u{feff}{\"watch_folder\": \"~/Downloads\", \"extensions\": [\"PDF\", \".pptx\"],\
                    \"courses\": [{\"name\": \" Statistik \", \"keywords\": [\"stats\", \" \"], \"folder\": null},\
                                  {\"name\": \"\"}]}";
        let cfg = Config::from_json(text).unwrap();
        assert_eq!(cfg.courses.len(), 1);
        assert_eq!(cfg.courses[0].name, "Statistik");
        assert_eq!(cfg.courses[0].subject, "");
        assert_eq!(cfg.courses[0].keywords, vec!["stats"]);
        assert_eq!(cfg.extensions, vec![".pdf", ".pptx"]);
        assert!(cfg.read_file_contents);
        assert_eq!(cfg.watch_path(), home().join("Downloads"));
    }
}
