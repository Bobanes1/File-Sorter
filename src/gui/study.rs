//! Helpers for the study week: the search over all documents (field in the top bar),
//! and the «This week» tab with deadlines and the files that came in during the week.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};

use chrono::{DateTime, Datelike, Local, NaiveDate, NaiveTime};
use eframe::egui::{self, Align, Id, Layout, Margin, RichText, Sense, text::LayoutJob, vec2};
use egui_phosphor::regular as icon;

use super::dialogs::{actions, dialog_frame};
use super::{App, Tab, section_title};
use crate::deadlines::{self, DEADLINES_FILE, Deadline, Deadlines, Found, Kind};
use crate::fx::{self, ButtonStyle};
use crate::i18n::Lang;
use crate::index::{Hit, Indexer};
use crate::sorter::{Outcome, SortResult};

/// Deadlines found in one file, sent back by the thread that read it.
struct Scanned {
    file: PathBuf,
    size: u64,
    module: String,
    found: Vec<Found>,
}

/// Files of the last days, grouped by module.
type Week = Vec<(String, Vec<(PathBuf, DateTime<Local>)>)>;

/// «+ Deadline»: what is typed so far.
pub(super) struct NewDeadline {
    title: String,
    module: usize,
    date: String,
    kind: Kind,
    /// Correcting this entry instead of adding one.
    editing: Option<u64>,
    /// The file it is in (added by hand after «Check a file» found nothing).
    file: Option<PathBuf>,
}

impl NewDeadline {
    fn empty() -> NewDeadline {
        NewDeadline {
            title: String::new(),
            module: usize::MAX,
            date: String::new(),
            kind: Kind::Due,
            editing: None,
            file: None,
        }
    }
}

/// What «Check a file» found.
struct Checked {
    file: PathBuf,
    module: String,
    found: usize,
    added: usize,
}

/// The last searches, kept between starts.
const RECENT_FILE: &str = "recent-searches.json";

/// Keys pressed in the search field to move through the results.
#[derive(Clone, Copy, PartialEq)]
enum Nav {
    Down,
    Up,
    Open,
    Reveal,
}

/// The kind of file a search looks for.
#[derive(Clone, Copy, PartialEq, Default)]
pub(super) enum FileType {
    #[default]
    Any,
    Pdf,
    Slides,
    Word,
    Text,
}

/// How far back a search looks.
#[derive(Clone, Copy, PartialEq, Default)]
pub(super) enum Period {
    #[default]
    Any,
    Week,
    Month,
    Semester,
}

/// The filters under the search field.
#[derive(Clone, PartialEq, Default)]
pub(super) struct SearchFilter {
    module: Option<usize>,
    kind: Option<crate::categories::Category>,
    file_type: FileType,
    period: Period,
}

/// Documents changed in the last this many days are looked through for dates.
const SCAN_DAYS: i64 = 150;
/// How many pages of a document are looked through for dates.
const SCAN_PAGES: usize = 12;

/// About when a file came (its last change), for dates written without a year or a weekday only.
fn reference_date(file: &Path) -> NaiveDate {
    std::fs::metadata(file)
        .and_then(|m| m.modified())
        .map(|t| DateTime::<Local>::from(t).date_naive())
        .unwrap_or_else(|_| Local::now().date_naive())
}

/// A small tinted label ("new", "date estimated").
fn badge(ui: &mut egui::Ui, text: &str, fill: egui::Color32, fg: egui::Color32) -> egui::Response {
    egui::Frame::new()
        .fill(fill)
        .corner_radius(6)
        .inner_margin(Margin::symmetric(6, 0))
        .show(ui, |ui| ui.label(RichText::new(text).small().color(fg)))
        .response
}

pub(super) struct Study {
    indexer: Option<Indexer>,
    /// The folder the index was last told to cover.
    indexed: Option<(PathBuf, Vec<String>)>,
    pub(super) query: String,
    searched: String,
    /// The filters the last search ran with.
    searched_filter: SearchFilter,
    pub(super) filter: SearchFilter,
    hits: Vec<Hit>,
    /// The result chosen with ↑/↓ (Enter opens it).
    selected: usize,
    /// ↑/↓/Enter pressed in the search field this frame.
    nav: Option<Nav>,
    /// The last searches, newest first.
    recent: Vec<String>,
    /// Ctrl+F was pressed: put the cursor into the search field.
    focus: bool,
    deadlines: Deadlines,
    scan_tx: Sender<Scanned>,
    scan_rx: Receiver<Scanned>,
    new_deadline: Option<NewDeadline>,
    /// Looking through the study folder's documents for dates right now.
    scanning: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// When that was last started.
    scanned_at: f64,
    week: Week,
    /// When `week` was read from the history.
    week_read: f64,
    /// «Check a file»: reading it, and what was found.
    checking: Option<Receiver<(PathBuf, Vec<Found>)>>,
    checked: Option<Checked>,
    /// The frame the tab was last drawn in, to tell a new visit.
    week_shown: Option<u64>,
    /// Entries added after this (Unix seconds) are marked new during this visit.
    new_since: i64,
    /// When the module model (learned from the modules' files) is built next.
    model_at: f64,
    /// When reminders are checked next.
    remind_at: f64,
    training: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl Study {
    pub(super) fn new(state_dir: Option<&Path>) -> Study {
        let (scan_tx, scan_rx) = channel();
        let deadlines = state_dir.map(|d| Deadlines::load(&d.join(DEADLINES_FILE))).unwrap_or_default();
        Study {
            indexer: None,
            indexed: None,
            query: String::new(),
            searched: String::new(),
            searched_filter: SearchFilter::default(),
            filter: SearchFilter::default(),
            selected: 0,
            nav: None,
            recent: state_dir
                .and_then(|d| std::fs::read_to_string(d.join(RECENT_FILE)).ok())
                .and_then(|t| serde_json::from_str(&t).ok())
                .unwrap_or_default(),
            hits: Vec::new(),
            focus: false,
            deadlines,
            scan_tx,
            scan_rx,
            new_deadline: None,
            scanning: Default::default(),
            scanned_at: f64::NEG_INFINITY,
            week: Vec::new(),
            week_read: f64::NEG_INFINITY,
            checking: None,
            checked: None,
            week_shown: None,
            new_since: 0,
            model_at: 30.0,
            remind_at: 5.0,
            training: Default::default(),
        }
    }

    /// The search field has text: show results instead of the tab.
    pub(super) fn searching(&self) -> bool {
        !self.query.trim().is_empty()
    }
}

fn weekday(lang: Lang, date: NaiveDate) -> &'static str {
    let i = date.weekday().num_days_from_monday() as usize;
    match lang {
        Lang::De => ["Mo", "Di", "Mi", "Do", "Fr", "Sa", "So"][i],
        Lang::En => ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"][i],
    }
}

