//! The window: folders, modules with keywords, start/stop and an animated activity feed (egui on wgpu).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};
use std::time::Duration;

use eframe::egui::{
    self, Align, Align2, Color32, CornerRadius, FontId, Id, LayerId, Layout, Margin, Order, Pos2, Rect, RichText,
    Sense, Stroke, vec2,
};
use egui_phosphor::regular as icon;

use crate::config::{Config, Course, expand_user, load_config, merge_courses, save_config, scan_study_folder};
use crate::fx;
use crate::i18n::Lang;
use crate::keywords::{Suggestion, suggest_keywords};
use crate::matcher::normalise;
use crate::sorter::{LeftReason, Outcome};
use crate::watcher::{FolderWatcher, Sink, StartError, WatchEvent};

/// How long a file card flies from the Downloads field to its module.
const FLIGHT_SECS: f64 = 1.1;
/// How long a module row glows after a file arrived.
const GLOW_SECS: f64 = 1.8;
const CARD_APPEAR_SECS: f64 = 0.35;
const CARD_FLASH_SECS: f64 = 0.9;
const MAX_CARDS: usize = 200;

pub fn run(config_path: PathBuf) -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("File Sorter")
            .with_inner_size([940.0, 900.0])
            .with_min_inner_size([720.0, 640.0]),
        ..Default::default()
    };
    eframe::run_native("File Sorter", options, Box::new(move |cc| Ok(Box::new(App::new(cc, config_path)))))
}

/// Things worth telling the user that aren't about a single file.
#[derive(Debug, Clone)]
enum Note {
    Started(String),
    Stopped,
    Saved,
    SortingExisting,
    Imported { added: usize, known: usize },
    KeywordsAdded { module: String, words: Vec<String> },
    NoNewKeywords { module: String, files: usize },
    NoFilesToScan { module: String, folder: String },
    ScanFailed,
    ConfigUnreadable { path: String, error: String },
}

#[derive(Debug, Clone)]
enum FileState {
    Waiting,
    Done(Outcome),
    Vanished,
}

#[derive(Debug, Clone)]
enum CardKind {
    File { name: String, state: FileState },
    Note(Note),
}

/// One entry in the activity feed. File cards change state in place (waiting → moved).
struct Card {
    kind: CardKind,
    born: f64,
    changed: f64,
    clock: String,
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

/// Counters for the statistics tiles (reset every day).
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

struct App {
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

    cards: Vec<Card>,
    flights: Vec<Flight>,
    glows: HashMap<String, f64>,
    /// Files moved into each module during this session.
    module_counts: HashMap<String, u32>,
    stats: Stats,
    /// When each statistic last went up (for a short highlight).
    stat_bumped: [f64; 4],
    stat_last: [u32; 4],
    // Where things were drawn in the last frame, for the flying files.
    watch_field_rect: Option<Rect>,
    row_rects: HashMap<String, Rect>,
    list_clip: Option<Rect>,
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>, config_path: PathBuf) -> Self {
        fx::install(&cc.egui_ctx);
        let (events_tx, events_rx) = channel();
        let (config, unreadable) = match load_config(&config_path) {
            Ok(config) => (config, None),
            Err(e) => (Config::default(), Some(e.to_string())),
        };
        let lang = Lang::from_setting(config.language.as_deref());
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
            cards: Vec::new(),
            flights: Vec::new(),
            glows: HashMap::new(),
            module_counts: HashMap::new(),
            stats: Stats { day: today(), sorted: 0, unmatched: 0 },
            stat_bumped: [f64::NEG_INFINITY; 4],
            stat_last: [0; 4],
            watch_field_rect: None,
            row_rects: HashMap::new(),
            list_clip: None,
        };
        if let Some(error) = unreadable {
            let path = app.config_path.display().to_string();
            app.note(Note::ConfigUnreadable { path, error });
        }
        app.sort_courses();
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

    fn push_card(&mut self, kind: CardKind) {
        let now = self.now();
        let clock = chrono::Local::now().format("%H:%M").to_string();
        self.cards.push(Card { kind, born: now, changed: now, clock });
        if self.cards.len() > MAX_CARDS {
            self.cards.remove(0);
        }
    }

    fn note(&mut self, note: Note) {
        log::info!("{}", note_title(Lang::En, &note));
        self.push_card(CardKind::Note(note));
    }

    /// The newest card for `name` that is still in progress (waiting, or retrying a locked file).
    fn open_file_card(&mut self, name: &str) -> Option<&mut Card> {
        self.cards.iter_mut().rev().find(|c| {
            matches!(&c.kind, CardKind::File { name: n, state: FileState::Waiting | FileState::Done(Outcome::Left(LeftReason::InUse { .. })) } if n == name)
        })
    }

