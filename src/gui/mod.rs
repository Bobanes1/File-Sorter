//! The window (egui on wgpu), in the style of a Google app: an overview with folders,
//! modules, keywords and the activity list, and a settings page.
//!
//! Everything that must keep going while the window is minimised or in the notification
//! area (watch results, undo, the tray icon, closing) lives in `App::logic`, which eframe
//! keeps calling then; `App::ui` only draws.

mod cards;
mod dialogs;
mod overview;
mod settings;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use eframe::egui::{self, Id, Margin, Rect, RichText, Sense, vec2};
use egui_phosphor::regular as icon;

pub(crate) use cards::{Card, CardKind, FileState, RowAction, activity_row};
use cards::{Note, SNACKBAR_SECS, Snackbar};

use crate::config::{
    Config, Course, DEFAULT_EXTENSIONS, EXTRA_EXTENSIONS, expand_user, load_config, merge_courses, save_config,
    scan_study_folder,
};
use crate::fx;
use crate::i18n::Lang;
use crate::keywords::{Suggestion, suggest_keywords};
use crate::sorter::{LeftReason, Outcome};
use crate::tray::{self, Tray};
use crate::undo::{UndoError, undo};
use crate::watcher::{FolderWatcher, Sink, StartError, WatchEvent};
use crate::winwin::{MainWindow, SingleInstance};

/// How long a file card flies from the Downloads field to its module.
const FLIGHT_SECS: f64 = 1.1;
/// How long a module row glows after a file arrived.
const GLOW_SECS: f64 = 1.8;
const MAX_CARDS: usize = 200;
/// Entries from earlier sessions shown at start.
const HISTORY_CARDS: usize = 60;

/// Open the window. `autostart`: opened at sign-in, so start minimised (or in the
/// notification area) and watching.
pub fn run(config_path: PathBuf, autostart: bool, instance: SingleInstance) -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("File Sorter")
            .with_icon(Arc::new(app_icon()))
            .with_inner_size([980.0, 900.0])
            .with_min_inner_size([760.0, 640.0]),
        ..Default::default()
    };
    eframe::run_native(
        "File Sorter",
        options,
        Box::new(move |cc| Ok(Box::new(App::new(cc, config_path, autostart, instance)))),
    )
}

/// The window and taskbar icon (the exe itself gets the same icon from build.rs).
pub fn app_icon() -> egui::IconData {
    egui::IconData { rgba: include_bytes!("../../assets/icon-64.rgba").to_vec(), width: 64, height: 64 }
}

/// A file card flying from the Downloads field to its module's row.
struct Flight {
    name: String,
    target: String,
    start: f64,
}

/// The Add/Edit module dialog.
struct CourseDialog {
    /// Index of the module being edited, or None when adding.
    editing: Option<usize>,
    subject: String,
    name: String,
    /// The module field gets keyboard focus once, when the dialog opens.
    focused: bool,
}

/// Suggested keywords waiting for the user to pick.
struct KeywordPreview {
    course: Course,
    suggestion: Suggestion,
    chosen: Vec<bool>,
}

/// Counters for the statistics line (reset every day).
struct Stats {
    day: chrono::NaiveDate,
    sorted: u32,
    unmatched: u32,
}

