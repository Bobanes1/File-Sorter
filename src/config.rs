//! Loading and saving the user's settings (watch folder, study folder, modules, keywords).

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::categories::Category;
use serde::{Deserialize, Deserializer, Serialize};

use crate::cloud::CloudSettings;
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
        // Part by part, so "~/Studium/Prog" gets the system's separators.
        return rest.split(['/', '\\']).filter(|p| !p.is_empty()).fold(home(), |path, part| path.join(part));
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
    /// Sort the files already in the watched folder when watching starts: none, only the
    /// ones from the last days, or all.
    #[serde(deserialize_with = "on_start_or_bool")]
    pub sort_existing_on_start: OnStart,
    /// "Sort existing files" in the window: the last 24 hours, the last 7 days or everything.
    /// (0.9 saved `sort_existing_recent`: true for 7 days, false for all.)
    #[serde(alias = "sort_existing_recent", deserialize_with = "range_or_bool")]
    pub sort_existing_range: SortRange,
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
    /// The study folder in OneDrive, Dropbox, Google Drive, … (moved there, or a copy).
    pub cloud: CloudSettings,
    /// Features that can be switched on or off in the settings.
    pub features: Features,
    /// Modules of finished semesters, moved into the archive (they can be brought back).
    pub archived: Vec<crate::archive::Archived>,
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

/// Which of the files already in the watched folder "Sort existing files" takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SortRange {
    /// Downloaded or changed in the last 24 hours.
    Day,
    /// … in the last 7 days.
    #[default]
    Week,
    /// Everything.
    All,
}

impl SortRange {
    pub fn scope(self) -> ExistingScope {
        match self {
            SortRange::Day => ExistingScope::LastHours(24),
            SortRange::Week => ExistingScope::LastHours(7 * 24),
            SortRange::All => ExistingScope::All,
        }
    }
}

/// 0.9 saved `true` (last 7 days) / `false` (all) here.
fn range_or_bool<'de, D: Deserializer<'de>>(d: D) -> Result<SortRange, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Either {
        Old(bool),
        New(SortRange),
    }
    Ok(match Option::<Either>::deserialize(d)? {
        Some(Either::Old(true)) | None => SortRange::Week,
        Some(Either::Old(false)) => SortRange::All,
        Some(Either::New(range)) => range,
    })
}

/// Which files already in the watched folder are sorted when watching starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OnStart {
    /// Only new downloads.
    #[default]
    Off,
    /// Files from the last 24 hours too.
    Day,
    /// Files from the last 7 days too (saved as "recent", as in 0.9).
    #[serde(rename = "recent")]
    Week,
    /// Everything that is there.
    All,
}

impl OnStart {
    /// What to sort when watching starts, if anything.
    pub fn scope(self) -> Option<ExistingScope> {
        match self {
            OnStart::Off => None,
            OnStart::Day => Some(SortRange::Day.scope()),
            OnStart::Week => Some(SortRange::Week.scope()),
            OnStart::All => Some(ExistingScope::All),
        }
    }
}

/// Versions before 0.9 saved `true` / `false` here.
fn on_start_or_bool<'de, D: Deserializer<'de>>(d: D) -> Result<OnStart, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Either {
        Old(bool),
        New(OnStart),
    }
    Ok(match Option::<Either>::deserialize(d)? {
        Some(Either::Old(true)) => OnStart::All,
        Some(Either::Old(false)) | None => OnStart::Off,
        Some(Either::New(choice)) => choice,
    })
}

/// Which of the files already in the watched folder to sort.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExistingScope {
    All,
    /// Only files downloaded (or changed) in the last so many hours.
    LastHours(u32),
}

/// Whether updates are installed by themselves or only offered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UpdateMode {
    #[default]
    Ask,
    Auto,
}

/// Features that can be switched on or off in the settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Features {
    /// "Assign" a file to a module by hand, and learn keywords from it.
    pub learn: bool,
    /// Use the web page a file was downloaded from (Moodle course, …) to find its module.
    pub origin: bool,
    /// Sort into subfolders inside the module (Vorlesung, Übungen, …).
    pub subfolders: bool,
    /// The subfolders every module has, and the words that send a file into each.
    pub subfolder_list: Vec<Subfolder>,
    /// Words learned from the files already in the subfolders, per subfolder.
    pub subfolder_learned: Vec<LearnedFolder>,
    /// Learn them by itself now and then (else only on «Learn»).
    pub subfolder_auto_learn: bool,
    /// 0.9/0.10 saved only the names of the four built-in subfolders here.
    #[serde(skip_serializing, rename = "subfolder_names")]
    pub old_subfolder_names: Option<[String; 4]>,
    /// Unpack zip files into the module.
    pub unzip: bool,
    /// Put the older version of a file into "Ältere Versionen" when a new one comes.
    pub versions: bool,
    /// Name of that folder (inside the module or subfolder).
    pub versions_folder: String,
    /// Search the text of all documents in the study folder.
    pub search: bool,
    /// Find submission deadlines in exercise sheets.
    pub deadlines: bool,
    /// "New this week" on the overview.
    pub weekly: bool,
    /// Look for new versions of File Sorter.
    pub update_check: bool,
    pub update_install: UpdateMode,
    /// When two modules fit about equally well, leave the file and ask.
    pub ask_when_unsure: bool,
    /// Learn the modules from the files already in them, for files the keywords don't decide.
    pub classifier: bool,
    /// Recognise the text of scanned PDFs and pictures.
    pub ocr: bool,
}