    fn handle_event(&mut self, event: WatchEvent) {
        let now = self.now();
        match event {
            WatchEvent::Detected(name) => {
                if let Some(card) = self.open_file_card(&name) {
                    card.kind = CardKind::File { name, state: FileState::Waiting };
                    card.changed = now;
                } else {
                    self.push_card(CardKind::File { name, state: FileState::Waiting });
                }
            }
            WatchEvent::Vanished(name) => {
                if let Some(card) = self.open_file_card(&name) {
                    card.kind = CardKind::File { name, state: FileState::Vanished };
                    card.changed = now;
                }
            }
            WatchEvent::Result(result) => {
                self.roll_day();
                let name = result.file_name();
                match &result.outcome {
                    Outcome::Moved { course, .. } | Outcome::DuplicateRemoved { course, .. } => {
                        self.stats.sorted += 1;
                        self.flights.push(Flight { name: name.clone(), target: course.clone(), start: now });
                    }
                    Outcome::Left(LeftReason::NoMatch) => self.stats.unmatched += 1,
                    Outcome::Left(_) => {}
                }
                let state = FileState::Done(result.outcome);
                if let Some(card) = self.open_file_card(&name) {
                    card.kind = CardKind::File { name, state };
                    card.changed = now;
                } else {
                    self.push_card(CardKind::File { name, state });
                }
            }
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

    /// Save without a card in the activity feed (for automatic saves).
    fn save_quietly(&mut self) -> bool {
        match save_config(&self.config, &self.config_path) {
            Ok(()) => true,
            Err(e) => {
                self.error = Some(ErrorBox::CouldNotSave(e.to_string()));
                false
            }
        }
    }

    fn save(&mut self) {
        if self.save_quietly() {
            self.note(Note::Saved);
        }
    }

    fn set_language(&mut self, lang: Lang) {
        if self.lang != lang {
            self.lang = lang;
            self.config.language = Some(lang.code().to_string());
            self.save_quietly();
        }
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

    /// Add the scan result to the module it was started for (which may have moved in the list meanwhile).
    fn finish_keyword_scan(&mut self, course: Course, suggestion: Suggestion) {
        let Some(i) = self.config.courses.iter().position(|c| c.subject == course.subject && c.name == course.name)
        else {
            return;
        };
        let module = self.config.courses[i].label();
        if suggestion.files_scanned == 0 {
            self.note(Note::NoFilesToScan { module, folder: suggestion.folder.display().to_string() });
            return;
        }
        let keywords = &mut self.config.courses[i].keywords;
        let known: Vec<String> = keywords.iter().map(|k| normalise(k)).collect();
        let new: Vec<String> = suggestion.keywords.into_iter().filter(|k| !known.contains(k)).collect();
        if new.is_empty() {
            self.note(Note::NoNewKeywords { module, files: suggestion.files_scanned });
            return;
        }
        keywords.extend(new.iter().cloned());
        if self.selected == Some(i) {
            self.keywords_text = self.config.courses[i].keywords.join("\n");
        }
        self.glows.insert(module.clone(), self.now());
        self.note(Note::KeywordsAdded { module, words: new });
        // Keep the new keywords even if the program is closed abruptly.
        self.save_quietly();
    }

    fn open_edit(&mut self, i: usize) {
        let c = &self.config.courses[i];
        self.dialog =
            Some(CourseDialog { editing: Some(i), subject: c.subject.clone(), name: c.name.clone(), focused: false });
    }

    // ---------- header ----------

    fn header_ui(&mut self, ui: &mut egui::Ui) {
        let lang = self.lang;
        let accent = fx::accent(ui);
        let now = self.now();
        ui.horizontal(|ui| {
            ui.label(RichText::new(icon::GRADUATION_CAP).size(28.0).color(accent));
            ui.label(RichText::new("File Sorter").size(22.0).strong());
            ui.add_space(14.0);

            // Status pill with a pulsing radar dot while watching.
            let running = self.running();
            let color = if running { fx::GREEN } else { ui.visuals().weak_text_color() };
            let text = if running {
                let folder = self.config.watch_path();
                let name = folder.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                lang.watching(&name)
            } else {
                lang.stopped().to_string()
            };
            egui::Frame::new()
                .fill(color.gamma_multiply(0.14))
                .corner_radius(CornerRadius::same(14))
                .inner_margin(Margin { left: 10, right: 12, top: 4, bottom: 4 })
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        let (rect, _) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::hover());
                        fx::pulse_dot(ui.painter(), rect.center(), color, running, now);
                        ui.label(RichText::new(text).color(color).strong());
                    });
                });

            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                for choice in [Lang::En, Lang::De] {
                    let label = RichText::new(choice.code().to_uppercase()).strong();
                    if ui.selectable_label(self.lang == choice, label).clicked() {
                        self.set_language(choice);
                    }
                }
                ui.label(RichText::new(icon::GLOBE).size(18.0)).on_hover_text(lang.language());
            });
        });
        ui.add_space(8.0);
        self.stats_ui(ui);
    }

    fn stats_ui(&mut self, ui: &mut egui::Ui) {
        self.roll_day();
        let lang = self.lang;
        let now = self.now();
        let values = [self.stats.sorted, self.waiting_count(), self.stats.unmatched, self.config.courses.len() as u32];
        for (i, value) in values.iter().enumerate() {
            if *value > self.stat_last[i] {
                self.stat_bumped[i] = now;
            }
            self.stat_last[i] = *value;
        }
        let tiles = [
            (icon::CHECK_CIRCLE, fx::GREEN, lang.stat_sorted_today()),
            (icon::HOURGLASS_MEDIUM, fx::BLUE, lang.stat_waiting()),
            (icon::WARNING_CIRCLE, fx::AMBER, lang.stat_unmatched()),
            (icon::BOOKS, fx::accent(ui), lang.stat_modules()),
        ];
        let bumped = self.stat_bumped;
        let ctx = self.ctx.clone();
        ui.columns(4, |cols| {
            for (i, ((glyph, color, label), value)) in tiles.into_iter().zip(values).enumerate() {
                let ui = &mut cols[i];
                let flash = (1.0 - (now - bumped[i]) / 1.2).clamp(0.0, 1.0) as f32;
                let shown = ctx.animate_value_with_time(Id::new(("stat", i)), value as f32, 0.8);
                egui::Frame::new()
                    .fill(fx::lerp_color(ui.visuals().faint_bg_color, color.gamma_multiply(0.25), flash))
                    .stroke(Stroke::new(1.0, color.gamma_multiply(0.25 + 0.6 * flash)))
                    .corner_radius(CornerRadius::same(12))
                    .inner_margin(Margin::symmetric(12, 8))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal(|ui| {
                            if i == 1 && value > 0 {
                                ui.add(egui::Spinner::new().size(24.0).color(color));
                            } else {
                                ui.label(RichText::new(glyph).size(26.0).color(color));
                            }
                            ui.vertical(|ui| {
                                ui.label(
                                    RichText::new(format!("{}", shown.round() as i64)).size(24.0).strong().color(color),
                                );
                                ui.label(RichText::new(label).small().weak());
                            });
                        });
                    });
            }
        });
    }

    // ---------- folders ----------

    fn folders_ui(&mut self, ui: &mut egui::Ui) {
        const LABEL_WIDTH: f32 = 170.0;
        const BUTTON_WIDTH: f32 = 130.0;
        let lang = self.lang;
        ui.set_width(ui.available_width());

        /// Label, stretching text field and Browse… button. Returns the field and the folder picked, if any.
        fn folder_row(ui: &mut egui::Ui, lang: Lang, label: &str, value: &mut String) -> (Rect, Option<String>) {
            ui.horizontal(|ui| {
                ui.allocate_ui_with_layout(vec2(LABEL_WIDTH, 20.0), Layout::left_to_right(Align::Center), |ui| {
                    ui.set_min_width(LABEL_WIDTH);
                    ui.label(label)
                });
                let width = ui.available_width() - BUTTON_WIDTH - ui.spacing().item_spacing.x;
                let field = ui.add(egui::TextEdit::singleline(value).desired_width(width)).rect;
                let browse = egui::Button::new(format!("{} {}", icon::FOLDER_OPEN, lang.browse()));
                let picked =
                    ui.add_sized([BUTTON_WIDTH, 24.0], browse).clicked().then(|| App::pick_folder(value)).flatten();
                (field, picked)
            })
            .inner
        }

        let (field, picked) = folder_row(ui, lang, lang.watch_folder(), &mut self.config.watch_folder);
        self.watch_field_rect = Some(field);
        if let Some(folder) = picked {
            self.config.watch_folder = folder;
        }
        let (_, picked) = folder_row(ui, lang, lang.study_folder(), &mut self.config.destination_root);
        if let Some(folder) = picked {
            self.config.destination_root = folder;
            if self.config.courses.is_empty() {
                self.import_modules(true);
            }
        }
        ui.horizontal(|ui| {
            ui.add_space(LABEL_WIDTH + ui.spacing().item_spacing.x);
            ui.label(RichText::new(lang.layout_hint()).weak());
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.button(format!("{} {}", icon::FOLDERS, lang.import_modules())).clicked() {
                    self.import_modules(false);
                }
            });
        });
        ui.horizontal_wrapped(|ui| {
            ui.checkbox(&mut self.config.read_file_contents, lang.read_contents());
            ui.add_space(16.0);
            ui.checkbox(&mut self.config.sort_existing_on_start, lang.sort_existing_on_start());
        });
    }

    // ---------- modules ----------

    fn modules_ui(&mut self, ui: &mut egui::Ui) {
        let lang = self.lang;
        ui.set_width(ui.available_width());
        let list_height = 170.0;
        ui.horizontal_top(|ui| {
            ui.vertical(|ui| {
                ui.set_width(310.0);
                egui::Frame::group(ui.style()).corner_radius(CornerRadius::same(10)).show(ui, |ui| {
                    egui::ScrollArea::vertical()
                        .id_salt("modules")
                        .max_height(list_height)
                        .auto_shrink([false, false])
                        .show(ui, |ui| self.module_rows(ui));
                });
                ui.horizontal(|ui| {
                    if ui.button(format!("{} {}", icon::PLUS, lang.add())).clicked() {
                        let subject = self.subjects().into_iter().next().unwrap_or_default();
                        self.dialog =
                            Some(CourseDialog { editing: None, subject, name: String::new(), focused: false });
                    }
                    let has_selection = self.selected.is_some();
                    let edit = egui::Button::new(format!("{} {}", icon::PENCIL_SIMPLE, lang.edit()));
                    if ui.add_enabled(has_selection, edit).clicked()
                        && let Some(i) = self.selected
                    {
                        self.open_edit(i);
                    }
                    let delete = egui::Button::new(format!("{} {}", icon::TRASH, lang.delete()));
                    if ui.add_enabled(has_selection, delete).clicked() {
                        self.confirm_delete = self.selected;
                    }
                });
            });

            ui.vertical(|ui| self.keywords_ui(ui));
        });
    }

    /// The module list: full-width rows with hover fade, selection, glow and a counter badge.
    fn module_rows(&mut self, ui: &mut egui::Ui) {
        let lang = self.lang;
        self.list_clip = Some(ui.clip_rect());
        self.row_rects.clear();
        if self.config.courses.is_empty() {
            ui.label(RichText::new(lang.no_modules_yet()).weak());
            return;
        }
        let now = self.now();
        let accent = fx::accent(ui);
        let row_height = 28.0;
        ui.spacing_mut().item_spacing.y = 2.0;
        let mut clicked = None;
        let mut double_clicked = None;
        for (i, course) in self.config.courses.iter().enumerate() {
            let label = course.label();
            let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), row_height), Sense::click());
            self.row_rects.insert(label.clone(), rect);
            if response.clicked() {
                clicked = Some(i);
            }
            if response.double_clicked() {
                double_clicked = Some(i);
            }
            let selected = self.selected == Some(i);
            let hover = self.ctx.animate_bool_with_time(Id::new(("row-hover", &label)), response.hovered(), 0.15);
            let painter = ui.painter();
            let visuals = ui.visuals();
            let row = rect.shrink2(vec2(2.0, 1.0));
            let fill = if selected {
                accent.gamma_multiply(0.85)
            } else {
                visuals.widgets.hovered.weak_bg_fill.gamma_multiply(hover)
            };
            painter.rect_filled(row, CornerRadius::same(7), fill);

            // A file just arrived here: glow, and a "+1" floating up.
            if let Some(&t0) = self.glows.get(&label) {
                let t = ((now - t0) / GLOW_SECS).clamp(0.0, 1.0) as f32;
                let glow = 1.0 - fx::ease_out_cubic(t);
                painter.rect_filled(row, CornerRadius::same(7), accent.gamma_multiply(0.55 * glow));
                painter.rect_stroke(
                    row,
                    CornerRadius::same(7),
                    Stroke::new(1.5, accent.gamma_multiply(glow)),
                    egui::StrokeKind::Inside,
                );
                painter.text(
                    rect.right_center() - vec2(58.0, 2.0 + 14.0 * t),
                    Align2::CENTER_CENTER,
                    "+1",
                    FontId::proportional(16.0),
                    accent.gamma_multiply(glow.sqrt()),
                );
            }

            let text_color = if selected { Color32::WHITE } else { visuals.text_color() };
            let dim = if selected { Color32::from_white_alpha(200) } else { visuals.weak_text_color() };
            let mut x = rect.left() + 10.0;
            let font = FontId::proportional(14.0);
            if !course.subject.is_empty() {
                let subject = format!("{} › ", course.subject);
                let width = fx::text_size(ui, &subject, font.clone()).x;
                painter.text(Pos2::new(x, rect.center().y), Align2::LEFT_CENTER, subject, font.clone(), dim);
                x += width;
            }
            painter.text(Pos2::new(x, rect.center().y), Align2::LEFT_CENTER, &course.name, font, text_color);

            if let Some(count) = self.module_counts.get(&label).filter(|c| **c > 0) {
                let badge = format!("{count}");
                let size = fx::text_size(ui, &badge, FontId::proportional(12.0));
                let badge_rect = Rect::from_center_size(
                    rect.right_center() - vec2(16.0 + size.x / 2.0, 0.0),
                    vec2(size.x + 12.0, 18.0),
                );
                let badge_fill = if selected { Color32::from_white_alpha(60) } else { accent.gamma_multiply(0.25) };
                painter.rect_filled(badge_rect, CornerRadius::same(9), badge_fill);
                painter.text(badge_rect.center(), Align2::CENTER_CENTER, badge, FontId::proportional(12.0), text_color);
            }
            response.on_hover_cursor(egui::CursorIcon::PointingHand);
        }
        if let Some(i) = clicked {
            self.select(Some(i));
        }
        if let Some(i) = double_clicked {
            self.open_edit(i);
        }
    }

    fn keywords_ui(&mut self, ui: &mut egui::Ui) {
        let lang = self.lang;
        ui.label(lang.keywords_label());
        let enabled = self.selected.is_some();
        const VISIBLE_ROWS: usize = 9;
        // Fixed height: long keyword lists scroll instead of pushing everything below out of view.
        let row_height = ui.text_style_height(&egui::TextStyle::Body);
        let changed = egui::ScrollArea::vertical()
            .id_salt("keywords")
            .max_height(row_height * VISIBLE_ROWS as f32 + 8.0)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                let hint = if enabled { "prog1\nINF-101\nProf. Müller" } else { lang.select_module() };
                let edit = egui::TextEdit::multiline(&mut self.keywords_text)
                    .desired_rows(VISIBLE_ROWS)
                    .desired_width(f32::INFINITY)
                    .hint_text(hint);
                ui.add_enabled(enabled, edit).changed()
            })
            .inner;
        if changed && let Some(i) = self.selected {
            self.config.courses[i].keywords = self
                .keywords_text
                .lines()
                .flat_map(|line| line.split(','))
                .map(str::trim)
                .filter(|k| !k.is_empty())
                .map(str::to_string)
                .collect();
        }
        if self.keyword_scan.is_some() {
            ui.horizontal(|ui| {
                ui.add(egui::Spinner::new().color(fx::PURPLE));
                ui.label(RichText::new(lang.scanning()).color(fx::PURPLE));
            });
        } else if ui
            .add_enabled(enabled, egui::Button::new(format!("{} {}", icon::SPARKLE, lang.suggest_keywords())))
            .on_hover_text(lang.suggest_tooltip())
            .clicked()
        {
            self.start_keyword_scan();
        }
        if let Some(i) = self.selected {
            let folder = self.config.course_folder(&self.config.courses[i]);
            let text = format!("{} {}", lang.files_go_to(), folder.display());
            ui.add(egui::Label::new(RichText::new(text).weak()).wrap());
        }
    }

    // ---------- controls and activity ----------

    fn controls_ui(&mut self, ui: &mut egui::Ui) {
        let lang = self.lang;
        ui.horizontal(|ui| {
            let running = self.running();
            let t = self.ctx.animate_bool_with_time(Id::new("watch-button"), running, 0.35);
            let fill = fx::lerp_color(fx::GREEN, fx::RED, t);
            let (glyph, text) = if running { (icon::STOP, lang.stop()) } else { (icon::PLAY, lang.start()) };
            let label = RichText::new(format!("{glyph}  {text}")).color(Color32::WHITE).strong().size(15.0);
            let button =
                egui::Button::new(label).fill(fill).corner_radius(CornerRadius::same(17)).min_size(vec2(220.0, 34.0));
            if ui.add(button).clicked() {
                self.toggle_watching();
            }
            let sort = egui::Button::new(format!("{} {}", icon::BROOM, lang.sort_now())).min_size(vec2(0.0, 34.0));
            if ui.add(sort).clicked() {
                self.sort_now();
            }
            let save = egui::Button::new(format!("{} {}", icon::FLOPPY_DISK, lang.save())).min_size(vec2(0.0, 34.0));
            if ui.add(save).clicked() {
                self.save();
            }
        });
    }

    fn activity_ui(&mut self, ui: &mut egui::Ui) {
        let lang = self.lang;
        ui.horizontal(|ui| {
            ui.label(RichText::new(icon::LIGHTNING).size(18.0).color(fx::accent(ui)));
            ui.label(RichText::new(lang.activity()).strong().size(16.0));
        });
        egui::ScrollArea::vertical().id_salt("activity").auto_shrink([false, false]).show(ui, |ui| {
            if self.cards.is_empty() {
                ui.add_space(12.0);
                ui.vertical_centered(|ui| {
                    ui.label(RichText::new(icon::TRAY_ARROW_DOWN).size(34.0).weak());
                    ui.label(RichText::new(lang.activity_empty()).weak());
                });
                return;
            }
            let now = self.now();
            for card in self.cards.iter().rev() {
                card_ui(ui, lang, card, now);
            }
        });
    }

    /// Draw the file cards flying from the Downloads field to their module rows.
    fn flights_ui(&mut self, ctx: &egui::Context) {
        if self.flights.is_empty() {
            return;
        }
        let now = self.now();
        let painter = ctx.layer_painter(LayerId::new(Order::Foreground, Id::new("flights")));
        let accent = fx::accent_for(ctx.theme() == egui::Theme::Dark);
        let screen = ctx.content_rect();
        let source = self
            .watch_field_rect
            .map(|r| r.left_center() + vec2(90.0, 0.0))
            .unwrap_or(screen.left_top() + vec2(120.0, 60.0));
        let mut arrived = Vec::new();
        for (i, flight) in self.flights.iter().enumerate() {
            let t = ((now - flight.start) / FLIGHT_SECS) as f32;
            if t >= 1.0 {
                arrived.push(i);
                continue;
            }
            // Fly to the row, or to the edge of the list if the row is scrolled out of view.
            let target = match (self.row_rects.get(&flight.target), self.list_clip) {
                (Some(row), Some(clip)) => {
                    let p = row.left_center() + vec2(60.0, 0.0);
                    Pos2::new(p.x, p.y.clamp(clip.top() + 10.0, clip.bottom() - 10.0))
                }
                (Some(row), None) => row.left_center() + vec2(60.0, 0.0),
                _ => self.list_clip.map(|c| c.center_top()).unwrap_or(screen.center()),
            };
            let control = Pos2::new((source.x + target.x) / 2.0 + 140.0, source.y.min(target.y) - 70.0);
            let e = fx::ease_in_out_cubic(t);
            let pos = fx::bezier(source, control, target, e);
            let pop = fx::ease_out_back((t / 0.25).min(1.0));
            let shrink = if t > 0.7 { 1.0 - (t - 0.7) / 0.3 * 0.45 } else { 1.0 };
            let alpha = if t > 0.85 { 1.0 - (t - 0.85) / 0.15 } else { 1.0 };
            // Sparkly trail.
            for k in 1..=6 {
                let tk = (e - k as f32 * 0.035).max(0.0);
                let p = fx::bezier(source, control, target, tk);
                painter.circle_filled(p, 4.0 - k as f32 * 0.5, accent.gamma_multiply(alpha * (0.5 - k as f32 * 0.07)));
            }
            let mut name = flight.name.clone();
            if name.chars().count() > 26 {
                name = name.chars().take(25).collect::<String>() + "…";
            }
            let font = FontId::proportional(13.0 * pop.max(0.2) * shrink);
            let text = format!("{}  {name}", icon::FILE_TEXT);
            let galley = painter.layout_no_wrap(text, font, Color32::WHITE.gamma_multiply(alpha));
            let rect = Rect::from_center_size(pos, galley.size() + vec2(18.0, 10.0));
            let shadow = Color32::from_black_alpha((60.0 * alpha) as u8);
            painter.rect_filled(rect.translate(vec2(0.0, 3.0)), CornerRadius::same(9), shadow);
            painter.rect_filled(rect, CornerRadius::same(9), accent.gamma_multiply(alpha));
            painter.galley(rect.min + vec2(9.0, 5.0), galley, Color32::WHITE);
        }
        for i in arrived.into_iter().rev() {
            let flight = self.flights.remove(i);
            *self.module_counts.entry(flight.target.clone()).or_default() += 1;
            self.glows.insert(flight.target, now);
        }
    }

    fn dialogs_ui(&mut self, ctx: &egui::Context) {
        let lang = self.lang;
        if self.dialog.is_some() {
            let subjects = self.subjects();
            let mut close = false;
            let mut ok = false;
            let title =
                if self.dialog.as_ref().unwrap().editing.is_some() { lang.edit_module() } else { lang.add_module() };
            let modal = egui::Modal::new(Id::new("course-dialog")).show(ctx, |ui| {
                let d = self.dialog.as_mut().unwrap();
                ui.heading(title);
                egui::Grid::new("course-dialog-grid").num_columns(2).spacing([8.0, 6.0]).show(ui, |ui| {
                    ui.label(lang.study_program());
                    ui.horizontal(|ui| {
                        ui.add(egui::TextEdit::singleline(&mut d.subject).desired_width(220.0));
                        egui::ComboBox::from_id_salt("subject-choices").selected_text("").width(24.0).show_ui(
                            ui,
                            |ui| {
                                for s in &subjects {
                                    if ui.selectable_label(d.subject == *s, s).clicked() {
                                        d.subject = s.clone();
                                    }
                                }
                            },
                        );
                    });
                    ui.end_row();
                    ui.label(lang.module());
                    let name = ui.add(egui::TextEdit::singleline(&mut d.name).desired_width(250.0));
                    if !d.focused {
                        name.request_focus();
                        d.focused = true;
                    }
                    if name.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        ok = true;
                    }
                    ui.end_row();
                });
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.add_enabled(!d.name.trim().is_empty(), egui::Button::new(lang.ok())).clicked() {
                        ok = true;
                    }
                    if ui.button(lang.cancel()).clicked() {
                        close = true;
                    }
                });
            });
            if modal.should_close() {
                close = true;
            }
            if ok && !self.dialog.as_ref().unwrap().name.trim().is_empty() {
                self.apply_dialog();
            } else if close {
                self.dialog = None;
            }
        }

        if let Some(i) = self.confirm_delete {
            let label = self.config.courses.get(i).map(Course::label).unwrap_or_default();
            let modal = egui::Modal::new(Id::new("confirm-delete")).show(ctx, |ui| {
                ui.heading(lang.delete_module());
                ui.label(lang.delete_confirm(&label));
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    let text = RichText::new(format!("{} {}", icon::TRASH, lang.delete())).color(fx::RED);
                    let delete = ui.button(text).clicked();
                    (delete, ui.button(lang.cancel()).clicked())
                })
                .inner
            });
            let (delete, cancel) = modal.inner;
            if delete && i < self.config.courses.len() {
                self.config.courses.remove(i);
                self.select(None);
            }
            if delete || cancel || modal.should_close() {
                self.confirm_delete = None;
            }
        }

        if let Some(error) = &self.error {
            let (title, text) = match error {
                ErrorBox::FolderNotFound(p) => (lang.folder_not_found(), lang.watch_folder_missing(p)),
                ErrorBox::StudyFolderMissing(p) => (lang.folder_not_found(), lang.study_folder_missing(p)),
                ErrorBox::NoModulesFound => (lang.no_modules_found(), lang.no_modules_found_text().to_string()),
                ErrorBox::NoModules => (lang.no_modules(), lang.no_modules_text().to_string()),
                ErrorBox::CouldNotStart(StartError::AlreadyRunning) => {
                    (lang.could_not_start(), lang.already_running().to_string())
                }
                ErrorBox::CouldNotStart(StartError::NoFolder(p)) => {
                    (lang.could_not_start(), lang.watch_folder_missing(&p.display().to_string()))
                }
                ErrorBox::CouldNotStart(e) => (lang.could_not_start(), e.to_string()),
                ErrorBox::CouldNotSave(e) => (lang.could_not_save(), e.clone()),
            };
            let modal = egui::Modal::new(Id::new("error")).show(ctx, |ui| {
                ui.set_max_width(420.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new(icon::WARNING_CIRCLE).size(26.0).color(fx::AMBER));
                    ui.heading(title);
                });
                ui.label(text);
                ui.add_space(8.0);
                ui.button(lang.ok()).clicked()
            });
            if modal.inner || modal.should_close() {
                self.error = None;
            }
        }
    }

    fn apply_dialog(&mut self) {
        let Some(d) = self.dialog.take() else { return };
        let subject = d.subject.trim().to_string();
        let name = d.name.trim().to_string();
        let course = match d.editing {
            Some(i) => {
                let c = &mut self.config.courses[i];
                c.subject = subject;
                c.name = name;
                c.clone()
            }
            None => {
                let mut course = Course::new(name, &[]).with_subject(subject);
                // Reuse the starter keywords an import would give this module, if its folder exists.
                if let Some(found) = scan_study_folder(&self.study_root()).into_iter().find(|f| {
                    f.subject.to_lowercase() == course.subject.to_lowercase()
                        && f.name.to_lowercase() == course.name.to_lowercase()
                }) {
                    course.keywords = found.keywords;
                }
                self.config.courses.push(course.clone());
                course
            }
        };
        self.selected = None;
        self.sort_courses();
        let index = self.config.courses.iter().position(|c| *c == course);
        self.select(index);
    }

    /// Keep animating while something moves; otherwise only repaint when something happens.
    fn schedule_repaint(&self) {
        let now = self.now();
        let animating = !self.flights.is_empty()
            || self.glows.values().any(|t| now - t < GLOW_SECS)
            || self
                .cards
                .iter()
                .rev()
                .take(20)
                .any(|c| now - c.born < CARD_APPEAR_SECS || now - c.changed < CARD_FLASH_SECS);
        if animating {
            self.ctx.request_repaint();
        } else if self.running() {
            // The radar pulse; 25 frames per second are plenty for it.
            self.ctx.request_repaint_after(Duration::from_millis(40));
        }
    }
}