enum ErrorBox {
    FolderNotFound(String),
    StudyFolderMissing(String),
    NoModulesFound,
    NoModules,
    CouldNotStart(StartError),
    CouldNotSave(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tab {
    Overview,
    Settings,
}

pub(crate) struct App {
    config_path: PathBuf,
    config: Config,
    lang: Lang,
    /// Settings the running watcher currently uses.
    pushed: Option<Config>,
    watcher: Option<FolderWatcher>,
    events_tx: Sender<WatchEvent>,
    events_rx: Receiver<WatchEvent>,
    ctx: egui::Context,
    selected: Option<usize>,
    keywords_text: String,
    dialog: Option<CourseDialog>,
    confirm_delete: Option<usize>,
    error: Option<ErrorBox>,
    /// A running keyword scan: the module it is for, and where its result arrives.
    keyword_scan: Option<(Course, Receiver<Suggestion>)>,
    keyword_preview: Option<KeywordPreview>,

    cards: Vec<Card>,
    /// Finished undos: the entry's key and where the file is now.
    undo_tx: Sender<(u64, Result<PathBuf, UndoError>)>,
    undo_rx: Receiver<(u64, Result<PathBuf, UndoError>)>,
    snackbar: Option<Snackbar>,
    flights: Vec<Flight>,
    glows: HashMap<String, f64>,
    /// Files moved into each module during this session.
    module_counts: HashMap<String, u32>,
    stats: Stats,
    stat_bumped: [f64; 3],
    stat_last: [u32; 3],
    // Where things were drawn in the last frame, for the flying files.
    watch_field_rect: Option<Rect>,
    row_rects: HashMap<String, Rect>,
    list_clip: Option<Rect>,

    tab: Tab,
    /// When the current tab was opened (it fades in).
    tab_opened: f64,
    /// The "More:" field for file types that have no chip.
    more_types: String,
    /// A settings change not saved yet (saved a moment later), and when the last save happened.
    unsaved_since: Option<f64>,
    saved_at: f64,
    /// What is in config.json, to save module and keyword changes automatically.
    saved_config: Config,

    /// The window, once eframe has created it, also for the thread that wakes it.
    window: Option<MainWindow>,
    shared_window: Arc<Mutex<Option<MainWindow>>>,
    /// Opened at sign-in: minimise (or go to the notification area) once shown.
    start_hidden: bool,
    tray: Option<Tray>,
    /// Language and tooltip the tray icon was made with.
    tray_state: Option<(Lang, String)>,
    /// "Quit" from the tray: really close instead of going to the background.
    quit: Arc<AtomicBool>,
}

fn known_types() -> impl Iterator<Item = &'static str> {
    DEFAULT_EXTENSIONS.iter().chain(EXTRA_EXTENSIONS).copied()
}

fn today() -> chrono::NaiveDate {
    chrono::Local::now().date_naive()
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>, config_path: PathBuf, autostart: bool, instance: SingleInstance) -> Self {
        fx::install(&cc.egui_ctx);
        let (events_tx, events_rx) = channel();
        let (undo_tx, undo_rx) = channel();
        let (config, unreadable) = match load_config(&config_path) {
            Ok(config) => (config, None),
            Err(e) => {
                let config =
                    Config { state_dir: Some(crate::config::config_path_dir(&config_path)), ..Config::default() };
                (config, Some(e.to_string()))
            }
        };
        let lang = Lang::from_setting(config.language.as_deref());
        let shared_window = Arc::new(Mutex::new(None));
        let mut app = App {
            config_path,
            config,
            lang,
            pushed: None,
            watcher: None,
            events_tx,
            events_rx,
            ctx: cc.egui_ctx.clone(),
            selected: None,
            keywords_text: String::new(),
            dialog: None,
            confirm_delete: None,
            error: None,
            keyword_scan: None,
            keyword_preview: None,
            cards: Vec::new(),
            undo_tx,
            undo_rx,
            snackbar: None,
            flights: Vec::new(),
            glows: HashMap::new(),
            module_counts: HashMap::new(),
            stats: Stats { day: today(), sorted: 0, unmatched: 0 },
            stat_bumped: [f64::NEG_INFINITY; 3],
            stat_last: [0; 3],
            watch_field_rect: None,
            row_rects: HashMap::new(),
            list_clip: None,
            tab: Tab::Overview,
            tab_opened: f64::NEG_INFINITY,
            more_types: String::new(),
            unsaved_since: None,
            saved_at: f64::NEG_INFINITY,
            saved_config: Config::default(),
            window: None,
            shared_window: Arc::clone(&shared_window),
            start_hidden: autostart,
            tray: None,
            tray_state: None,
            quit: Arc::new(AtomicBool::new(false)),
        };
        app.saved_config = app.config.clone();
        app.load_history();
        if let Some(error) = unreadable {
            let path = app.config_path.display().to_string();
            app.note(Note::ConfigUnreadable { path, error });
        }
        app.more_types = app
            .config
            .extensions
            .iter()
            .filter(|e| !known_types().any(|k| k == *e))
            .cloned()
            .collect::<Vec<_>>()
            .join(", ");
        fx::apply_theme(&app.ctx, app.config.theme);
        fx::set_motion(&app.ctx, app.config.animations);
        app.sort_courses();
        // Keep the right-click entry and the sign-in entry pointing at this exe (it may have moved).
        app.apply_context_menu();
        crate::desktop::apply_autostart_setting(app.config.autostart);
        // Started File Sorter again (taskbar, Start menu): show this window instead.
        let ctx = app.ctx.clone();
        instance.on_wake(move || {
            if let Some(window) = *shared_window.lock().unwrap() {
                window.bring_back();
            }
            ctx.request_repaint();
        });
        // Started here rather than on the first frame, which a minimised window may not get soon.
        if autostart || app.config.watch_on_open {
            app.toggle_watching();
        }
        app
    }

