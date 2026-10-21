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
    /// Credit points.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ects: Option<f32>,
    /// The grade, on the scale chosen in the settings.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grade: Option<f32>,
    /// Passed or failed without a grade (pass/fail modules), or still open.
    #[serde(default, skip_serializing_if = "Status::is_open")]
    pub status: Status,
    /// When it was taken («HS 2026»).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub semester: String,
    /// The tests, papers, … that make up the grade, each with its weight.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub assessments: Vec<Assessment>,
    /// Taken out of the grades list (the module itself stays for sorting).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub hide_grades: bool,
}

/// One test, paper or project of a module, and how much it counts.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Assessment {
    pub name: String,
    /// As typed: «1/6», «30 %», «0.25» or a relative weight like «2»; empty counts as 1.
    pub weight: String,
    pub grade: Option<f32>,
}

impl Assessment {
    /// The weight as a number (relative to the others), if it can be read.
    pub fn weight_value(&self) -> Option<f32> {
        parse_weight(&self.weight)
    }
}

/// «1/6» → 0.1667, «30 %» → 0.3, «0,25» → 0.25, «2» → 2, empty → 1.
pub fn parse_weight(text: &str) -> Option<f32> {
    let text = text.trim().replace(',', ".");
    if text.is_empty() {
        return Some(1.0);
    }
    let value = if let Some(percent) = text.strip_suffix('%') {
        percent.trim().parse::<f32>().ok()? / 100.0
    } else if let Some((a, b)) = text.split_once('/') {
        let (a, b) = (a.trim().parse::<f32>().ok()?, b.trim().parse::<f32>().ok()?);
        if b == 0.0 {
            return None;
        }
        a / b
    } else {
        text.parse::<f32>().ok()?
    };
    (value.is_finite() && value > 0.0).then_some(value)
}

/// Where a module stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    #[default]
    Open,
    Passed,
    Failed,
}

impl Status {
    fn is_open(&self) -> bool {
        *self == Status::Open
    }
}

/// How grades are written.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GradeScale {
    /// 1 to 6, 6 best, 4 and up passed.
    #[default]
    Swiss,
    /// 1.0 to 5.0, 1.0 best, up to 4.0 passed.
    German,
}

impl GradeScale {
    pub fn range(self) -> (f32, f32) {
        match self {
            GradeScale::Swiss => (1.0, 6.0),
            GradeScale::German => (1.0, 5.0),
        }
    }

    pub fn passed(self, grade: f32) -> bool {
        match self {
            GradeScale::Swiss => grade >= 4.0,
            GradeScale::German => grade <= 4.0,
        }
    }
}

/// What the grades add up to.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct GradeSummary {
    /// The ECTS-weighted average of the graded modules (with ECTS).
    pub weighted: Option<f32>,
    /// The plain average of all graded modules.
    pub simple: Option<f32>,
    /// Modules with a grade.
    pub graded: usize,
    /// ECTS of passed modules.
    pub earned: f32,
    /// ECTS of the modules with a grade.
    pub ects_graded: f32,
}

/// The averages, the number of graded modules and the ECTS earned. A module's grade is its
/// own, or the one its tests give.
pub fn grade_summary(courses: &[&Course], scale: GradeScale) -> GradeSummary {
    let grades: Vec<(f32, f32)> =
        courses.iter().filter_map(|c| Some((c.final_grade()?.0, c.ects.unwrap_or(0.0)))).collect();
    let with_ects: Vec<&(f32, f32)> = grades.iter().filter(|(_, e)| *e > 0.0).collect();
    // (`+ 0.0`: an empty float sum is -0.)
    let ects_graded: f32 = with_ects.iter().map(|(_, e)| e).sum::<f32>() + 0.0;
    let weighted = (ects_graded > 0.0).then(|| with_ects.iter().map(|(g, e)| g * e).sum::<f32>() / ects_graded);
    let simple = (!grades.is_empty()).then(|| grades.iter().map(|(g, _)| g).sum::<f32>() / grades.len() as f32);
    let earned: f32 = courses
        .iter()
        .filter(|c| match c.final_grade() {
            Some((g, true)) => scale.passed(g),
            Some((_, false)) => false,
            None => c.status == Status::Passed,
        })
        .filter_map(|c| c.ects)
        .sum::<f32>()
        + 0.0;
    GradeSummary { weighted, simple, graded: grades.len(), earned, ects_graded }
}