impl Default for Features {
    fn default() -> Self {
        Features {
            learn: true,
            origin: true,
            subfolders: false,
            subfolder_list: Subfolder::defaults(),
            subfolder_learned: Vec::new(),
            subfolder_auto_learn: false,
            old_subfolder_names: None,
            unzip: false,
            versions: false,
            versions_folder: "Ältere Versionen".to_string(),
            search: true,
            deadlines: true,
            weekly: true,
            update_check: true,
            update_install: UpdateMode::Ask,
            ask_when_unsure: true,
            classifier: true,
            ocr: true,
        }
    }
}

/// One subfolder in every module.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Subfolder {
    pub name: String,
    /// Words that send a file here when its name (or the start of the text) contains them.
    #[serde(default)]
    pub keywords: Vec<String>,
    /// One of the kinds File Sorter knows by itself (lecture slides, exercises, solutions,
    /// exams), which come with their own words in German and English.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub builtin: Option<Category>,
}

/// Words that the files in a subfolder have in their names, learned from them ("vl" for
/// «Vorlesung», "musterlosung" for «Lösungen»), and the ones the user took out.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct LearnedFolder {
    pub name: String,
    pub words: Vec<String>,
    /// Taken out by the user: not learned again.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub removed: Vec<String>,
}

impl Subfolder {
    pub fn defaults() -> Vec<Subfolder> {
        [
            ("Vorlesung", Category::Lecture),
            ("Übungen", Category::Exercise),
            ("Lösungen", Category::Solution),
            ("Prüfungen", Category::Exam),
        ]
        .into_iter()
        .map(|(name, kind)| Subfolder { name: name.to_string(), keywords: Vec::new(), builtin: Some(kind) })
        .collect()
    }
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
            sort_existing_on_start: OnStart::Off,
            sort_existing_range: SortRange::Week,
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
            cloud: CloudSettings::default(),
            features: Features::default(),
            archived: Vec::new(),
            state_dir: None,
        }
    }
}

impl Config {
    /// What "Sort existing files" sorts: the last 24 hours, the last 7 days or everything.
    pub fn existing_scope(&self) -> ExistingScope {
        self.sort_existing_range.scope()
    }

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
        // Names of the built-in subfolders from 0.9/0.10.
        if let Some(names) = config.features.old_subfolder_names.take()
            && config.features.subfolder_list == Subfolder::defaults()
        {
            for (folder, name) in config.features.subfolder_list.iter_mut().zip(names) {
                if !name.trim().is_empty() {
                    folder.name = name;
                }
            }
        }
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
    crate::ocr::set_cache_dir(&config_path_dir(path));
    crate::ocr::set_enabled(config.features.ocr);
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
    // The semester archive holds finished modules, not a study program.
    for subject_dir in visible_subdirs(root).into_iter().filter(|d| file_name(d) != crate::archive::ARCHIVE_FOLDER) {
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
    fn old_on_start_switch_and_missing_features_still_read() {
        let read = |json: &str| serde_json::from_str::<Config>(json).unwrap();
        assert_eq!(read(r#"{"sort_existing_on_start": true}"#).sort_existing_on_start, OnStart::All);
        assert_eq!(read(r#"{"sort_existing_on_start": false}"#).sort_existing_on_start, OnStart::Off);
        assert_eq!(read(r#"{"sort_existing_on_start": "recent"}"#).sort_existing_on_start, OnStart::Week);
        assert_eq!(read(r#"{"sort_existing_on_start": "day"}"#).sort_existing_on_start, OnStart::Day);
        assert_eq!(read(r#"{"sort_existing_recent": false}"#).sort_existing_range, SortRange::All);
        assert_eq!(read(r#"{"sort_existing_recent": true}"#).sort_existing_range, SortRange::Week);
        assert_eq!(read(r#"{"sort_existing_range": "day"}"#).existing_scope(), ExistingScope::LastHours(24));
        let old = read("{}");
        assert_eq!(old.sort_existing_on_start, OnStart::Off);
        assert_eq!(old.features, Features::default());
        assert_eq!(old.existing_scope(), ExistingScope::LastHours(7 * 24));
        // 0.9/0.10 had only the four names; they carry over, words can be added now.
        let named =
            Config::from_json(r#"{"features": {"subfolders": true, "subfolder_names": ["VL", "Ü", "L", "P"]}}"#)
                .unwrap();
        let names: Vec<_> = named.features.subfolder_list.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, ["VL", "Ü", "L", "P"]);
        assert!(!serde_json::to_string(&named).unwrap().contains("subfolder_names"));
        assert_eq!(read("{}").features.subfolder_list, Subfolder::defaults());
        let partial = read(r#"{"features": {"unzip": true}}"#).features;
        assert!(partial.unzip && partial.learn && !partial.subfolders);
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