    // ---------- helpers ----------

    fn now(&self) -> f64 {
        self.ctx.input(|i| i.time)
    }

    fn sink(&self) -> Sink {
        let tx = self.events_tx.clone();
        let ctx = self.ctx.clone();
        Arc::new(move |event| {
            let _ = tx.send(event);
            ctx.request_repaint(); // wake the UI: results arrive from background threads
        })
    }

    fn push_card(&mut self, kind: CardKind) -> u64 {
        let card = Card::new(kind, self.now());
        let key = card.key;
        self.cards.push(card);
        if self.cards.len() > MAX_CARDS {
            self.cards.remove(0);
        }
        key
    }

    fn note(&mut self, note: Note) {
        log::info!("{}", cards::note_title(Lang::En, &note));
        self.push_card(CardKind::Note(note));
    }

    /// The previous sessions' sorted files, so they can still be undone.
    fn load_history(&mut self) {
        let Some(dir) = self.config.state_dir.clone() else { return };
        for entry in crate::history::recent(&dir, HISTORY_CARDS) {
            let name = entry.result.file_name();
            let state = if entry.undone {
                FileState::Undone { back: entry.result.source }
            } else {
                FileState::Done(entry.result)
            };
            self.cards.push(Card::past(CardKind::File { name, state }, entry.time));
        }
    }

    /// The newest entry for `name` that is still in progress (waiting, or retrying a locked file).
    fn open_file_card(&mut self, name: &str) -> Option<&mut Card> {
        self.cards.iter_mut().rev().find(|c| match &c.kind {
            CardKind::File { name: n, state: FileState::Waiting } => n == name,
            CardKind::File { name: n, state: FileState::Done(r) } => {
                n == name && matches!(r.outcome, Outcome::Left(LeftReason::InUse { .. }))
            }
            _ => false,
        })
    }

    fn handle_event(&mut self, event: WatchEvent) {
        let now = self.now();
        match event {
            WatchEvent::Detected(name) => {
                if let Some(card) = self.open_file_card(&name) {
                    card.set_state(FileState::Waiting, now);
                } else {
                    self.push_card(CardKind::File { name, state: FileState::Waiting });
                }
            }
            WatchEvent::Vanished(name) => {
                if let Some(card) = self.open_file_card(&name) {
                    card.set_state(FileState::Vanished, now);
                }
            }
            WatchEvent::Result(result) => {
                self.roll_day();
                let name = result.file_name();
                // Put back by "undo": no entry for it.
                if result.outcome == Outcome::Left(LeftReason::Kept) {
                    let key = self.open_file_card(&name).map(|c| c.key);
                    self.cards.retain(|c| Some(c.key) != key);
                    return;
                }
                let course = match &result.outcome {
                    Outcome::Moved { course, .. }
                    | Outcome::Copied { course, .. }
                    | Outcome::DuplicateRemoved { course, .. } => Some(course.clone()),
                    Outcome::Left(LeftReason::NoMatch) => {
                        self.stats.unmatched += 1;
                        None
                    }
                    Outcome::Left(_) => None,
                };
                let snack = course.as_ref().map(|course| match &result.outcome {
                    Outcome::DuplicateRemoved { .. } => self.lang.snack_duplicate(&name),
                    Outcome::Copied { .. } => self.lang.snack_sorted(&name, course, true),
                    _ => self.lang.snack_sorted(&name, course, false),
                });
                let state = FileState::Done(result);
                let key = if let Some(card) = self.open_file_card(&name) {
                    card.set_state(state, now);
                    card.key
                } else {
                    self.push_card(CardKind::File { name: name.clone(), state })
                };
                if let (Some(course), Some(text)) = (course, snack) {
                    self.stats.sorted += 1;
                    self.flights.push(Flight { name, target: course, start: now });
                    self.snackbar = Some(Snackbar { card: key, text, can_undo: true, shown: now });
                }
            }
        }
    }

