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
use crate::deadlines::{self, DEADLINES_FILE, Deadlines, Found, Kind};
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
}

/// Documents changed in the last this many days are looked through for dates.
const SCAN_DAYS: i64 = 150;

pub(super) struct Study {
    indexer: Option<Indexer>,
    /// The folder the index was last told to cover.
    indexed: Option<(PathBuf, Vec<String>)>,
    pub(super) query: String,
    searched: String,
    hits: Vec<Hit>,
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
        if features.search && !self.config.destination_root.trim().is_empty() {
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
        // Search as you type, once the index answers.
        if self.study.query != self.study.searched {
            self.run_search();
        }
    }

    fn run_search(&mut self) {
        self.study.searched = self.study.query.clone();
        self.study.hits = match &self.study.indexer {
            Some(indexer) => indexer.index.read().map(|i| i.search(&self.study.query, 60)).unwrap_or_default(),
            None => Vec::new(),
        };
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
            let found = deadlines::find(&crate::extract::extract_text(&file), Local::now().date_naive());
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
            Ok(index) => index.texts(since, 12, 200, |path, size| deadlines.was_scanned(path, size)),
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
        ui.add_space(8.0);
        let mut open = None;
        let mut reveal = None;
        egui::ScrollArea::vertical().id_salt("search-results").auto_shrink([false, false]).show(ui, |ui| {
            if self.study.hits.is_empty() {
                ui.add_space(24.0);
                ui.vertical_centered(|ui| {
                    ui.label(RichText::new(icon::MAGNIFYING_GLASS).size(34.0).color(p.outline));
                    ui.label(RichText::new(lang.no_results()).color(p.on_surface_variant));
                });
                return;
            }
            for hit in &self.study.hits {
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
                });
                let click = ui.interact(response.rect, Id::new(("search-hit", &hit.path)), Sense::click());
                if click.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(lang.open_file()).clicked() {
                    open = Some(hit.path.clone());
                }
                ui.add_space(6.0);
            }
        });
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
        let width = ui.available_width().min(860.0);
        let margin = ((ui.available_width() - width) / 2.0).max(0.0);
        egui::ScrollArea::vertical().id_salt("week").auto_shrink([false, false]).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.add_space(margin);
                ui.vertical(|ui| {
                    ui.set_width(width);
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

    fn deadlines_ui(&mut self, ui: &mut egui::Ui) {
        let p = fx::pal(ui);
        let lang = self.lang;
        ui.horizontal(|ui| {
            section_title(ui, lang.due());
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if fx::button(ui, ButtonStyle::Tonal, icon::PLUS, lang.add_deadline()).clicked() {
                    self.study.new_deadline = Some(NewDeadline {
                        title: String::new(),
                        module: 0,
                        date: String::new(),
                        kind: Kind::Due,
                        editing: None,
                    });
                }
                let scanning = self.study.scanning.load(std::sync::atomic::Ordering::Relaxed);
                if scanning {
                    ui.add(egui::Spinner::new());
                } else if fx::button(ui, ButtonStyle::Text, icon::ARROW_CLOCKWISE, lang.search_again())
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
        let now = Local::now().naive_local();
        let upcoming: Vec<_> = self.study.deadlines.upcoming(now, 7, 366).into_iter().cloned().collect();
        let mut change: Option<(u64, &str)> = None;
        fx::card(ui, |ui| {
            if upcoming.is_empty() {
                ui.label(RichText::new(lang.no_deadlines()).color(p.on_surface_variant));
                return;
            }
            for (i, d) in upcoming.iter().enumerate() {
                if i > 0 {
                    super::settings::divider(ui);
                }
                let Some(due) = d.due() else { continue };
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
                    ui.set_min_height(52.0);
                    // The day in a tinted box.
                    let (rect, _) = ui.allocate_exact_size(vec2(64.0, 46.0), Sense::hover());
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
                        fx::medium_font(15.0),
                        fg,
                    );
                    ui.add_space(8.0);
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 2.0;
                        ui.label(fx::title(&d.title, 15.0).color(p.on_surface));
                        let when = lang.due_when(days, late, d.time.as_deref());
                        let kind = lang.deadline_kind(d.kind);
                        let line = if d.module.is_empty() {
                            format!("{kind} · {when}")
                        } else {
                            format!("{kind} · {when} · {}", d.module)
                        };
                        ui.label(RichText::new(line).color(if late { p.error } else { p.on_surface_variant }));
                        if !d.context.is_empty() {
                            ui.add(
                                egui::Label::new(RichText::new(format!("«{}»", d.context)).small().color(p.outline))
                                    .truncate(),
                            );
                        }
                    });
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if fx::button(ui, ButtonStyle::Text, icon::TRASH, "").on_hover_text(lang.delete()).clicked() {
                            change = Some((d.id, "delete"));
                        }
                        if fx::button(ui, ButtonStyle::Text, icon::CALENDAR_PLUS, "")
                            .on_hover_text(lang.to_calendar())
                            .clicked()
                        {
                            change = Some((d.id, "calendar"));
                        }
                        if fx::icon_button(ui, icon::PENCIL_SIMPLE).on_hover_text(lang.correct()).clicked() {
                            change = Some((d.id, "edit"));
                        }
                        if let Some(file) = &d.file
                            && fx::icon_button(ui, icon::FOLDER_OPEN).on_hover_text(lang.show_in_folder()).clicked()
                        {
                            crate::desktop::reveal(file);
                        }
                        if let Some(file) = &d.file
                            && fx::icon_button(ui, icon::ARROW_SQUARE_OUT).on_hover_text(lang.open_file()).clicked()
                        {
                            crate::desktop::open(file);
                        }
                        if fx::button(ui, ButtonStyle::Tonal, icon::CHECK, lang.done()).clicked() {
                            change = Some((d.id, "done"));
                        }
                    });
                });
            }
        });
        ui.add_space(4.0);
        ui.label(RichText::new(lang.deadlines_note()).small().color(p.outline));
        if let Some((id, what)) = change {
            self.change_deadline(id, what);
        }
    }

    fn change_deadline(&mut self, id: u64, what: &str) {
        let lang = self.lang;
        let Some(d) = self.study.deadlines.items.iter_mut().find(|d| d.id == id) else { return };
        match what {
            "edit" => {
                let module = self.config.courses.iter().position(|c| c.label() == d.module).unwrap_or(usize::MAX);
                let time = d.time.as_deref().map(|t| format!(" {t}")).unwrap_or_default();
                let date = d.day().map(|day| format!("{}{time}", day.format("%d.%m.%Y"))).unwrap_or_default();
                self.study.new_deadline =
                    Some(NewDeadline { title: d.title.clone(), module, date, kind: d.kind, editing: Some(id) });
                return;
            }
            "done" => d.done = true,
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
                    self.study.deadlines.add_manual(&new.title, &module, new.kind, date, time);
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