impl Course {
    pub fn new(name: impl Into<String>, keywords: &[&str]) -> Self {
        Course {
            name: name.into(),
            keywords: keywords.iter().map(|k| k.to_string()).collect(),
            folder: String::new(),
            subject: String::new(),
            ects: None,
            grade: None,
            status: Status::Open,
            semester: String::new(),
            assessments: Vec::new(),
            hide_grades: false,
        }
    }

    /// The module's grade: from its tests if it has graded ones (weighted; `false` while
    /// some are still open), else the one entered.
    pub fn final_grade(&self) -> Option<(f32, bool)> {
        let graded: Vec<(f32, f32)> =
            self.assessments.iter().filter_map(|a| Some((a.grade?, a.weight_value()?))).collect();
        if graded.is_empty() {
            return self.grade.map(|g| (g, true));
        }
        let total: f32 = graded.iter().map(|(_, w)| w).sum();
        let grade = graded.iter().map(|(g, w)| g * w).sum::<f32>() / total;
        Some((grade, graded.len() == self.assessments.len()))
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
    /// While the window is closed or minimised: use less memory and reading.
    pub lean_background: bool,
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
    /// Show it also when files are sorted by themselves (watching, sorting existing files)
    /// while the window isn't in front.
    pub auto_results_window: bool,
    /// Modules directly in the study folder (`Studium/Statistik`), without a folder per
    /// study program (`Studium/Informatik/Statistik`).
    pub flat_modules: bool,
    /// Keep watching in the notification area when the window is closed (Windows).
    pub background: bool,
    /// The study folder in OneDrive, Dropbox, Google Drive, … (moved there, or a copy).
    pub cloud: CloudSettings,
    /// The study folder kept in sync with Google Drive by File Sorter itself (no Google app).
    pub gdrive: crate::cloud::gdrive::DriveSettings,
    /// Features that can be switched on or off in the settings.
    pub features: Features,
    /// Modules of finished semesters, moved into the archive (they can be brought back).
    pub archived: Vec<crate::archive::Archived>,
    /// Setting up the AI model Laya was offered (at the setup, or once after the update).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub laya_offered: bool,
    /// Modules only in the grades list (no folder, nothing sorted into them).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub grade_modules: Vec<Course>,
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
    /// Use the AI model Laya (once it is set up): it alone decides each file's module and
    /// subfolder, and it finds deadlines and exams in any language.
    pub laya: bool,
    /// Recognise the text of scanned PDFs and pictures.
    pub ocr: bool,
    /// Remind of deadlines and exams with a system notification.
    pub reminders: bool,
    /// Between modules that fit equally well, prefer the one whose lesson is running.
    pub timetable_hint: bool,
    /// When before a deadline.
    pub reminder_leads: Vec<crate::reminders::Lead>,
    /// What deadlines and appointments are put into («Übung», «Prüfung», …).
    pub deadline_categories: Vec<crate::deadlines::Category>,
    /// How grades are written.
    pub grade_scale: GradeScale,
    /// ECTS needed for the degree.
    pub ects_target: f32,
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
            laya: true,
            ocr: true,
            reminders: true,
            timetable_hint: true,
            reminder_leads: crate::reminders::default_leads(),
            deadline_categories: crate::deadlines::Category::defaults(),
            grade_scale: GradeScale::Swiss,
            ects_target: 180.0,
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
            lean_background: false,
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
            auto_results_window: true,
            flat_modules: false,
            background: false,
            cloud: CloudSettings::default(),
            gdrive: Default::default(),
            features: Features::default(),
            archived: Vec::new(),
            laya_offered: false,
            grade_modules: Vec::new(),
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
    crate::laya::configure(&config_path_dir(path), config.features.laya);
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

/// Find module folders laid out as root/<subject>/<module>, or with `flat` as root/<module>
/// (where folders named in `programs` are still study programs with modules in them: those
/// not moved up yet).
///
/// Each module gets the significant words of its name as starter keywords; words
/// shared by several modules (e.g. "Analysis" in "Analysis I" and "Analysis II")
/// are left out because they can't tell the modules apart. The full module name
/// is matched anyway, which separates those, and so is its core name without
/// filler words ("Einführung in die Analysis I" → "analysis i").
pub fn scan_study_folder(root: &Path, flat: bool, programs: &[String]) -> Vec<Course> {
    if !root.is_dir() {
        return Vec::new();
    }
    let mut found: Vec<Course> = Vec::new();
    // The semester archive holds finished modules, not a study program.
    for subject_dir in visible_subdirs(root).into_iter().filter(|d| file_name(d) != crate::archive::ARCHIVE_FOLDER) {
        let name = file_name(&subject_dir);
        if flat && !programs.iter().any(|p| p.eq_ignore_ascii_case(&name)) {
            found.push(Course::new(name, &[]));
            continue;
        }
        for module_dir in visible_subdirs(&subject_dir) {
            found.push(Course::new(file_name(&module_dir), &[]).with_subject(name.clone()));
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

/// Whether the folders directly in `root` look like modules rather than study programs:
/// most of them hold documents themselves, or only the usual subfolders of a module
/// («Vorlesung», «Übungen», …).
pub fn looks_flat(root: &Path, subfolders: &[Subfolder]) -> bool {
    let dirs: Vec<PathBuf> =
        visible_subdirs(root).into_iter().filter(|d| file_name(d) != crate::archive::ARCHIVE_FOLDER).collect();
    if dirs.is_empty() {
        return false;
    }
    const USUAL: &[&str] = &[
        "vorlesung",
        "vorlesungen",
        "vl",
        "folien",
        "skript",
        "ubung",
        "ubungen",
        "uebungen",
        "serien",
        "blatter",
        "losungen",
        "loesungen",
        "prufungen",
        "altklausuren",
        "notizen",
        "literatur",
        "lectures",
        "slides",
        "exercises",
        "solutions",
        "exams",
        "notes",
    ];
    let is_subfolder = |name: &str| {
        let name = normalise(name);
        USUAL.contains(&name.as_str())
            || subfolders.iter().any(|f| normalise(&f.name) == name || f.keywords.iter().any(|w| normalise(w) == name))
    };
    let module_like = dirs
        .iter()
        .filter(|dir| {
            let has_files = fs::read_dir(dir).is_ok_and(|entries| {
                entries.flatten().any(|e| {
                    e.file_type().is_ok_and(|t| t.is_file()) && !e.file_name().to_string_lossy().starts_with('.')
                })
            });
            let children = visible_subdirs(dir);
            has_files || (!children.is_empty() && children.iter().all(|c| is_subfolder(&file_name(c))))
        })
        .count();
    module_like * 2 > dirs.len()
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
    fn grade_averages_on_both_scales() {
        let course = |grade: Option<f32>, ects: f32, status: Status| Course {
            grade,
            ects: Some(ects),
            status,
            ..Course::new("x", &[])
        };
        let swiss = [
            course(Some(5.5), 6.0, Status::Open),
            course(Some(4.0), 3.0, Status::Open),
            course(Some(3.5), 3.0, Status::Open),
            course(None, 2.0, Status::Passed),
        ];
        let refs: Vec<&Course> = swiss.iter().collect();
        let sum = grade_summary(&refs, GradeScale::Swiss);
        assert_eq!(sum.weighted, Some((5.5 * 6.0 + 4.0 * 3.0 + 3.5 * 3.0) / 12.0));
        assert_eq!(sum.simple, Some((5.5 + 4.0 + 3.5) / 3.0));
        assert_eq!((sum.earned, sum.ects_graded, sum.graded), (11.0, 12.0, 3), "3.5 failed; pass/fail counts");
        let german = [course(Some(1.3), 6.0, Status::Open), course(Some(4.3), 6.0, Status::Open)];
        let refs: Vec<&Course> = german.iter().collect();
        assert_eq!(grade_summary(&refs, GradeScale::German).earned, 6.0, "4.3 failed");
        assert!(GradeScale::German.passed(4.0) && !GradeScale::Swiss.passed(3.9));
        // Nothing yet: 0, not -0.
        let none = grade_summary(&[], GradeScale::Swiss);
        assert_eq!(format!("{:.0}", none.earned), "0");
        assert_eq!(none.weighted, None);
    }

    #[test]
    fn grades_from_weighted_tests() {
        assert_eq!(parse_weight("1/6"), Some(1.0 / 6.0));
        assert_eq!(parse_weight("30 %"), Some(0.3));
        assert_eq!(parse_weight("0,25"), Some(0.25));
        assert_eq!(parse_weight(""), Some(1.0));
        assert_eq!(parse_weight("1/0"), None);
        assert_eq!(parse_weight("abc"), None);
        let test = |name: &str, weight: &str, grade: Option<f32>| Assessment {
            name: name.into(),
            weight: weight.into(),
            grade,
        };
        let mut stats = Course::new("Statistik", &[]);
        stats.ects = Some(6.0);
        stats.assessments = vec![test("Zwischenprüfung", "1/6", Some(4.0)), test("Schlussprüfung", "5/6", None)];
        assert_eq!(stats.final_grade(), Some((4.0, false)), "only one graded so far: provisional");
        stats.assessments[1].grade = Some(5.5);
        let (grade, done) = stats.final_grade().unwrap();
        assert!((grade - (4.0 / 6.0 + 5.5 * 5.0 / 6.0)).abs() < 1e-5 && done);
        let refs = [&stats];
        assert_eq!(grade_summary(&refs, GradeScale::Swiss).earned, 6.0);
        // Provisional grades don't count as passed yet.
        stats.assessments[1].grade = None;
        assert_eq!(grade_summary(&[&stats], GradeScale::Swiss).earned, 0.0);
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
        let found = scan_study_folder(&make_tree(tmp.path()), false, &[]);
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
    fn flat_study_folders_have_the_modules_directly() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("Uni");
        for dir in
            ["Statistik/Vorlesung", "Statistik/Übungen", "Programmieren", "Informatik/Datenbanken", "Archiv/HS2025"]
        {
            fs::create_dir_all(root.join(dir)).unwrap();
        }
        fs::write(root.join("Programmieren/vl1.pdf"), "x").unwrap();
        // Informatik is a program whose modules weren't moved up yet.
        let found = scan_study_folder(&root, true, &["Informatik".to_string()]);
        let labels: Vec<String> = found.iter().map(|c| c.label()).collect();
        assert_eq!(labels, ["Informatik › Datenbanken", "Programmieren", "Statistik"]);
        assert!(looks_flat(&root, &[]), "modules hold files or their usual subfolders");
        assert!(!looks_flat(&make_tree(tmp.path()), &[]), "programs with modules in them");
    }

    #[test]
    fn scan_missing_folder() {
        assert!(scan_study_folder(Path::new("/definitely/not/here"), false, &[]).is_empty());
    }

    #[test]
    fn merge_keeps_existing() {
        let tmp = tempfile::tempdir().unwrap();
        let studium = make_tree(tmp.path());
        let mut existing = vec![Course::new("Programmierung 1", &["prog1", "java"]).with_subject("Informatik")];
        assert_eq!(merge_courses(&mut existing, scan_study_folder(&studium, false, &[])), 3);
        assert_eq!(existing.len(), 4);
        assert_eq!(existing[0].keywords, vec!["prog1", "java"]);
        assert_eq!(merge_courses(&mut existing, scan_study_folder(&studium, false, &[])), 0);
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