    /// Put an entry's file back, on a background thread (it may wait for a program to let go).
    fn start_undo(&mut self, key: u64) {
        let now = self.now();
        let Some(card) = self.cards.iter_mut().find(|c| c.key == key) else { return };
        let Some(result) = card.undoable().cloned() else { return };
        card.set_state(FileState::Undoing(result.clone()), now);
        let (tx, config, ctx) = (self.undo_tx.clone(), self.config.clone(), self.ctx.clone());
        std::thread::spawn(move || {
            let _ = tx.send((key, undo(&result, &config)));
            ctx.request_repaint();
        });
        if self.snackbar.as_ref().is_some_and(|s| s.card == key) {
            self.snackbar = None;
        }
    }

    fn finish_undo(&mut self, key: u64, outcome: Result<PathBuf, UndoError>) {
        let now = self.now();
        let lang = self.lang;
        let Some(card) = self.cards.iter_mut().find(|c| c.key == key) else { return };
        let CardKind::File { name, state: FileState::Undoing(result) } = &card.kind else { return };
        let (name, result) = (name.clone(), result.clone());
        match outcome {
            Ok(back) => {
                card.set_state(FileState::Undone { back }, now);
                if result.sorted() {
                    self.stats.sorted = self.stats.sorted.saturating_sub(1);
                }
                if let Outcome::Moved { course, .. }
                | Outcome::Copied { course, .. }
                | Outcome::DuplicateRemoved { course, .. } = &result.outcome
                    && let Some(count) = self.module_counts.get_mut(course)
                {
                    *count = count.saturating_sub(1);
                }
                self.snackbar =
                    Some(Snackbar { card: key, text: lang.snack_undone(&name), can_undo: false, shown: now });
            }
            Err(error) => card.set_state(FileState::UndoFailed { result, error }, now),
        }
    }

    fn roll_day(&mut self) {
        if self.stats.day != today() {
            self.stats = Stats { day: today(), sorted: 0, unmatched: 0 };
        }
    }

    fn waiting_count(&self) -> u32 {
        self.cards.iter().filter(|c| matches!(c.kind, CardKind::File { state: FileState::Waiting, .. })).count() as u32
    }

    fn running(&self) -> bool {
        self.watcher.as_ref().is_some_and(FolderWatcher::running)
    }

    fn sort_courses(&mut self) {
        let selected = self.selected.map(|i| self.config.courses[i].clone());
        self.config.courses.sort_by_key(|c| (c.subject.to_lowercase(), c.name.to_lowercase()));
        self.select(selected.and_then(|c| self.config.courses.iter().position(|x| *x == c)));
    }

    fn select(&mut self, index: Option<usize>) {
        self.selected = index.filter(|&i| i < self.config.courses.len());
        self.keywords_text = self.selected.map(|i| self.config.courses[i].keywords.join("\n")).unwrap_or_default();
    }

    fn study_root(&self) -> PathBuf {
        expand_user(self.config.destination_root.trim())
    }