fn today() -> chrono::NaiveDate {
    chrono::Local::now().date_naive()
}

/// Title line of a note, in the given language.
fn note_title(lang: Lang, note: &Note) -> String {
    match note {
        Note::Started(folder) => {
            let path = std::path::Path::new(folder);
            lang.started(&path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| folder.clone()))
        }
        Note::Stopped => lang.stopped_watching().into(),
        Note::Saved => lang.settings_saved().into(),
        Note::SortingExisting => lang.sorting_existing().into(),
        Note::Imported { added, known } => lang.imported(*added, *known),
        Note::KeywordsAdded { module, words } => lang.keywords_added(words.len(), module),
        Note::NoNewKeywords { module, files } => lang.no_new_keywords(module, *files),
        Note::NoFilesToScan { module, .. } => lang.no_files_to_scan(module),
        Note::ScanFailed => lang.scan_failed().into(),
        Note::ConfigUnreadable { path, error } => lang.config_unreadable(path, error),
    }
}

/// What a card shows: icon (empty = spinner), colour, title, line under it, small detail line.
type CardLook = (&'static str, Color32, String, Option<String>, Option<String>);

fn card_look(ui: &egui::Ui, lang: Lang, card: &Card) -> CardLook {
    let weak = ui.visuals().weak_text_color();
    let accent = fx::accent(ui);
    match &card.kind {
        CardKind::File { name, state } => match state {
            FileState::Waiting => ("", fx::BLUE, name.clone(), Some(lang.waiting_for_download().into()), None),
            FileState::Vanished => (icon::X_CIRCLE, weak, name.clone(), Some(lang.vanished().into()), None),
            FileState::Done(Outcome::Moved { course, reason, .. }) => {
                (icon::CHECK_CIRCLE, fx::GREEN, name.clone(), Some(lang.moved_to(course)), Some(lang.reason(reason)))
            }
            FileState::Done(Outcome::DuplicateRemoved { course, .. }) => {
                (icon::COPY_SIMPLE, accent, name.clone(), Some(lang.duplicate_removed(course)), None)
            }
            FileState::Done(Outcome::Left(reason)) => {
                let (glyph, color) = match reason {
                    LeftReason::NoMatch => (icon::WARNING_CIRCLE, fx::AMBER),
                    LeftReason::InUse { .. } => (icon::HOURGLASS_MEDIUM, fx::AMBER),
                    LeftReason::Error(_) => (icon::X_CIRCLE, fx::RED),
                    _ => (icon::INFO, weak),
                };
                (glyph, color, name.clone(), Some(lang.left_reason(reason)), None)
            }
        },
        CardKind::Note(note) => {
            let title = note_title(lang, note);
            match note {
                Note::Started(folder) => (icon::PLAY, fx::GREEN, title, None, Some(folder.clone())),
                Note::Stopped => (icon::STOP, weak, title, None, None),
                Note::Saved => (icon::FLOPPY_DISK, weak, title, None, None),
                Note::SortingExisting => (icon::BROOM, fx::BLUE, title, None, None),
                Note::Imported { .. } => (icon::FOLDERS, accent, title, None, None),
                Note::KeywordsAdded { words, .. } => (icon::SPARKLE, fx::PURPLE, title, Some(words.join(" · ")), None),
                Note::NoNewKeywords { .. } => (icon::SPARKLE, weak, title, None, None),
                Note::NoFilesToScan { folder, .. } => {
                    (icon::FOLDER_OPEN, fx::AMBER, title, Some(lang.folder_empty(folder)), None)
                }
                Note::ScanFailed | Note::ConfigUnreadable { .. } => (icon::X_CIRCLE, fx::RED, title, None, None),
            }
        }
    }
}

/// One card in the activity feed: slides in, flashes when its state changes.
fn card_ui(ui: &mut egui::Ui, lang: Lang, card: &Card, now: f64) {
    let (glyph, color, title, line, detail) = card_look(ui, lang, card);
    let weak = ui.visuals().weak_text_color();
    let appear = fx::ease_out_cubic(((now - card.born) / CARD_APPEAR_SECS) as f32);
    let flash = if card.changed > card.born {
        (1.0 - (now - card.changed) / CARD_FLASH_SECS).clamp(0.0, 1.0) as f32
    } else {
        0.0
    };
    ui.scope(|ui| {
        ui.set_opacity(appear);
        ui.horizontal(|ui| {
            ui.add_space((1.0 - appear) * 36.0);
            let base = ui.visuals().faint_bg_color;
            let response = egui::Frame::new()
                .fill(fx::lerp_color(base, color.gamma_multiply(0.30), flash))
                .stroke(Stroke::new(1.0, color.gamma_multiply(0.30 + 0.6 * flash)))
                .corner_radius(CornerRadius::same(10))
                .inner_margin(Margin { left: 14, right: 10, top: 7, bottom: 7 })
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        if glyph.is_empty() {
                            ui.add(egui::Spinner::new().size(22.0).color(color));
                        } else {
                            ui.label(RichText::new(glyph).size(22.0).color(color));
                        }
                        ui.vertical(|ui| {
                            // Time on the right; the title takes the rest and is shortened if needed.
                            ui.allocate_ui_with_layout(
                                vec2(ui.available_width(), 20.0),
                                Layout::right_to_left(Align::Center),
                                |ui| {
                                    ui.label(RichText::new(&card.clock).small().weak());
                                    ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                                        ui.add(egui::Label::new(RichText::new(&title).strong()).truncate())
                                            .on_hover_text(&title);
                                    });
                                },
                            );
                            if let Some(line) = &line {
                                let line_color = if color == weak { weak } else { color };
                                ui.add(egui::Label::new(RichText::new(line).color(line_color)).wrap());
                            }
                            if let Some(detail) = &detail {
                                ui.add(egui::Label::new(RichText::new(detail).small().weak()).wrap());
                            }
                        });
                    });
                })
                .response;
            // Coloured bar on the left edge.
            let bar = Rect::from_min_size(response.rect.min, vec2(4.0, response.rect.height()));
            ui.painter().rect_filled(bar, CornerRadius { nw: 10, sw: 10, ne: 0, se: 0 }, color);
        });
    });
    ui.add_space(4.0);
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        while let Ok(event) = self.events_rx.try_recv() {
            self.handle_event(event);
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

        let header_fill = fx::lerp_color(ui.visuals().panel_fill, fx::accent(ui), 0.06);
        egui::Panel::top("header")
            .frame(egui::Frame::new().fill(header_fill).inner_margin(Margin::symmetric(14, 10)))
            .show(ui, |ui| self.header_ui(ui));
        egui::Panel::bottom("activity").resizable(true).default_size(210.0).min_size(120.0).show(ui, |ui| {
            ui.add_space(6.0);
            self.activity_ui(ui);
        });
        // Directly above the activity feed, so the buttons are always visible.
        egui::Panel::bottom("controls").show(ui, |ui| {
            ui.add_space(8.0);
            self.controls_ui(ui);
            ui.add_space(6.0);
        });
        egui::CentralPanel::default_margins().show(ui, |ui| {
            egui::ScrollArea::vertical().id_salt("main").auto_shrink([false, false]).show(ui, |ui| {
                let lang = self.lang;
                section(ui, icon::FOLDER_SIMPLE, lang.folders(), |ui| self.folders_ui(ui));
                ui.add_space(10.0);
                section(ui, icon::STACK, lang.modules(), |ui| self.modules_ui(ui));
            });
        });
        let ctx = ui.ctx().clone();
        self.flights_ui(&ctx);
        self.dialogs_ui(&ctx);

        // Keyword or folder edits take effect immediately while watching.
        if let Some(w) = &self.watcher
            && self.pushed.as_ref() != Some(&self.config)
        {
            w.update_config(self.config.clone());
            self.pushed = Some(self.config.clone());
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

/// A titled, rounded section.
fn section(ui: &mut egui::Ui, glyph: &str, title: &str, add_contents: impl FnOnce(&mut egui::Ui)) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(glyph).size(18.0).color(fx::accent(ui)));
        ui.label(RichText::new(title).strong().size(16.0));
    });
    egui::Frame::group(ui.style())
        .corner_radius(CornerRadius::same(12))
        .inner_margin(Margin::same(12))
        .show(ui, add_contents);
}