/// Where a file is, relative to the study folder ("Informatik › Programmieren › Übungen").
fn place(path: &Path, root: &Path) -> String {
    let parent = path.parent().unwrap_or(path);
    let relative = parent.strip_prefix(root).unwrap_or(parent);
    relative.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect::<Vec<_>>().join(" › ")
}

impl App {
    /// Every frame: keep the index pointed at the study folder, take in found deadlines.
    pub(super) fn study_logic(&mut self) {
        let features = &self.config.features;
        // The index also serves the deadlines: every document in the study folder is looked through.
        if (features.search || features.deadlines) && !self.config.destination_root.trim().is_empty() {
            let wanted = (self.study_root(), self.config.extensions.clone());
            if self.study.indexer.is_none() {
                let ctx = self.ctx.clone();
                self.study.indexer = Some(Indexer::start(self.config.state_dir.clone(), move || ctx.request_repaint()));
            }
            if self.study.indexed.as_ref() != Some(&wanted)
                && let Some(indexer) = &self.study.indexer
            {
                indexer.set_root(wanted.0.clone(), wanted.1.clone());
                self.study.indexed = Some(wanted);
            }
        }
        if self.config.features.deadlines && self.now() - self.study.scanned_at > 20.0 {
            self.scan_index_for_deadlines();
        }
        let mut changed = false;
        while let Ok(scanned) = self.study.scan_rx.try_recv() {
            let added = self.study.deadlines.add_found(&scanned.file, scanned.size, &scanned.module, scanned.found);
            changed = true;
            if added > 0 {
                log::info!("Found {added} deadline(s) in {}", scanned.file.display());
            }
        }
        if changed {
            self.save_deadlines();
        }
        self.train_model();
        self.remind();
        // Search as you type, once the index answers.
        if self.study.query != self.study.searched || self.study.filter != self.study.searched_filter {
            self.run_search();
        }
    }

    /// Drop search results (saving resources in the background).
    pub(super) fn study_forget_results(&mut self) {
        self.study.hits = Vec::new();
        self.study.searched.clear();
    }

    /// Remind of deadlines that come close (checked every minute, also in the background).
    fn remind(&mut self) {
        let features = &self.config.features;
        if !features.deadlines || !features.reminders {
            return;
        }
        let now = self.now();
        if now < self.study.remind_at {
            return;
        }
        self.study.remind_at = now + 60.0;
        self.ctx.request_repaint_after(std::time::Duration::from_secs(60));
        let leads = features.reminder_leads.clone();
        let due = crate::reminders::take_due(&mut self.study.deadlines, &leads, Local::now().naive_local());
        if due.is_empty() {
            return;
        }
        self.save_deadlines();
        let lang = self.lang;
        for d in due {
            let Some(at) = d.due() else { continue };
            let days = (at.date() - Local::now().date_naive()).num_days();
            let when = lang.due_when(days, false, d.time.as_deref());
            let title = format!("{}: {}", lang.deadline_kind(d.kind), d.title);
            let module = d.module.rsplit(" › ").next().unwrap_or(&d.module).to_string();
            let body = if module.is_empty() { when.clone() } else { format!("{module} · {when}") };
            crate::reminders::notify(&title, &body);
            self.note(super::Note::Reminder { title, details: body });
        }
    }

    /// Now and then (and a little after the start): learn the modules from their files again,
    /// for files the keywords don't decide.
    fn train_model(&mut self) {
        use std::sync::atomic::Ordering;
        let now = self.now();
        if !self.config.features.classifier
            || self.lean
            || self.config.courses.is_empty()
            || now < self.study.model_at
            || self.study.training.load(Ordering::Relaxed)
            || self.study.indexer.as_ref().is_some_and(|i| i.busy())
        {
            return;
        }
        self.study.model_at = now + 60.0 * 60.0;
        let Some(file) = self.config.state_file(crate::classifier::MODEL_FILE) else { return };
        let config = self.config.clone();
        let index = self.study.indexer.as_ref().map(|i| i.index.clone());
        let flag = self.study.training.clone();
        flag.store(true, Ordering::Relaxed);
        std::thread::spawn(move || {
            let starts =
                index.and_then(|i| i.read().ok().map(|i| i.starts(crate::classifier::TEXT_WORDS))).unwrap_or_default();
            let model = crate::classifier::Model::train(&config, &starts);
            if let Err(e) = model.save(&file) {
                log::info!("Could not save the module model: {e}");
            }
            flag.store(false, Ordering::Relaxed);
        });
    }

    fn run_search(&mut self) {
        self.study.searched = self.study.query.clone();
        self.study.searched_filter = self.study.filter.clone();
        self.study.selected = 0;
        let f = &self.study.filter;
        let days =
            |d: i64| (chrono::Utc::now() - chrono::Duration::days(d)).timestamp_nanos_opt().map(|n| n.max(0) as u64);
        let filter = crate::index::Filter {
            folder: f.module.and_then(|i| self.config.courses.get(i)).map(|c| self.config.course_folder(c)),
            kind: f.kind,
            extensions: match f.file_type {
                FileType::Any => Vec::new(),
                FileType::Pdf => vec!["pdf".into()],
                FileType::Slides => vec!["pptx".into(), "ppt".into(), "key".into(), "odp".into()],
                FileType::Word => vec!["docx".into(), "doc".into(), "odt".into()],
                FileType::Text => vec!["txt".into(), "md".into(), "ipynb".into()],
            },
            since: match f.period {
                Period::Any => None,
                Period::Week => days(7),
                Period::Month => days(31),
                Period::Semester => days(183),
            },
        };
        let query = crate::index::Query::parse(&self.study.query);
        self.study.hits = match &self.study.indexer {
            Some(indexer) => indexer.index.read().map(|i| i.search_with(&query, &filter, 80)).unwrap_or_default(),
            None => Vec::new(),
        };
    }

    /// Remember a search that was used (a result opened).
    fn remember_search(&mut self) {
        let query = self.study.query.trim().to_string();
        if query.is_empty() {
            return;
        }
        self.study.recent.retain(|q| *q != query);
        self.study.recent.insert(0, query);
        self.study.recent.truncate(8);
        if let Some(file) = self.config.state_file(RECENT_FILE) {
            let _ = crate::cloud::save_json(&self.study.recent, &file);
        }
    }

    /// Deadlines follow their files when they move.
    pub(super) fn deadlines_follow(&mut self, moved: impl Fn(&Path) -> Option<PathBuf>) {
        if self.study.deadlines.follow(moved) {
            self.save_deadlines();
        }
    }