    fn subjects(&self) -> Vec<String> {
        let mut subjects: Vec<String> =
            self.config.courses.iter().map(|c| c.subject.clone()).filter(|s| !s.is_empty()).collect();
        if let Ok(entries) = std::fs::read_dir(self.study_root()) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                if entry.path().is_dir() && !name.starts_with('.') {
                    subjects.push(name);
                }
            }
        }
        subjects.sort_by_key(|s| s.to_lowercase());
        subjects.dedup();
        subjects
    }

    /// Save the settings (modules and keywords too).
    fn save_quietly(&mut self) -> bool {
        match save_config(&self.config, &self.config_path) {
            Ok(()) => {
                self.saved_config = self.config.clone();
                true
            }
            Err(e) => {
                self.error = Some(ErrorBox::CouldNotSave(e.to_string()));
                false
            }
        }
    }

    /// `None` follows the system language.
    fn set_language(&mut self, choice: Option<Lang>) {
        self.config.language = choice.map(|lang| lang.code().to_string());
        self.lang = Lang::from_setting(self.config.language.as_deref());
        self.apply_context_menu();
    }

    /// A setting changed: save it a moment later (typing in a field changes it on every key).
    fn settings_changed(&mut self) {
        self.unsaved_since = Some(self.now());
    }

    fn save_pending_settings(&mut self) {
        if let Some(t) = self.unsaved_since
            && self.now() - t > 0.4
        {
            self.unsaved_since = None;
            if self.save_quietly() {
                self.saved_at = self.now();
            }
        }
    }

    /// Add, refresh (exe moved, language changed) or remove the right-click menu entry.
    fn apply_context_menu(&self) {
        crate::desktop::apply_menu_setting(self.config.context_menu, self.lang.context_menu_label());
    }

    fn import_modules(&mut self, quiet: bool) {
        let root = self.study_root();
        if !root.is_dir() {
            if !quiet {
                self.error = Some(ErrorBox::StudyFolderMissing(root.display().to_string()));
            }
            return;
        }
        let found = scan_study_folder(&root);
        if found.is_empty() {
            if !quiet {
                self.error = Some(ErrorBox::NoModulesFound);
            }
            return;
        }
        let total = found.len();
        let added = merge_courses(&mut self.config.courses, found);
        self.sort_courses();
        self.note(Note::Imported { added, known: total - added });
    }

    fn check_ready(&mut self) -> bool {
        if self.config.courses.is_empty() {
            self.error = Some(ErrorBox::NoModules);
            return false;
        }
        let watch = self.config.watch_path();
        if !watch.is_dir() {
            self.error = Some(ErrorBox::FolderNotFound(watch.display().to_string()));
            return false;
        }
        true
    }

    fn toggle_watching(&mut self) {
        if self.running() {
            if let Some(mut w) = self.watcher.take() {
                w.stop();
            }
            self.pushed = None;
            // Files still "waiting" won't be finished by this watcher any more.
            self.cards.retain(|c| !matches!(c.kind, CardKind::File { state: FileState::Waiting, .. }));
            self.note(Note::Stopped);
            return;
        }
        if !self.check_ready() || !self.save_quietly() {
            return;
        }
        let lock = crate::watch_lock_path(&self.config_path);
        let mut watcher = FolderWatcher::new(self.config.clone(), self.sink()).with_lock(lock);
        match watcher.start() {
            Ok(()) => {
                self.pushed = Some(self.config.clone());
                self.watcher = Some(watcher);
                self.note(Note::Started(self.config.watch_path().display().to_string()));
            }
            Err(e) => self.error = Some(ErrorBox::CouldNotStart(e)),
        }
    }

    fn sort_now(&mut self) {
        if !self.check_ready() {
            return;
        }
        match &self.watcher {
            Some(w) if w.running() => w.sort_existing_async(),
            _ => FolderWatcher::new(self.config.clone(), self.sink()).sort_existing_async(),
        }
        self.note(Note::SortingExisting);
    }

    fn pick_folder(current: &str) -> Option<String> {
        let start = expand_user(current.trim());
        let mut dialog = rfd::FileDialog::new();
        if start.is_dir() {
            dialog = dialog.set_directory(&start);
        }
        dialog.pick_folder().map(|p| p.to_string_lossy().into_owned())
    }

    fn start_keyword_scan(&mut self) {
        let Some(i) = self.selected else { return };
        let (tx, rx) = channel();
        let config = self.config.clone();
        let ctx = self.ctx.clone();
        // Reading PDFs can take a few seconds; keep the window responsive.
        std::thread::spawn(move || {
            let _ = tx.send(suggest_keywords(&config, i));
            ctx.request_repaint();
        });
        self.keyword_scan = Some((self.config.courses[i].clone(), rx));
    }

    /// The scan is done: show what it found, or say why there is nothing.
    fn finish_keyword_scan(&mut self, course: Course, suggestion: Suggestion) {
        let module = course.label();
        if suggestion.files_scanned == 0 {
            self.note(Note::NoFilesToScan { module, folder: suggestion.folder.display().to_string() });
        } else if suggestion.keywords.is_empty() {
            self.note(Note::NoNewKeywords { module, files: suggestion.files_scanned });
        } else {
            let chosen = vec![true; suggestion.keywords.len()];
            self.keyword_preview = Some(KeywordPreview { course, suggestion, chosen });
        }
    }

    /// Add the picked keywords to the module they were found for (which may have moved in the list).
    fn add_keywords(&mut self, course: &Course, words: Vec<String>) {
        let Some(i) = self.config.courses.iter().position(|c| c.subject == course.subject && c.name == course.name)
        else {
            return;
        };
        if words.is_empty() {
            return;
        }
        let module = self.config.courses[i].label();
        self.config.courses[i].keywords.extend(words.iter().cloned());
        if self.selected == Some(i) {
            self.keywords_text = self.config.courses[i].keywords.join("\n");
        }
        self.glows.insert(module.clone(), self.now());
        self.note(Note::KeywordsAdded { module, words });
        // Keep the new keywords even if the program is closed abruptly.
        self.save_quietly();
    }

    fn open_edit(&mut self, i: usize) {
        let c = &self.config.courses[i];
        self.dialog =
            Some(CourseDialog { editing: Some(i), subject: c.subject.clone(), name: c.name.clone(), focused: false });
    }

    fn switch_tab(&mut self, tab: Tab) {
        if self.tab != tab {
            self.tab = tab;
            self.tab_opened = self.now();
        }
    }

    /// The chosen chips plus whatever is typed into the "More:" field.
    fn apply_more_types(&mut self) {
        let mut types: Vec<String> =
            self.config.extensions.iter().filter(|e| known_types().any(|k| k == *e)).cloned().collect();
        for part in self.more_types.split([',', ';', ' ']) {
            let ext = crate::config::normalise_ext(part);
            let valid = ext.len() > 1 && !ext[1..].contains(['.', '/', '\\', '*']);
            if valid && !types.contains(&ext) {
                types.push(ext);
            }
        }
        self.config.extensions = types;
    }

    /// Keep watching in the notification area when the window is closed.
    fn background_active(&self) -> bool {
        self.config.background && tray::supported()
    }

    fn in_tray(&self) -> bool {
        self.window.is_some_and(MainWindow::in_tray)
    }

    // ---------- window, tray and closing ----------

    fn window_logic(&mut self, ctx: &egui::Context, frame: &eframe::Frame) {
        if self.window.is_none()
            && let Some(window) = MainWindow::of(frame)
        {
            self.window = Some(window);
            *self.shared_window.lock().unwrap() = Some(window);
        }
        // Opened at sign-in: once eframe has shown the window (after the first frame), tuck it away.
        if self.start_hidden
            && ctx.cumulative_frame_nr() >= 1
            && let Some(window) = self.window
        {
            self.start_hidden = false;
            if self.background_active() {
                window.to_tray();
            } else {
                window.minimise();
            }
        }

        // The tray icon exists while "keep running in the background" is on.
        if self.background_active() {
            let folder = self.config.watch_path();
            let folder = folder.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let tooltip = self.lang.tray_tooltip(self.running().then_some(folder.as_str()));
            let lang = self.lang;
            match (&self.tray, &self.tray_state, self.window) {
                (Some(tray), Some((made_with, shown)), _) if *made_with == lang => {
                    if *shown != tooltip {
                        tray.set_tooltip(&tooltip);
                        self.tray_state = Some((lang, tooltip));
                    }
                }
                (_, _, Some(window)) => {
                    self.tray = None; // recreate, e.g. in the new language
                    self.tray = tray::create(lang, &tooltip, window, ctx.clone(), Arc::clone(&self.quit));
                    self.tray_state = Some((lang, tooltip));
                }
                _ => {}
            }
        } else if self.tray.is_some() {
            self.tray = None;
            self.tray_state = None;
        }

        // Closing while watching in background mode: go to the notification area instead.
        if ctx.input(|i| i.viewport().close_requested())
            && !self.quit.load(Ordering::SeqCst)
            && self.background_active()
            && self.running()
            && self.tray.is_some()
            && let Some(window) = self.window
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            window.to_tray();
        }
    }

    /// Keep animating while something moves; otherwise only repaint when something happens.
    fn schedule_repaint(&self) {
        if self.in_tray() {
            return;
        }
        let now = self.now();
        let animating = self.start_hidden
            || !self.flights.is_empty()
            || self.glows.values().any(|t| now - t < GLOW_SECS)
            || now - self.tab_opened < 0.3
            || now - self.saved_at < 1.9
            || self.snackbar.as_ref().is_some_and(|s| now - s.shown < SNACKBAR_SECS + 0.4)
            || self.cards.iter().rev().take(20).any(|c| {
                now - c.born < cards::CARD_APPEAR_SECS
                    || now - c.changed < cards::CARD_FLASH_SECS
                    || matches!(c.kind, CardKind::File { state: FileState::Waiting | FileState::Undoing(_), .. })
            });
        if animating {
            self.ctx.request_repaint();
        } else if self.unsaved_since.is_some() || self.keyword_scan.is_some() {
            self.ctx.request_repaint_after(Duration::from_millis(150));
        } else if self.running() && fx::motion(&self.ctx) {
            // The pulsing dot on the stop button; 25 frames per second are plenty for it.
            self.ctx.request_repaint_after(Duration::from_millis(40));
        }
    }

    // ---------- top bar ----------

    fn top_bar(&mut self, ui: &mut egui::Ui) {
        let p = fx::pal(ui);
        ui.horizontal(|ui| {
            ui.set_min_height(48.0);
            let (logo, _) = ui.allocate_exact_size(vec2(36.0, 36.0), Sense::hover());
            fx::avatar(
                ui.painter(),
                logo.center(),
                18.0,
                icon::GRADUATION_CAP,
                p.primary_container,
                p.on_primary_container,
            );
            ui.add_space(4.0);
            ui.label(fx::title("File Sorter", 20.0).color(p.on_surface));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| self.tabs_ui(ui));
        });
    }

    /// "Overview | Settings" with an indicator that slides to the open tab.
    fn tabs_ui(&mut self, ui: &mut egui::Ui) {
        let p = fx::pal(ui);
        let lang = self.lang;
        let mut open_rect = None;
        // Right to left: Settings is drawn first.
        for (tab, glyph, text) in [
            (Tab::Settings, icon::GEAR_SIX, lang.tab_settings()),
            (Tab::Overview, icon::SQUARES_FOUR, lang.tab_overview()),
        ] {
            let open = self.tab == tab;
            let color = if open { p.primary } else { p.on_surface_variant };
            let galley = ui.painter().layout_no_wrap(format!("{glyph}  {text}"), fx::medium_font(14.0), color);
            let (rect, response) = ui.allocate_exact_size(vec2(galley.size().x + 32.0, 48.0), Sense::click());
            fx::state_layer(ui, &response, rect, 8, p.on_surface);
            fx::ripple(ui, &response, rect, p.primary);
            ui.painter().galley(rect.center() - galley.size() / 2.0, galley, color);
            if response.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                self.switch_tab(tab);
            }
            if open {
                open_rect = Some(rect);
            }
        }
        if let Some(target) = open_rect {
            let secs = fx::anim_time(ui.ctx(), 0.25);
            let left = ui.ctx().animate_value_with_time(Id::new("tab-left"), target.left() + 12.0, secs);
            let width = ui.ctx().animate_value_with_time(Id::new("tab-width"), target.width() - 24.0, secs);
            let bar = Rect::from_min_size(egui::pos2(left, target.bottom() - 3.0), vec2(width, 3.0));
            ui.painter().rect_filled(bar, egui::CornerRadius { nw: 3, ne: 3, sw: 0, se: 0 }, p.primary);
        }
    }
}