    fn save_deadlines(&self) {
        if let Some(file) = self.config.state_file(DEADLINES_FILE)
            && let Err(e) = self.study.deadlines.save(&file)
        {
            log::info!("Could not save the deadlines: {e}");
        }
    }

    /// A file was sorted: index it, look for deadlines in it, and show it in this week's list.
    pub(super) fn study_after_sort(&mut self, result: &SortResult) {
        let (Outcome::Moved { destination, course, .. } | Outcome::Copied { destination, course, .. }) =
            &result.outcome
        else {
            return;
        };
        if let Some(indexer) = &self.study.indexer {
            indexer.refresh();
        }
        self.study.week_read = f64::NEG_INFINITY;
        if self.config.features.deadlines {
            self.scan_for_deadlines(destination.clone(), course.clone());
        }
    }

    /// Read `file` on a background thread and look for deadlines in it.
    fn scan_for_deadlines(&self, file: PathBuf, module: String) {
        let Ok(meta) = std::fs::metadata(&file) else { return };
        if self.study.deadlines.was_scanned(&file, meta.len()) || crate::cloud::is_placeholder(&file) {
            return;
        }
        let tx = self.study.scan_tx.clone();
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            // Just downloaded: dates without a year are the next ones from today.
            let found = deadlines::find(&crate::extract::first_pages(&file, SCAN_PAGES), Local::now().date_naive());
            let _ = tx.send(Scanned { file, size: meta.len(), module, found });
            ctx.request_repaint();
        });
    }

    /// Look through the documents in the search index (the whole study folder, also files
    /// sorted from the right-click menu or copied in by hand) that were changed in the last
    /// months and not looked at yet.
    fn scan_index_for_deadlines(&mut self) {
        use std::sync::atomic::Ordering;
        self.study.scanned_at = self.now();
        let Some(indexer) = &self.study.indexer else { return };
        if indexer.busy() || self.study.scanning.load(Ordering::Relaxed) {
            return;
        }
        let since =
            (chrono::Utc::now() - chrono::Duration::days(SCAN_DAYS)).timestamp_nanos_opt().unwrap_or(0).max(0) as u64;
        let deadlines = &self.study.deadlines;
        let docs = match indexer.index.read() {
            Ok(index) => index.texts(since, SCAN_PAGES, 200, |path, size| deadlines.was_scanned(path, size)),
            Err(_) => return,
        };
        if docs.is_empty() {
            return;
        }
        // Which module each document is in, by its folder.
        let folders: Vec<(PathBuf, String)> =
            self.config.courses.iter().map(|c| (self.config.course_folder(c), c.label())).collect();
        let (tx, ctx, flag) = (self.study.scan_tx.clone(), self.ctx.clone(), self.study.scanning.clone());
        flag.store(true, Ordering::Relaxed);
        std::thread::spawn(move || {
            for (file, size, modified, text) in docs {
                let module = folders
                    .iter()
                    .find(|(folder, _)| file.starts_with(folder))
                    .map(|(_, label)| label.clone())
                    .unwrap_or_default();
                let reference = DateTime::from_timestamp(modified, 0)
                    .map(|t| t.with_timezone(&Local).date_naive())
                    .unwrap_or_else(|| Local::now().date_naive());
                let found = deadlines::find(&text, reference);
                if tx.send(Scanned { file, size, module, found }).is_err() {
                    break;
                }
            }
            flag.store(false, Ordering::Relaxed);
            ctx.request_repaint();
        });
    }

    /// The first time deadlines are on: look through the files sorted in the last weeks.
    pub(super) fn scan_recent_for_deadlines(&self) {
        let Some(dir) = &self.config.state_dir else { return };
        let since = Local::now() - chrono::Duration::days(30);
        for entry in crate::history::recent(dir, 300) {
            if entry.undone || entry.time < since {
                continue;
            }
            if let Outcome::Moved { destination, course, .. } | Outcome::Copied { destination, course, .. } =
                entry.result.outcome
                && destination.is_file()
            {
                self.scan_for_deadlines(destination, course);
            }
        }
    }

    fn read_week(&mut self) {
        let mut groups: BTreeMap<String, Vec<(PathBuf, DateTime<Local>)>> = BTreeMap::new();
        if let Some(dir) = &self.config.state_dir {
            let since = Local::now() - chrono::Duration::days(7);
            for entry in crate::history::recent(dir, 500) {
                if entry.undone || entry.time < since {
                    continue;
                }
                if let Outcome::Moved { destination, course, .. } | Outcome::Copied { destination, course, .. } =
                    entry.result.outcome
                {
                    let files = groups.entry(course).or_default();
                    files.retain(|(p, _)| *p != destination);
                    files.push((destination, entry.time));
                }
            }
        }
        let mut week: Week = groups.into_iter().collect();
        for (_, files) in &mut week {
            files.sort_by_key(|f| std::cmp::Reverse(f.1));
        }
        // Modules with the most new files first.
        week.sort_by(|a, b| b.1.len().cmp(&a.1.len()).then(a.0.cmp(&b.0)));
        self.study.week = week;
        self.study.week_read = self.now();
    }

    // ---------- search ----------

    /// The search field in the top bar (Ctrl+F).
    pub(super) fn search_field(&mut self, ui: &mut egui::Ui) {
        if !self.config.features.search {
            self.study.query.clear();
            return;
        }
        let p = fx::pal(ui);
        let lang = self.lang;
        if ui.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::F)) {
            self.study.focus = true;
        }
        let id = Id::new("search-field");
        // ↑/↓ choose a result, Enter opens it, Ctrl+Enter shows it in its folder.
        let focused = ui.memory(|m| m.has_focus(id));
        if focused && !self.study.query.is_empty() {
            self.study.nav = ui.input_mut(|i| {
                if i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown) {
                    Some(Nav::Down)
                } else if i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp) {
                    Some(Nav::Up)
                } else if i.consume_key(egui::Modifiers::COMMAND, egui::Key::Enter) {
                    Some(Nav::Reveal)
                } else if i.consume_key(egui::Modifiers::NONE, egui::Key::Enter) {
                    Some(Nav::Open)
                } else {
                    None
                }
            });
        }
        let field = egui::TextEdit::singleline(&mut self.study.query)
            .id(id)
            .hint_text(format!("{}  {}", icon::MAGNIFYING_GLASS, lang.search_hint()))
            .desired_width(240.0)
            .margin(Margin::ZERO);
        let frame = egui::Frame::new()
            .fill(p.container_highest)
            .corner_radius(egui::CornerRadius::same(20))
            .inner_margin(Margin::symmetric(16, 10));
        let response = frame.show(ui, |ui| ui.add(field.frame(egui::Frame::NONE))).inner;
        if self.study.focus {
            response.request_focus();
            self.study.focus = false;
        }
        if response.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.study.query.clear();
        }
        // The last searches, while the field is empty.
        if response.has_focus() && self.study.query.is_empty() && !self.study.recent.is_empty() {
            let mut chosen = None;
            egui::Area::new(Id::new("recent-searches"))
                .order(egui::Order::Foreground)
                .fixed_pos(response.rect.left_bottom() + vec2(0.0, 6.0))
                .show(ui.ctx(), |ui| {
                    egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.set_width(response.rect.width() + 32.0);
                        ui.label(RichText::new(lang.recent_searches()).small().color(p.on_surface_variant));
                        for query in &self.study.recent {
                            let row = ui.add(egui::Button::selectable(
                                false,
                                format!("{}  {query}", icon::CLOCK_COUNTER_CLOCKWISE),
                            ));
                            if row.clicked() {
                                chosen = Some(query.clone());
                            }
                        }
                    });
                });
            if let Some(query) = chosen {
                self.study.query = query;
                self.study.focus = true;
            }
        }
    }

    /// Module, kind, file type and period to narrow a search.
    fn search_filters_ui(&mut self, ui: &mut egui::Ui) {
        use crate::categories::Category;
        let lang = self.lang;
        let f = &mut self.study.filter;
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
            let modules: Vec<String> = self.config.courses.iter().map(|c| c.name.clone()).collect();
            let shown = f.module.and_then(|i| modules.get(i).cloned()).unwrap_or_else(|| lang.all_modules().into());
            egui::ComboBox::from_id_salt("search-module").selected_text(shown).width(170.0).show_ui(ui, |ui| {
                ui.selectable_value(&mut f.module, None, lang.all_modules());
                for (i, m) in modules.iter().enumerate() {
                    ui.selectable_value(&mut f.module, Some(i), m);
                }
            });
            for (kind, label) in [Category::Lecture, Category::Exercise, Category::Solution, Category::Exam]
                .into_iter()
                .zip(lang.kind_names())
            {
                let on = f.kind == Some(kind);
                if fx::chip(ui, on, label).clicked() {
                    f.kind = if on { None } else { Some(kind) };
                }
            }
            let types = [
                (FileType::Any, lang.any_type()),
                (FileType::Pdf, "PDF"),
                (FileType::Slides, lang.slides()),
                (FileType::Word, "Word"),
                (FileType::Text, lang.text_files()),
            ];
            let shown = types.iter().find(|(t, _)| *t == f.file_type).map_or("", |(_, l)| l);
            egui::ComboBox::from_id_salt("search-type").selected_text(shown).width(110.0).show_ui(ui, |ui| {
                for (t, label) in types {
                    ui.selectable_value(&mut f.file_type, t, label);
                }
            });
            let periods = [
                (Period::Any, lang.any_time()),
                (Period::Week, lang.last_week()),
                (Period::Month, lang.last_month()),
                (Period::Semester, lang.this_semester()),
            ];
            let shown = periods.iter().find(|(t, _)| *t == f.period).map_or("", |(_, l)| l);
            egui::ComboBox::from_id_salt("search-period").selected_text(shown).width(130.0).show_ui(ui, |ui| {
                for (t, label) in periods {
                    ui.selectable_value(&mut f.period, t, label);
                }
            });
            if *f != SearchFilter::default()
                && fx::button(ui, ButtonStyle::Text, icon::X, lang.reset_filters()).clicked()
            {
                *f = SearchFilter::default();
            }
        });
    }

    /// The results while something is typed into the search field.
    pub(super) fn search_results_ui(&mut self, ui: &mut egui::Ui) {
        let p = fx::pal(ui);
        let lang = self.lang;
        let root = self.study_root();
        let (busy, count) = match &self.study.indexer {
            Some(indexer) => (indexer.busy(), indexer.index.read().map(|i| i.len()).unwrap_or(0)),
            None => (false, 0),
        };
        if busy {
            // New files arrive in the index while it is being built.
            self.run_search();
            self.ctx.request_repaint_after(std::time::Duration::from_millis(500));
        }
        ui.horizontal(|ui| {
            ui.label(fx::title(lang.search_results(self.study.hits.len()), 18.0).color(p.on_surface));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if fx::button(ui, ButtonStyle::Text, icon::X, lang.close()).clicked() {
                    self.study.query.clear();
                }
                let status = if busy { lang.indexing(count) } else { lang.indexed(count) };
                ui.label(RichText::new(status).small().color(p.on_surface_variant));
                if busy {
                    ui.add(egui::Spinner::new().size(14.0));
                }
            });
        });
        ui.add_space(4.0);
        self.search_filters_ui(ui);
        ui.label(RichText::new(lang.search_tips()).small().color(p.outline));
        ui.add_space(6.0);
        let mut open = None;
        let mut reveal = None;
        // ↑/↓/Enter from the search field.
        let count = self.study.hits.len();
        match self.study.nav.take() {
            Some(Nav::Down) if count > 0 => self.study.selected = (self.study.selected + 1).min(count - 1),
            Some(Nav::Up) => self.study.selected = self.study.selected.saturating_sub(1),
            Some(Nav::Open) => open = self.study.hits.get(self.study.selected).map(|h| h.path.clone()),
            Some(Nav::Reveal) => reveal = self.study.hits.get(self.study.selected).map(|h| h.path.clone()),
            _ => {}
        }
        let selected = self.study.selected;
        let module_filter = self.study.filter.module.and_then(|i| self.config.courses.get(i)).map(|c| c.label());
        let note_hits: Vec<(String, String)> = self
            .notes
            .search(&self.study.query)
            .into_iter()
            .filter(|(m, _)| module_filter.as_ref().is_none_or(|f| f == m))
            .collect();
        let mut open_note: Option<String> = None;
        egui::ScrollArea::vertical().id_salt("search-results").auto_shrink([false, false]).show(ui, |ui| {
            // Notes of modules come first.
            for (module, passage) in &note_hits {
                let response = fx::card(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(icon::NOTE_PENCIL).size(18.0).color(p.primary));
                        let short = module.rsplit(" › ").next().unwrap_or(module);
                        ui.label(fx::title(lang.note_hit(short), 15.0).color(p.on_surface));
                    });
                    ui.add(egui::Label::new(RichText::new(passage).color(p.on_surface_variant)).wrap());
                });
                let click = ui.interact(response.rect, Id::new(("note-hit", module)), Sense::click());
                if click.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                    open_note = Some(module.clone());
                }
                ui.add_space(6.0);
            }
            if self.study.hits.is_empty() && note_hits.is_empty() {
                ui.add_space(24.0);
                ui.vertical_centered(|ui| {
                    ui.label(RichText::new(icon::MAGNIFYING_GLASS).size(34.0).color(p.outline));
                    ui.label(RichText::new(lang.no_results()).color(p.on_surface_variant));
                });
                return;
            }
            for (k, hit) in self.study.hits.iter().enumerate() {
                let response = fx::card(ui, |ui| {
                    ui.horizontal(|ui| {
                        let name = hit.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                        ui.label(RichText::new(icon::FILE_TEXT).size(18.0).color(p.primary));
                        ui.label(fx::title(&name, 15.0).color(p.on_surface));
                        if let Some(page) = hit.page {
                            ui.label(RichText::new(lang.page(page)).small().color(p.on_surface_variant));
                        }
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if fx::button(ui, ButtonStyle::Text, icon::FOLDER_OPEN, lang.show_in_folder()).clicked() {
                                reveal = Some(hit.path.clone());
                            }
                        });
                    });
                    ui.label(RichText::new(place(&hit.path, &root)).small().color(p.on_surface_variant));
                    if !hit.snippet.is_empty() {
                        ui.add_space(2.0);
                        ui.add(egui::Label::new(highlighted(ui, &hit.snippet, &hit.marks)).wrap());
                    }
                    if !hit.more_pages.is_empty() {
                        ui.label(RichText::new(lang.also_on_pages(&hit.more_pages)).small().color(p.outline));
                    }
                });
                if k == selected {
                    ui.painter().rect_stroke(
                        response.rect,
                        egui::CornerRadius::same(16),
                        egui::Stroke::new(2.0, p.primary),
                        egui::StrokeKind::Inside,
                    );
                    if self.study.hits.len() > 1 {
                        response.scroll_to_me(None);
                    }
                }
                let click = ui.interact(response.rect, Id::new(("search-hit", &hit.path)), Sense::click());
                if click.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(lang.open_file()).clicked() {
                    open = Some(hit.path.clone());
                }
                ui.add_space(6.0);
            }
        });
        if let Some(module) = open_note {
            self.remember_search();
            self.study.query.clear();
            if let Some(i) = self.config.courses.iter().position(|c| c.label() == module) {
                self.select(Some(i));
                self.module_pane = super::overview::ModulePane::Notes;
                self.switch_tab(Tab::Overview);
            }
        }
        if open.is_some() || reveal.is_some() {
            self.remember_search();
        }
        if let Some(path) = open {
            crate::desktop::open(&path);
        }
        if let Some(path) = reveal {
            crate::desktop::reveal(&path);
        }
    }

    // ---------- this week ----------

    pub(super) fn study_indexer(&self) -> Option<&Indexer> {
        self.study.indexer.as_ref()
    }

    pub(super) fn week_tab_visible(&self) -> bool {
        self.config.features.deadlines || self.config.features.weekly
    }

    pub(super) fn week_ui(&mut self, ui: &mut egui::Ui) {
        if !self.week_tab_visible() {
            self.tab = Tab::Overview;
            return;
        }
        if self.now() - self.study.week_read > 30.0 {
            self.read_week();
        }
        // A new visit (the tab wasn't drawn in the last frame): entries added since the last
        // visit are marked «new» during this one.
        let frame = ui.ctx().cumulative_frame_nr();
        if self.study.week_shown.is_none_or(|last| frame > last + 1) {
            self.study.new_since = self.study.deadlines.seen;
            self.study.deadlines.seen = chrono::Utc::now().timestamp();
            self.save_deadlines();
        }
        self.study.week_shown = Some(frame);
        let width = ui.available_width().min(860.0);
        let margin = ((ui.available_width() - width) / 2.0).max(0.0);
        egui::ScrollArea::vertical().id_salt("week").auto_shrink([false, false]).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.add_space(margin);
                ui.vertical(|ui| {
                    ui.set_width(width);
                    self.today_lessons_ui(ui);
                    if self.config.features.deadlines {
                        self.deadlines_ui(ui);
                        ui.add_space(16.0);
                    }
                    if self.config.features.weekly {
                        self.weekly_ui(ui);
                    }
                    ui.add_space(24.0);
                });
            });
        });
        self.deadline_dialog(&ui.ctx().clone());
    }

    /// Pick one file and look for deadlines in it right away (also one looked at before).
    fn check_file(&mut self) {
        let mut dialog = rfd::FileDialog::new().add_filter("", &["pdf", "docx", "pptx", "txt", "md"]);
        let root = self.study_root();
        if root.is_dir() {
            dialog = dialog.set_directory(&root);
        }
        let Some(file) = dialog.pick_file() else { return };
        let (tx, rx) = channel();
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            let found = deadlines::find(&crate::extract::first_pages(&file, SCAN_PAGES), reference_date(&file));
            let _ = tx.send((file, found));
            ctx.request_repaint();
        });
        self.study.checking = Some(rx);
        self.study.checked = None;
    }

    /// Take in the result of «Check a file».
    fn checked_logic(&mut self) {
        let Some(rx) = &self.study.checking else { return };
        let Ok((file, found)) = rx.try_recv() else { return };
        self.study.checking = None;
        let module = self.module_of(&file);
        let size = std::fs::metadata(&file).map(|m| m.len()).unwrap_or(0);
        let count = found.len();
        let added = self.study.deadlines.add_found(&file, size, &module, found);
        self.save_deadlines();
        self.study.checked = Some(Checked { file, module, found: count, added });
    }

    /// The module a file in the study folder belongs to, by its folder.
    fn module_of(&self, file: &Path) -> String {
        self.config
            .courses
            .iter()
            .find(|c| file.starts_with(self.config.course_folder(c)))
            .map(|c| c.label())
            .unwrap_or_default()
    }

    fn deadlines_ui(&mut self, ui: &mut egui::Ui) {
        self.checked_logic();
        let p = fx::pal(ui);
        let lang = self.lang;
        let now = Local::now().naive_local();
        let today = now.date();
        let monday = today - chrono::Duration::days(today.weekday().num_days_from_monday() as i64);
        let sunday = monday + chrono::Duration::days(6);
        // Title with the week, and what can be done.
        ui.horizontal(|ui| {
            ui.label(fx::title(lang.deadlines_title(), 20.0).color(p.on_surface));
            ui.label(
                RichText::new(format!("{} – {}", monday.format("%d.%m."), sunday.format("%d.%m.%Y")))
                    .color(p.on_surface_variant),
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                if fx::button(ui, ButtonStyle::Tonal, icon::PLUS, lang.add_deadline()).clicked() {
                    self.study.new_deadline = Some(NewDeadline::empty());
                }
                let checking = self.study.checking.is_some();
                let check = ui.add_enabled_ui(!checking, |ui| {
                    fx::button(ui, ButtonStyle::Text, icon::FILE_MAGNIFYING_GLASS, lang.check_file())
                });
                if check.inner.on_hover_text(lang.check_file_hint()).clicked() {
                    self.check_file();
                }
                let scanning = self.study.scanning.load(std::sync::atomic::Ordering::Relaxed);
                if !scanning
                    && fx::button(ui, ButtonStyle::Text, icon::ARROW_CLOCKWISE, lang.search_again())
                        .on_hover_text(lang.search_again_hint())
                        .clicked()
                {
                    self.study.deadlines.forget_scanned();
                    self.save_deadlines();
                    self.study.scanned_at = f64::NEG_INFINITY;
                    self.scan_recent_for_deadlines();
                }
            });
        });
        // How much was looked through, so a missing date can be told from a file not read yet.
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            let scanning = self.study.scanning.load(std::sync::atomic::Ordering::Relaxed)
                || self.study.checking.is_some()
                || self.study.indexer.as_ref().is_some_and(|i| i.busy());
            if scanning {
                ui.add(egui::Spinner::new().size(12.0));
            }
            let status = lang.scanned_status(self.study.deadlines.scanned_count(), scanning);
            ui.label(RichText::new(status).small().color(p.on_surface_variant));
        });
        ui.add_space(6.0);
        if let Some(checked) = &self.study.checked {
            let mut close = false;
            let mut by_hand = false;
            let name = checked.file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let (fill, fg) = if checked.found == 0 {
                (p.warning_container, p.on_warning_container)
            } else {
                (p.secondary_container, p.on_secondary_container)
            };
            egui::Frame::new().fill(fill).corner_radius(12).inner_margin(Margin::symmetric(14, 10)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    let text = if checked.found == 0 {
                        lang.checked_nothing(&name)
                    } else {
                        lang.checked_found(&name, checked.found, checked.added)
                    };
                    ui.add(egui::Label::new(RichText::new(text).color(fg)).wrap());
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if fx::icon_button(ui, icon::X).on_hover_text(lang.close()).clicked() {
                            close = true;
                        }
                        if checked.found == 0
                            && fx::button(ui, ButtonStyle::Text, icon::PLUS, lang.add_by_hand()).clicked()
                        {
                            by_hand = true;
                        }
                    });
                });
            });
            ui.add_space(8.0);
            if by_hand {
                let module = self.config.courses.iter().position(|c| c.label() == checked.module).unwrap_or(usize::MAX);
                let title = checked.file.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                self.study.new_deadline =
                    Some(NewDeadline { title, module, file: Some(checked.file.clone()), ..NewDeadline::empty() });
            }
            if close || by_hand {
                self.study.checked = None;
            }
        }
        let upcoming: Vec<Deadline> = self.study.deadlines.upcoming(now, 7, 366).into_iter().cloned().collect();
        self.week_strip(ui, monday, today, &upcoming);
        ui.add_space(10.0);
        let mut change: Option<(u64, &str)> = None;
        if upcoming.is_empty() {
            fx::card(ui, |ui| {
                ui.add(egui::Label::new(RichText::new(lang.no_deadlines()).color(p.on_surface_variant)).wrap());
            });
        }
        // Overdue, this week, next week, later (closed: far away is less urgent).
        let next_sunday = sunday + chrono::Duration::days(7);
        let group = |d: &Deadline| match d.due() {
            Some(due) if due < now => 0,
            Some(due) if due.date() <= sunday => 1,
            Some(due) if due.date() <= next_sunday => 2,
            _ => 3,
        };
        for g in 0..4 {
            let items: Vec<&Deadline> = upcoming.iter().filter(|d| group(d) == g).collect();
            if items.is_empty() {
                continue;
            }
            let title = match g {
                0 => lang.overdue_group(items.len()),
                1 => lang.this_week_group(items.len()),
                2 => lang.next_week_group(items.len()),
                _ => lang.later_group(items.len()),
            };
            let header =
                egui::CollapsingHeader::new(RichText::new(title).family(fx::medium()).size(14.0).color(if g == 0 {
                    p.error
                } else {
                    p.primary
                }))
                .id_salt(("deadline-group", g))
                .default_open(g < 3);
            header.show_unindented(ui, |ui| {
                fx::card(ui, |ui| {
                    for (i, d) in items.iter().enumerate() {
                        if i > 0 {
                            super::settings::divider(ui);
                        }
                        if let Some(c) = self.deadline_row(ui, d, now) {
                            change = Some(c);
                        }
                    }
                });
            });
            ui.add_space(6.0);
        }
        let done: Vec<Deadline> = self.study.deadlines.recently_done(today, 14).into_iter().cloned().collect();
        if !done.is_empty() {
            let header = egui::CollapsingHeader::new(
                RichText::new(lang.done_group(done.len())).family(fx::medium()).size(14.0).color(p.on_surface_variant),
            )
            .id_salt("deadline-done")
            .default_open(false);
            header.show_unindented(ui, |ui| {
                fx::card(ui, |ui| {
                    for (i, d) in done.iter().enumerate() {
                        if i > 0 {
                            super::settings::divider(ui);
                        }
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(icon::CHECK_CIRCLE).size(18.0).color(p.success));
                            let day = d.day().map(|day| format!("{} {}", weekday(lang, day), day.format("%d.%m.")));
                            ui.label(RichText::new(&d.title).strikethrough().color(p.on_surface_variant));
                            ui.label(RichText::new(day.unwrap_or_default()).small().color(p.outline));
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                if fx::button(ui, ButtonStyle::Text, icon::ARROW_U_UP_LEFT, lang.reopen()).clicked() {
                                    change = Some((d.id, "reopen"));
                                }
                            });
                        });
                    }
                });
            });
        }
        ui.add_space(4.0);
        ui.label(RichText::new(lang.deadlines_note()).small().color(p.outline));
        if let Some((id, what)) = change {
            self.change_deadline(id, what);
        }
    }

    /// Mon–Sun of this week, today marked, a dot per deadline in the colour of its urgency.
    fn week_strip(&self, ui: &mut egui::Ui, monday: NaiveDate, today: NaiveDate, upcoming: &[Deadline]) {
        let p = fx::pal(ui);
        let lang = self.lang;
        let gap = 8.0;
        let width = ((ui.available_width() - gap * 6.0) / 7.0).max(40.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for i in 0..7 {
                let day = monday + chrono::Duration::days(i);
                let due: Vec<&Deadline> = upcoming.iter().filter(|d| d.day() == Some(day)).collect();
                let (rect, response) = ui.allocate_exact_size(vec2(width, 62.0), Sense::hover());
                let is_today = day == today;
                let fill = if is_today { p.primary_container } else { p.card };
                let fg = if is_today { p.on_primary_container } else { p.on_surface };
                let painter = ui.painter();
                painter.rect_filled(rect, egui::CornerRadius::same(12), fill);
                let faded = if day < today { p.outline } else { p.on_surface_variant };
                painter.text(
                    rect.center_top() + vec2(0.0, 12.0),
                    egui::Align2::CENTER_CENTER,
                    weekday(lang, day),
                    egui::FontId::proportional(12.0),
                    if is_today { fg } else { faded },
                );
                painter.text(
                    rect.center_top() + vec2(0.0, 30.0),
                    egui::Align2::CENTER_CENTER,
                    day.format("%d").to_string(),
                    fx::medium_font(16.0),
                    if day < today && !is_today { faded } else { fg },
                );
                let dots = due.len().min(4);
                for (k, d) in due.iter().take(dots).enumerate() {
                    let x = rect.center().x + (k as f32 - (dots as f32 - 1.0) / 2.0) * 10.0;
                    let color = match d.kind {
                        Kind::Exam => p.error,
                        Kind::Due if day <= today + chrono::Duration::days(2) => p.warning,
                        Kind::Due => p.primary,
                    };
                    painter.circle_filled(egui::pos2(x, rect.bottom() - 11.0), 3.5, color);
                }
                if !due.is_empty() {
                    let titles = due.iter().map(|d| d.title.as_str()).collect::<Vec<_>>().join("\n");
                    response.on_hover_text(titles);
                }
            }
        });
    }

    /// One deadline: the day in a tinted box, what and when, and what can be done with it.
    fn deadline_row(&self, ui: &mut egui::Ui, d: &Deadline, now: chrono::NaiveDateTime) -> Option<(u64, &'static str)> {
        let p = fx::pal(ui);
        let lang = self.lang;
        let mut change = None;
        let due = d.due()?;
        let days = (due.date() - now.date()).num_days();
        let late = due < now;
        let (fill, fg) = if late {
            (p.error_container, p.on_error_container)
        } else if days <= 2 {
            (p.warning_container, p.on_warning_container)
        } else {
            (p.secondary_container, p.on_secondary_container)
        };
        ui.horizontal(|ui| {
            ui.set_min_height(48.0);
            let (rect, _) = ui.allocate_exact_size(vec2(58.0, 44.0), Sense::hover());
            ui.painter().rect_filled(rect, egui::CornerRadius::same(12), fill);
            let day = d.day().unwrap_or(now.date());
            ui.painter().text(
                rect.center() - vec2(0.0, 8.0),
                egui::Align2::CENTER_CENTER,
                weekday(lang, day),
                egui::FontId::proportional(12.0),
                fg,
            );
            ui.painter().text(
                rect.center() + vec2(0.0, 9.0),
                egui::Align2::CENTER_CENTER,
                day.format("%d.%m.").to_string(),
                fx::medium_font(14.0),
                fg,
            );
            ui.add_space(6.0);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    let title = ui.label(fx::title(&d.title, 15.0).color(p.on_surface));
                    // Where it was found, to check it was read right.
                    if !d.context.is_empty() {
                        title.on_hover_text(format!("«{}»", d.context));
                    }
                    if d.added > self.study.new_since && self.study.new_since > 0 {
                        badge(ui, lang.badge_new(), p.primary, p.on_primary);
                    }
                    if d.estimated {
                        badge(ui, lang.estimated(), p.warning_container, p.on_warning_container)
                            .on_hover_text(lang.estimated_hint());
                    }
                });
                let when = lang.due_when(days, late, d.time.as_deref());
                let kind = lang.deadline_kind(d.kind);
                let line = if d.module.is_empty() {
                    format!("{kind} · {when}")
                } else {
                    let short = d.module.rsplit(" › ").next().unwrap_or(&d.module);
                    format!("{kind} · {when} · {short}")
                };
                ui.label(RichText::new(line).small().color(if late { p.error } else { p.on_surface_variant }));
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                if fx::icon_button(ui, icon::TRASH).on_hover_text(lang.delete()).clicked() {
                    change = Some((d.id, "delete"));
                }
                if fx::icon_button(ui, icon::CALENDAR_PLUS).on_hover_text(lang.to_calendar()).clicked() {
                    change = Some((d.id, "calendar"));
                }
                if fx::icon_button(ui, icon::PENCIL_SIMPLE).on_hover_text(lang.correct()).clicked() {
                    change = Some((d.id, "edit"));
                }
                if let Some(file) = &d.file {
                    if fx::icon_button(ui, icon::FOLDER_OPEN).on_hover_text(lang.show_in_folder()).clicked() {
                        crate::desktop::reveal(file);
                    }
                    if fx::icon_button(ui, icon::ARROW_SQUARE_OUT).on_hover_text(lang.open_file()).clicked() {
                        crate::desktop::open(file);
                    }
                }
                ui.add_space(4.0);
                if fx::button(ui, ButtonStyle::Tonal, icon::CHECK, lang.done()).clicked() {
                    change = Some((d.id, "done"));
                }
            });
        });
        change
    }

    fn change_deadline(&mut self, id: u64, what: &str) {
        let lang = self.lang;
        let Some(d) = self.study.deadlines.items.iter_mut().find(|d| d.id == id) else { return };
        match what {
            "edit" => {
                let module = self.config.courses.iter().position(|c| c.label() == d.module).unwrap_or(usize::MAX);
                let time = d.time.as_deref().map(|t| format!(" {t}")).unwrap_or_default();
                let date = d.day().map(|day| format!("{}{time}", day.format("%d.%m.%Y"))).unwrap_or_default();
                self.study.new_deadline = Some(NewDeadline {
                    title: d.title.clone(),
                    module,
                    date,
                    kind: d.kind,
                    editing: Some(id),
                    file: None,
                });
                return;
            }
            "done" => d.done = true,
            "reopen" => d.done = false,
            "delete" => d.deleted = true,
            "calendar" => {
                let summary = if d.module.is_empty() {
                    lang.deadline_summary(&d.title, "")
                } else {
                    let short = d.module.rsplit(" › ").next().unwrap_or(&d.module).to_string();
                    lang.deadline_summary(&d.title, &short)
                };
                let text = deadlines::ics(d, &summary, lang.reminder_tomorrow());
                let file = std::env::temp_dir().join(format!("FileSorter-{}-{}.ics", d.id, d.date));
                match std::fs::write(&file, text) {
                    Ok(()) => crate::desktop::open(&file),
                    Err(e) => log::info!("Could not write the calendar file: {e}"),
                }
                return;
            }
            _ => return,
        }
        self.save_deadlines();
    }

    fn deadline_dialog(&mut self, ctx: &egui::Context) {
        let lang = self.lang;
        let Some(new) = &mut self.study.new_deadline else { return };
        let modules: Vec<String> = self.config.courses.iter().map(|c| c.label()).collect();
        let today = Local::now().date_naive();
        let parsed = deadlines::parse_typed(&new.date, today);
        let mut close = false;
        let mut add = false;
        let modal = egui::Modal::new(Id::new("deadline-dialog")).frame(dialog_frame(ctx)).show(ctx, |ui| {
            let p = fx::pal(ui);
            ui.set_width(400.0);
            let title = if new.editing.is_some() { lang.correct() } else { lang.add_deadline() };
            ui.label(fx::title(title, 22.0).color(p.on_surface));
            ui.add_space(12.0);
            let kinds = [
                (Kind::Due, lang.deadline_kind(Kind::Due).to_string()),
                (Kind::Exam, lang.deadline_kind(Kind::Exam).to_string()),
            ];
            fx::segmented(ui, "deadline-kind", &mut new.kind, &kinds);
            ui.add_space(8.0);
            ui.label(RichText::new(lang.what_is_due()).small().color(p.on_surface_variant));
            ui.add(
                egui::TextEdit::singleline(&mut new.title)
                    .hint_text(lang.what_is_due_hint())
                    .desired_width(f32::INFINITY)
                    .margin(Margin::symmetric(12, 8)),
            );
            ui.add_space(8.0);
            ui.label(RichText::new(lang.module_label()).small().color(p.on_surface_variant));
            let selected = modules.get(new.module).cloned().unwrap_or_else(|| lang.no_module().to_string());
            egui::ComboBox::from_id_salt("deadline-module").selected_text(selected).width(360.0).show_ui(ui, |ui| {
                ui.selectable_value(&mut new.module, usize::MAX, lang.no_module());
                for (i, m) in modules.iter().enumerate() {
                    ui.selectable_value(&mut new.module, i, m);
                }
            });
            ui.add_space(8.0);
            ui.label(RichText::new(lang.due_date()).small().color(p.on_surface_variant));
            ui.add(
                egui::TextEdit::singleline(&mut new.date)
                    .hint_text("24.10. 23:59")
                    .desired_width(f32::INFINITY)
                    .margin(Margin::symmetric(12, 8)),
            );
            let preview = match parsed {
                Some((date, time)) => {
                    let time = time.map(|t: NaiveTime| format!(", {}", t.format("%H:%M"))).unwrap_or_default();
                    (format!("{} {}{time}", weekday(lang, date), date.format("%d.%m.%Y")), p.on_surface_variant)
                }
                None if new.date.trim().is_empty() => (String::new(), p.on_surface_variant),
                None => (lang.date_not_understood().to_string(), p.error),
            };
            ui.label(RichText::new(preview.0).small().color(preview.1));
            actions(ui, |ui| {
                let ready = parsed.is_some() && !new.title.trim().is_empty();
                let label = if new.editing.is_some() { lang.save() } else { lang.add() };
                if ui.add_enabled_ui(ready, |ui| fx::button(ui, ButtonStyle::Filled, "", label)).inner.clicked() {
                    add = true;
                }
                if fx::button(ui, ButtonStyle::Text, "", lang.cancel()).clicked() {
                    close = true;
                }
            });
        });
        if add && let Some((date, time)) = parsed {
            let module = modules.get(new.module).cloned().unwrap_or_default();
            match new.editing {
                Some(id) => self.study.deadlines.edit(id, &new.title, &module, new.kind, date, time),
                None => {
                    let id = self.study.deadlines.add_manual(&new.title, &module, new.kind, date, time);
                    if let Some(item) = self.study.deadlines.items.iter_mut().find(|d| d.id == id) {
                        item.file = new.file.clone();
                    }
                }
            }
            self.study.new_deadline = None;
            self.save_deadlines();
        } else if close || modal.should_close() {
            self.study.new_deadline = None;
        }
    }

    fn weekly_ui(&mut self, ui: &mut egui::Ui) {
        let p = fx::pal(ui);
        let lang = self.lang;
        let total: usize = self.study.week.iter().map(|(_, f)| f.len()).sum();
        section_title(ui, &lang.new_this_week(total));
        let mut open = None;
        let mut reveal = None;
        fx::card(ui, |ui| {
            if self.study.week.is_empty() {
                ui.label(RichText::new(lang.nothing_new_this_week()).color(p.on_surface_variant));
                return;
            }
            for (i, (module, files)) in self.study.week.iter().enumerate() {
                if i > 0 {
                    super::settings::divider(ui);
                }
                let header = egui::CollapsingHeader::new(
                    RichText::new(format!("{module}  ·  {}", files.len())).family(fx::medium()).color(p.on_surface),
                )
                .id_salt(("week-module", module))
                .default_open(i < 3);
                header.show(ui, |ui| {
                    for (path, time) in files {
                        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                        ui.horizontal(|ui| {
                            let text = format!("{}  {name}", icon::FILE_TEXT);
                            let link =
                                ui.add(egui::Label::new(RichText::new(text).color(p.primary)).sense(Sense::click()));
                            if link
                                .on_hover_cursor(egui::CursorIcon::PointingHand)
                                .on_hover_text(lang.open_file())
                                .clicked()
                            {
                                open = Some(path.clone());
                            }
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                if fx::icon_button(ui, icon::FOLDER_OPEN).on_hover_text(lang.show_in_folder()).clicked()
                                {
                                    reveal = Some(path.clone());
                                }
                                if fx::icon_button(ui, icon::ARROW_SQUARE_OUT).on_hover_text(lang.open_file()).clicked()
                                {
                                    open = Some(path.clone());
                                }
                                let when = format!("{} {}", weekday(lang, time.date_naive()), time.format("%H:%M"));
                                ui.label(RichText::new(when).small().color(p.on_surface_variant));
                            });
                        });
                    }
                });
            }
        });
        if let Some(path) = open {
            crate::desktop::open(&path);
        }
        if let Some(path) = reveal {
            crate::desktop::reveal(&path);
        }
    }
}

/// The snippet with the search words in bold, highlighted.
fn highlighted(ui: &egui::Ui, text: &str, marks: &[std::ops::Range<usize>]) -> LayoutJob {
    let p = fx::pal(ui);
    let plain = egui::TextFormat { color: p.on_surface_variant, ..Default::default() };
    let mark = egui::TextFormat {
        color: p.on_surface,
        background: p.primary.gamma_multiply(0.25),
        font_id: fx::medium_font(14.0),
        ..Default::default()
    };
    let mut job = LayoutJob::default();
    let mut at = 0;
    for range in marks {
        if range.start < at || range.end > text.len() {
            continue;
        }
        job.append(&text[at..range.start], 0.0, plain.clone());
        job.append(&text[range.clone()], 0.0, mark.clone());
        at = range.end;
    }
    job.append(&text[at..], 0.0, plain);
    job
}