impl eframe::App for App {
    fn logic(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        self.window_logic(ctx, frame);
        while let Ok(event) = self.events_rx.try_recv() {
            self.handle_event(event);
        }
        while let Ok((key, outcome)) = self.undo_rx.try_recv() {
            self.finish_undo(key, outcome);
        }
        if let Some((course, rx)) = &self.keyword_scan {
            match rx.try_recv() {
                Ok(suggestion) => {
                    let course = course.clone();
                    self.keyword_scan = None;
                    self.finish_keyword_scan(course, suggestion);
                }
                Err(TryRecvError::Disconnected) => {
                    self.keyword_scan = None;
                    self.note(Note::ScanFailed);
                }
                Err(TryRecvError::Empty) => {}
            }
        }
        let now = self.now();
        self.glows.retain(|_, t| now - *t < GLOW_SECS);
        // Keyword or folder edits take effect immediately while watching.
        if let Some(w) = &self.watcher
            && self.pushed.as_ref() != Some(&self.config)
        {
            w.update_config(self.config.clone());
            self.pushed = Some(self.config.clone());
        }
        // Module and keyword edits are saved a moment later, like settings.
        if self.config != self.saved_config && self.unsaved_since.is_none() {
            self.unsaved_since = Some(now);
        }
        self.save_pending_settings();
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let p = fx::pal(ui);
        let now = self.now();
        egui::Panel::top("top-bar")
            .frame(egui::Frame::new().fill(p.surface).inner_margin(Margin { left: 20, right: 12, top: 8, bottom: 0 }))
            .show_separator_line(false)
            .show(ui, |ui| self.top_bar(ui));
        // The opened tab fades in.
        let fade = if fx::motion(ui.ctx()) { fx::ease_out_cubic(((now - self.tab_opened) / 0.25) as f32) } else { 1.0 };
        match self.tab {
            Tab::Overview => self.overview_ui(ui, fade),
            Tab::Settings => {
                egui::CentralPanel::default()
                    .frame(egui::Frame::new().fill(p.surface).inner_margin(Margin::symmetric(20, 12)))
                    .show(ui, |ui| {
                        ui.set_opacity(fade);
                        egui::ScrollArea::vertical().id_salt("settings").auto_shrink([false, false]).show(ui, |ui| {
                            self.settings_ui(ui);
                            ui.add_space(24.0);
                        });
                    });
            }
        }
        let ctx = ui.ctx().clone();
        self.flights_ui(&ctx);
        self.dialogs_ui(&ctx);
        if let Some(bar) = &mut self.snackbar {
            match cards::snackbar_ui(&ctx, self.lang, bar, now) {
                None => self.snackbar = None,
                Some(true) => {
                    let key = bar.card;
                    self.start_undo(key);
                }
                Some(false) => {}
            }
        }
        self.schedule_repaint();
    }

    fn on_exit(&mut self) {
        if let Some(mut w) = self.watcher.take() {
            w.stop();
        }
        if let Err(e) = save_config(&self.config, &self.config_path) {
            log::error!("Could not save settings: {e}");
        }
    }
}

/// A section title in the primary colour.
fn section_title(ui: &mut egui::Ui, text: &str) {
    let p = fx::pal(ui);
    ui.add_space(4.0);
    ui.label(RichText::new(text).family(fx::medium()).size(14.0).color(p.primary));
    ui.add_space(2.0);
}
