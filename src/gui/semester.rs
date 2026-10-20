//! The «Semester» tab: the timetable as a week grid (imported from a calendar file or link,
//! or entered by hand), and the grades with ECTS.

use std::sync::mpsc::{Receiver, channel};

use chrono::{Datelike, Duration, Local, NaiveDate, NaiveTime, Timelike};
use eframe::egui::{self, Align, Align2, CornerRadius, Id, Layout, Margin, Pos2, Rect, RichText, Sense, Stroke, vec2};
use egui_phosphor::regular as icon;

use super::App;
use super::dialogs::{actions, dialog_frame};
use crate::fx::{self, ButtonStyle};
use crate::timetable::{self, Lesson, Subscription, TIMETABLE_FILE, Timetable, day_text};

/// A calendar link is read again after this long.
const REFRESH_LINK: i64 = 24 * 60 * 60;
const HOUR_HEIGHT: f32 = 46.0;
const GUTTER: f32 = 46.0;

/// A calendar link being read: the link, whether it was read by itself, and the answer.
type Fetching = (String, bool, Receiver<Result<String, String>>);

/// A lesson being added or changed.
pub(super) struct LessonEdit {
    lesson: Lesson,
    /// The date that was clicked (to cancel just that one).
    date: Option<NaiveDate>,
    new: bool,
    from: String,
    until: String,
}

/// Lessons read from a calendar, waiting for «Import».
pub(super) struct Import {
    source: String,
    lessons: Vec<Lesson>,
    /// Read from a link: keep reading it every day.
    subscribe: bool,
}

pub(super) struct Semester {
    pub(super) table: Timetable,
    /// The Monday of the week shown.
    week: NaiveDate,
    edit: Option<LessonEdit>,
    import: Option<Import>,
    /// «Calendar link…»: the link typed so far.
    link: Option<String>,
    /// Reading a link: (link, whether by itself, the text or why not).
    fetching: Option<Fetching>,
    error: Option<String>,
}

fn monday_of(date: NaiveDate) -> NaiveDate {
    date - Duration::days(date.weekday().num_days_from_monday() as i64)
}

fn parse_day(text: &str) -> Option<NaiveDate> {
    crate::deadlines::parse_typed(text, Local::now().date_naive()).map(|(d, _)| d)
}

fn shown_day(text: &str) -> String {
    NaiveDate::parse_from_str(text, "%Y-%m-%d").map(|d| d.format("%d.%m.%Y").to_string()).unwrap_or_default()
}

impl Semester {
    /// The module whose lesson is running now, if one is.
    pub(super) fn running_now(&self) -> Option<String> {
        let now = Local::now().naive_local();
        self.table.lessons.iter().find(|l| !l.module.is_empty() && l.running(now)).map(|l| l.module.clone())
    }

    pub(super) fn new(state_dir: Option<&std::path::Path>) -> Semester {
        Semester {
            table: state_dir.map(|d| Timetable::load(&d.join(TIMETABLE_FILE))).unwrap_or_default(),
            week: monday_of(Local::now().date_naive()),
            edit: None,
            import: None,
            link: None,
            fetching: None,
            error: None,
        }
    }
}

/// Read a calendar link (`webcal://` is `https://`).
fn fetch(url: &str) -> Result<String, String> {
    let url = url.trim().replacen("webcal://", "https://", 1);
    if !url.starts_with("https://") && !url.starts_with("http://") {
        return Err("not a web link".into());
    }
    let output = crate::update::curl().arg(&url).output().map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    let text = String::from_utf8_lossy(&output.stdout).into_owned();
    if !text.contains("BEGIN:VCALENDAR") {
        return Err("no calendar at this link".into());
    }
    Ok(text)
}

impl App {
    fn save_timetable(&self) {
        if let Some(file) = self.config.state_file(TIMETABLE_FILE)
            && let Err(e) = self.semester.table.save(&file)
        {
            log::info!("Could not save the timetable: {e}");
        }
    }

    /// Read calendar links again once a day; take in what was read.
    pub(super) fn semester_logic(&mut self) {
        if let Some((url, by_itself, rx)) = &self.semester.fetching
            && let Ok(result) = rx.try_recv()
        {
            let (url, by_itself) = (url.clone(), *by_itself);
            self.semester.fetching = None;
            match result {
                Ok(text) => {
                    let lessons = self.prepare_lessons(&url, &text);
                    if by_itself {
                        self.semester.table.replace_source(&url, lessons);
                        if let Some(s) = self.semester.table.subscriptions.iter_mut().find(|s| s.url == url) {
                            s.read = chrono::Utc::now().timestamp();
                        }
                        self.save_timetable();
                    } else {
                        self.semester.import = Some(Import { source: url, lessons, subscribe: true });
                    }
                }
                Err(e) if by_itself => log::info!("Could not read the calendar link {url}: {e}"),
                Err(e) => self.semester.error = Some(e),
            }
        }
        if self.semester.fetching.is_none() {
            let now = chrono::Utc::now().timestamp();
            if let Some(url) =
                self.semester.table.subscriptions.iter().find(|s| now - s.read > REFRESH_LINK).map(|s| s.url.clone())
            {
                self.start_fetch(url, true);
            }
        }
    }

    fn start_fetch(&mut self, url: String, by_itself: bool) {
        let (tx, rx) = channel();
        let (link, ctx) = (url.clone(), self.ctx.clone());
        std::thread::spawn(move || {
            let _ = tx.send(fetch(&link));
            ctx.request_repaint();
        });
        if by_itself && let Some(s) = self.semester.table.subscriptions.iter_mut().find(|s| s.url == url) {
            // Not again right away if it fails.
            s.read = chrono::Utc::now().timestamp();
        }
        self.semester.fetching = Some((url, by_itself, rx));
    }

    /// Lessons read from a calendar, with module and kind guessed; what the user set for
    /// the same title before is kept.
    fn prepare_lessons(&self, source: &str, text: &str) -> Vec<Lesson> {
        let before: Vec<&Lesson> = self.semester.table.lessons.iter().filter(|l| l.source == source).collect();
        let mut lessons = timetable::read_ics(text);
        for lesson in &mut lessons {
            match before.iter().find(|b| b.title == lesson.title) {
                Some(b) => {
                    lesson.module = b.module.clone();
                    lesson.kind = b.kind.clone();
                }
                None => {
                    lesson.module = timetable::guess_module(&self.config, &lesson.title).unwrap_or_default();
                    lesson.kind = timetable::guess_kind(&lesson.title);
                }
            }
        }
        lessons.sort_by(|a, b| (a.weekday, &a.start).cmp(&(b.weekday, &b.start)));
        lessons
    }

    fn import_file(&mut self) {
        let Some(file) = rfd::FileDialog::new().add_filter("iCalendar", &["ics", "ical"]).pick_file() else { return };
        match std::fs::read_to_string(&file) {
            Ok(text) => {
                let source = file.display().to_string();
                let lessons = self.prepare_lessons(&source, &text);
                if lessons.is_empty() {
                    self.semester.error = Some(self.lang.no_lessons_found().into());
                } else {
                    self.semester.import = Some(Import { source, lessons, subscribe: false });
                }
            }
            Err(e) => self.semester.error = Some(e.to_string()),
        }
    }

    pub(super) fn semester_ui(&mut self, ui: &mut egui::Ui) {
        let width = ui.available_width().min(1040.0);
        let margin = ((ui.available_width() - width) / 2.0).max(0.0);
        egui::ScrollArea::vertical().id_salt("semester").auto_shrink([false, false]).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.add_space(margin);
                ui.vertical(|ui| {
                    ui.set_width(width);
                    self.timetable_ui(ui);
                    ui.add_space(18.0);
                    self.grades_ui(ui);
                    ui.add_space(24.0);
                });
            });
        });
        let ctx = ui.ctx().clone();
        self.lesson_dialog(&ctx);
        self.import_dialog(&ctx);
        self.link_dialog(&ctx);
    }

    fn timetable_ui(&mut self, ui: &mut egui::Ui) {
        let p = fx::pal(ui);
        let lang = self.lang;
        let today = Local::now().date_naive();
        let week = self.semester.week;
        ui.horizontal(|ui| {
            ui.label(fx::title(lang.timetable(), 20.0).color(p.on_surface));
            ui.add_space(8.0);
            if fx::icon_button(ui, icon::CARET_LEFT).on_hover_text(lang.previous_week()).clicked() {
                self.semester.week = week - Duration::weeks(1);
            }
            let label = format!(
                "{} {} · {} – {}",
                lang.calendar_week(),
                week.iso_week().week(),
                week.format("%d.%m."),
                (week + Duration::days(6)).format("%d.%m.%Y")
            );
            ui.label(RichText::new(label).color(p.on_surface_variant));
            if fx::icon_button(ui, icon::CARET_RIGHT).on_hover_text(lang.next_week()).clicked() {
                self.semester.week = week + Duration::weeks(1);
            }
            if week != monday_of(today) && fx::button(ui, ButtonStyle::Text, "", lang.today()).clicked() {
                self.semester.week = monday_of(today);
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                if fx::button(ui, ButtonStyle::Tonal, icon::PLUS, lang.add_lesson()).clicked() {
                    let date = today;
                    self.semester.edit = Some(LessonEdit {
                        lesson: Lesson {
                            weekday: date.weekday().num_days_from_monday(),
                            start: "10:15".into(),
                            end: "12:00".into(),
                            every: 1,
                            ..Default::default()
                        },
                        date: None,
                        new: true,
                        from: date.format("%d.%m.%Y").to_string(),
                        until: (date + Duration::days(120)).format("%d.%m.%Y").to_string(),
                    });
                }
                if fx::button(ui, ButtonStyle::Text, icon::LINK, lang.calendar_link())
                    .on_hover_text(lang.calendar_link_hint())
                    .clicked()
                {
                    self.semester.link = Some(String::new());
                }
                if fx::button(ui, ButtonStyle::Text, icon::CALENDAR_PLUS, lang.import_calendar())
                    .on_hover_text(lang.import_calendar_hint())
                    .clicked()
                {
                    self.import_file();
                }
                if self.semester.fetching.as_ref().is_some_and(|(_, by_itself, _)| !by_itself) {
                    ui.add(egui::Spinner::new());
                }
            });
        });
        if let Some(error) = &self.semester.error {
            let mut close = false;
            ui.horizontal(|ui| {
                ui.label(RichText::new(format!("{}  {error}", icon::WARNING)).color(p.error));
                close = fx::icon_button(ui, icon::X).clicked();
            });
            if close {
                self.semester.error = None;
            }
        }
        ui.add_space(6.0);
        fx::card(ui, |ui| {
            if self.semester.table.lessons.is_empty() {
                ui.add(egui::Label::new(RichText::new(lang.timetable_empty()).color(p.on_surface_variant)).wrap());
                return;
            }
            self.week_grid(ui, week, today);
        });
        if !self.semester.table.subscriptions.is_empty() {
            let links: Vec<String> = self.semester.table.subscriptions.iter().map(|s| s.url.clone()).collect();
            ui.label(RichText::new(lang.subscribed(&links.join(", "))).small().color(p.outline));
        }
        ui.label(RichText::new(lang.timetable_sorting_note()).small().color(p.outline));
    }

    fn week_grid(&mut self, ui: &mut egui::Ui, week: NaiveDate, today: NaiveDate) {
        let p = fx::pal(ui);
        let lang = self.lang;
        let dark = ui.visuals().dark_mode;
        let table = &self.semester.table;
        let days: Vec<NaiveDate> = (0..7).map(|i| week + Duration::days(i)).collect();
        // Saturday and Sunday only if something is on then.
        let weekend = days[5..].iter().any(|d| table.lessons.iter().any(|l| l.on(*d)));
        let shown = if weekend { 7 } else { 5 };
        let lessons: Vec<(usize, Lesson, bool)> = days[..shown]
            .iter()
            .enumerate()
            .flat_map(|(i, d)| {
                table
                    .lessons
                    .iter()
                    .filter(move |l| {
                        let mut all = (*l).clone();
                        all.skipped.clear();
                        all.on(*d)
                    })
                    .map(move |l| (i, l.clone(), !l.on(*d)))
            })
            .collect();
        let hour = |t: Option<NaiveTime>| t.map(|t| t.hour() as f32 + t.minute() as f32 / 60.0);
        let first = lessons.iter().filter_map(|(_, l, _)| hour(l.start_time())).fold(8.0_f32, f32::min).floor();
        let last = lessons.iter().filter_map(|(_, l, _)| hour(l.end_time())).fold(18.0_f32, f32::max).ceil();
        let header = 30.0;
        let size = vec2(ui.available_width(), header + (last - first) * HOUR_HEIGHT);
        let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
        let column = (rect.width() - GUTTER) / shown as f32;
        let painter = ui.painter_at(rect);
        let y_of = |h: f32| rect.top() + header + (h - first) * HOUR_HEIGHT;
        // Hours.
        for h in first as u32..=last as u32 {
            let y = y_of(h as f32);
            painter.line_segment(
                [Pos2::new(rect.left() + GUTTER, y), Pos2::new(rect.right(), y)],
                Stroke::new(1.0, p.outline_variant),
            );
            painter.text(
                Pos2::new(rect.left() + GUTTER - 8.0, y),
                Align2::RIGHT_CENTER,
                format!("{h}:00"),
                egui::FontId::proportional(11.0),
                p.on_surface_variant,
            );
        }
        // Days.
        for (i, d) in days[..shown].iter().enumerate() {
            let x = rect.left() + GUTTER + i as f32 * column;
            let head = Rect::from_min_size(Pos2::new(x + 2.0, rect.top()), vec2(column - 4.0, header - 4.0));
            let is_today = *d == today;
            if is_today {
                painter.rect_filled(head, CornerRadius::same(8), p.primary_container);
            }
            let text = format!("{} {}", lang.weekday_short(i as u32), d.format("%d.%m."));
            let color = if is_today { p.on_primary_container } else { p.on_surface_variant };
            painter.text(head.center(), Align2::CENTER_CENTER, text, fx::medium_font(12.5), color);
        }
        // Lessons.
        let now = Local::now().naive_local();
        let mut clicked = None;
        for (i, lesson, cancelled) in &lessons {
            let (Some(start), Some(end)) = (hour(lesson.start_time()), hour(lesson.end_time())) else { continue };
            let x = rect.left() + GUTTER + *i as f32 * column;
            let block =
                Rect::from_min_max(Pos2::new(x + 3.0, y_of(start) + 1.0), Pos2::new(x + column - 3.0, y_of(end) - 1.0));
            let course = self.config.courses.iter().position(|c| c.label() == lesson.module);
            let (fill, fg) = match course {
                Some(c) => fx::avatar_colors(dark, c),
                None => (p.container_highest, p.on_surface),
            };
            let (fill, fg) = if *cancelled { (fill.gamma_multiply(0.35), fg.gamma_multiply(0.6)) } else { (fill, fg) };
            painter.rect_filled(block, CornerRadius::same(8), fill);
            let date = days[*i];
            if !cancelled && lesson.running(now) {
                painter.rect_stroke(
                    block,
                    CornerRadius::same(8),
                    Stroke::new(2.0, p.primary),
                    egui::StrokeKind::Inside,
                );
            }
            let name = match course {
                Some(c) => self.config.courses[c].name.clone(),
                None => lesson.title.clone(),
            };
            let mut lines = vec![(name, fx::medium_font(12.5))];
            let detail = if *cancelled {
                lang.cancelled().to_string()
            } else {
                [format!("{}–{}", lesson.start, lesson.end), lesson.room.clone(), lesson.kind.clone()]
                    .into_iter()
                    .filter(|s| !s.is_empty())
                    .collect::<Vec<_>>()
                    .join(" · ")
            };
            lines.push((detail, egui::FontId::proportional(11.0)));
            let mut y = block.top() + 4.0;
            for (text, font) in lines {
                let galley = painter.layout(text, font, fg, block.width() - 10.0);
                if y + galley.size().y > block.bottom() {
                    break;
                }
                let height = galley.size().y;
                painter.galley(Pos2::new(block.left() + 6.0, y), galley, fg);
                y += height;
            }
            let response = ui
                .interact(block, Id::new(("lesson", lesson.id, date)), Sense::click())
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .on_hover_text(format!("{}\n{}–{} {}", lesson.title, lesson.start, lesson.end, lesson.room));
            if response.clicked() {
                clicked = Some((lesson.clone(), date));
            }
        }
        // Now.
        if let Some(i) = days[..shown].iter().position(|d| *d == today) {
            let h = now.hour() as f32 + now.minute() as f32 / 60.0;
            if (first..=last).contains(&h) {
                let x = rect.left() + GUTTER + i as f32 * column;
                let y = y_of(h);
                painter.line_segment([Pos2::new(x, y), Pos2::new(x + column, y)], Stroke::new(2.0, p.error));
                painter.circle_filled(Pos2::new(x, y), 4.0, p.error);
            }
        }
        if let Some((lesson, date)) = clicked {
            self.semester.edit = Some(LessonEdit {
                from: shown_day(&lesson.from),
                until: shown_day(&lesson.until),
                lesson,
                date: Some(date),
                new: false,
            });
        }
    }

    fn lesson_dialog(&mut self, ctx: &egui::Context) {
        let lang = self.lang;
        let modules: Vec<String> = self.config.courses.iter().map(|c| c.label()).collect();
        let Some(edit) = &mut self.semester.edit else { return };
        let mut save = false;
        let mut delete = false;
        let mut close = false;
        let mut cancel_date = None;
        let mut open_folder = None;
        let (from, until) = (parse_day(&edit.from), parse_day(&edit.until));
        let times_ok = edit.lesson.start_time().is_some()
            && edit.lesson.end_time().is_some()
            && edit.lesson.start_time() < edit.lesson.end_time();
        let ready = times_ok && from.is_some() && until.is_some() && from <= until;
        let modal = egui::Modal::new(Id::new("lesson-dialog")).frame(dialog_frame(ctx)).show(ctx, |ui| {
            let p = fx::pal(ui);
            ui.set_width(440.0);
            let title = if edit.new { lang.add_lesson() } else { lang.lesson() };
            ui.label(fx::title(title, 22.0).color(p.on_surface));
            ui.add_space(10.0);
            let field = |ui: &mut egui::Ui, label: &str, value: &mut String, hint: &str| {
                ui.label(RichText::new(label).small().color(p.on_surface_variant));
                ui.add(
                    egui::TextEdit::singleline(value)
                        .hint_text(hint)
                        .desired_width(f32::INFINITY)
                        .margin(Margin::symmetric(10, 6)),
                );
            };
            field(ui, lang.lesson_title(), &mut edit.lesson.title, "Statistik I – Vorlesung");
            ui.label(RichText::new(lang.module_label()).small().color(p.on_surface_variant));
            let shown =
                if edit.lesson.module.is_empty() { lang.no_module().to_string() } else { edit.lesson.module.clone() };
            egui::ComboBox::from_id_salt("lesson-module").selected_text(shown).width(400.0).show_ui(ui, |ui| {
                ui.selectable_value(&mut edit.lesson.module, String::new(), lang.no_module());
                for m in &modules {
                    ui.selectable_value(&mut edit.lesson.module, m.clone(), m);
                }
            });
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.label(RichText::new(lang.weekday_label()).small().color(p.on_surface_variant));
                    egui::ComboBox::from_id_salt("lesson-day")
                        .selected_text(lang.weekday_long(edit.lesson.weekday))
                        .width(130.0)
                        .show_ui(ui, |ui| {
                            for d in 0..7 {
                                ui.selectable_value(&mut edit.lesson.weekday, d, lang.weekday_long(d));
                            }
                        });
                });
                ui.vertical(|ui| {
                    ui.label(RichText::new(lang.time_from_to()).small().color(p.on_surface_variant));
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut edit.lesson.start)
                                .desired_width(56.0)
                                .margin(Margin::symmetric(8, 6)),
                        );
                        ui.label("–");
                        ui.add(
                            egui::TextEdit::singleline(&mut edit.lesson.end)
                                .desired_width(56.0)
                                .margin(Margin::symmetric(8, 6)),
                        );
                    });
                });
                ui.vertical(|ui| {
                    ui.label(RichText::new(lang.every_label()).small().color(p.on_surface_variant));
                    egui::ComboBox::from_id_salt("lesson-every")
                        .selected_text(lang.every_weeks(edit.lesson.every))
                        .width(110.0)
                        .show_ui(ui, |ui| {
                            for n in [1, 2] {
                                ui.selectable_value(&mut edit.lesson.every, n, lang.every_weeks(n));
                            }
                        });
                });
            });
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.set_width(200.0);
                    field(ui, lang.lesson_room(), &mut edit.lesson.room, "HG F 1");
                });
                ui.vertical(|ui| field(ui, lang.lesson_kind(), &mut edit.lesson.kind, "Vorlesung"));
            });
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.set_width(200.0);
                    field(ui, lang.first_day(), &mut edit.from, "20.10.2026");
                });
                ui.vertical(|ui| field(ui, lang.last_day(), &mut edit.until, "22.12.2026"));
            });
            if !ready {
                ui.label(RichText::new(lang.lesson_not_ready()).small().color(p.error));
            }
            ui.add_space(6.0);
            ui.horizontal_wrapped(|ui| {
                if let Some(date) = edit.date {
                    let cancelled = edit.lesson.skipped.contains(&day_text(date));
                    let text = if cancelled { lang.takes_place(date) } else { lang.cancel_on(date) };
                    if fx::button(ui, ButtonStyle::Text, icon::CALENDAR_X, &text).clicked() {
                        cancel_date = Some(date);
                    }
                }
                if let Some(course) = self.config.courses.iter().find(|c| c.label() == edit.lesson.module)
                    && fx::button(ui, ButtonStyle::Text, icon::FOLDER_OPEN, lang.open_module_folder()).clicked()
                {
                    open_folder = Some(self.config.course_folder(course));
                }
            });
            actions(ui, |ui| {
                let label = if edit.new { lang.add() } else { lang.save() };
                if ui.add_enabled_ui(ready, |ui| fx::button(ui, ButtonStyle::Filled, "", label)).inner.clicked() {
                    save = true;
                }
                if fx::button(ui, ButtonStyle::Text, "", lang.cancel()).clicked() {
                    close = true;
                }
                if !edit.new && fx::button(ui, ButtonStyle::Text, icon::TRASH, lang.delete()).clicked() {
                    delete = true;
                }
            });
        });
        if let Some(dir) = open_folder {
            crate::desktop::open(&dir);
        }
        if let Some(date) = cancel_date {
            let key = day_text(date);
            let id = edit.lesson.id;
            if let Some(l) = self.semester.table.lessons.iter_mut().find(|l| l.id == id) {
                if l.skipped.contains(&key) {
                    l.skipped.retain(|d| *d != key);
                } else {
                    l.skipped.push(key);
                }
            }
            self.semester.edit = None;
            self.save_timetable();
            return;
        }
        if save && let (Some(from), Some(until)) = (from, until) {
            let mut lesson = edit.lesson.clone();
            lesson.from = day_text(from);
            lesson.until = day_text(until);
            lesson.every = lesson.every.max(1);
            if edit.new {
                lesson.id = self.semester.table.next_id();
                self.semester.table.lessons.push(lesson);
            } else if let Some(l) = self.semester.table.lessons.iter_mut().find(|l| l.id == lesson.id) {
                *l = lesson;
            }
            self.semester.edit = None;
            self.save_timetable();
        } else if delete {
            let id = edit.lesson.id;
            self.semester.table.lessons.retain(|l| l.id != id);
            self.semester.edit = None;
            self.save_timetable();
        } else if close || modal.should_close() {
            self.semester.edit = None;
        }
    }

    fn import_dialog(&mut self, ctx: &egui::Context) {
        let lang = self.lang;
        let modules: Vec<String> = self.config.courses.iter().map(|c| c.label()).collect();
        let Some(import) = &mut self.semester.import else { return };
        let mut take = false;
        let mut close = false;
        let modal = egui::Modal::new(Id::new("import-dialog")).frame(dialog_frame(ctx)).show(ctx, |ui| {
            let p = fx::pal(ui);
            ui.set_width(620.0);
            ui.label(fx::title(lang.import_calendar(), 22.0).color(p.on_surface));
            ui.add_space(6.0);
            ui.add(
                egui::Label::new(
                    RichText::new(lang.import_explained(import.lessons.len())).color(p.on_surface_variant),
                )
                .wrap(),
            );
            ui.add_space(8.0);
            egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
                egui::Grid::new("import-lessons").num_columns(3).spacing(vec2(12.0, 6.0)).show(ui, |ui| {
                    for (k, lesson) in import.lessons.iter_mut().enumerate() {
                        ui.label(
                            RichText::new(format!(
                                "{} {}–{}",
                                lang.weekday_short(lesson.weekday),
                                lesson.start,
                                lesson.end
                            ))
                            .color(p.on_surface_variant),
                        );
                        ui.add(egui::Label::new(RichText::new(&lesson.title).color(p.on_surface)).truncate());
                        let shown =
                            if lesson.module.is_empty() { lang.no_module().to_string() } else { lesson.module.clone() };
                        egui::ComboBox::from_id_salt(("import-module", k)).selected_text(shown).width(210.0).show_ui(
                            ui,
                            |ui| {
                                ui.selectable_value(&mut lesson.module, String::new(), lang.no_module());
                                for m in &modules {
                                    ui.selectable_value(&mut lesson.module, m.clone(), m);
                                }
                            },
                        );
                        ui.end_row();
                    }
                });
            });
            actions(ui, |ui| {
                if fx::button(ui, ButtonStyle::Filled, "", lang.import_now()).clicked() {
                    take = true;
                }
                if fx::button(ui, ButtonStyle::Text, "", lang.cancel()).clicked() {
                    close = true;
                }
            });
        });
        if take {
            let import = self.semester.import.take().expect("shown");
            if import.subscribe && !self.semester.table.subscriptions.iter().any(|s| s.url == import.source) {
                self.semester
                    .table
                    .subscriptions
                    .push(Subscription { url: import.source.clone(), read: chrono::Utc::now().timestamp() });
            }
            self.semester.table.replace_source(&import.source, import.lessons);
            self.save_timetable();
        } else if close || modal.should_close() {
            self.semester.import = None;
        }
    }

    fn link_dialog(&mut self, ctx: &egui::Context) {
        let lang = self.lang;
        let Some(mut link) = self.semester.link.take() else { return };
        let mut read = false;
        let mut close = false;
        let mut forget: Option<String> = None;
        let subscribed: Vec<String> = self.semester.table.subscriptions.iter().map(|s| s.url.clone()).collect();
        let modal = egui::Modal::new(Id::new("link-dialog")).frame(dialog_frame(ctx)).show(ctx, |ui| {
            let p = fx::pal(ui);
            ui.set_width(520.0);
            ui.label(fx::title(lang.calendar_link(), 22.0).color(p.on_surface));
            ui.add_space(6.0);
            ui.add(egui::Label::new(RichText::new(lang.calendar_link_explained()).color(p.on_surface_variant)).wrap());
            ui.add_space(8.0);
            ui.add(
                egui::TextEdit::singleline(&mut link)
                    .hint_text("https://… .ics / webcal://…")
                    .desired_width(f32::INFINITY)
                    .margin(Margin::symmetric(10, 8)),
            );
            for url in &subscribed {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(format!("{}  {url}", icon::LINK)).small().color(p.on_surface_variant));
                    if fx::icon_button(ui, icon::TRASH).on_hover_text(lang.unsubscribe()).clicked() {
                        forget = Some(url.clone());
                    }
                });
            }
            actions(ui, |ui| {
                if ui
                    .add_enabled_ui(!link.trim().is_empty(), |ui| {
                        fx::button(ui, ButtonStyle::Filled, "", lang.read_link())
                    })
                    .inner
                    .clicked()
                {
                    read = true;
                }
                if fx::button(ui, ButtonStyle::Text, "", lang.cancel()).clicked() {
                    close = true;
                }
            });
        });
        if let Some(url) = forget {
            self.semester.table.subscriptions.retain(|s| s.url != url);
            self.semester.table.lessons.retain(|l| l.source != url);
            self.save_timetable();
        }
        if read {
            self.start_fetch(link.trim().to_string(), false);
        } else if !(close || modal.should_close()) {
            self.semester.link = Some(link);
        }
    }

    /// Today's lessons, for the top of «This week».
    pub(super) fn today_lessons_ui(&mut self, ui: &mut egui::Ui) {
        let today = Local::now().date_naive();
        let lessons: Vec<Lesson> = self.semester.table.on(today).into_iter().cloned().collect();
        if lessons.is_empty() {
            return;
        }
        let p = fx::pal(ui);
        let lang = self.lang;
        let now = Local::now().naive_local();
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
            ui.label(
                RichText::new(format!("{}  {}", icon::CHALKBOARD_TEACHER, lang.today_lessons()))
                    .family(fx::medium())
                    .color(p.primary),
            );
            for l in &lessons {
                let name = self
                    .config
                    .courses
                    .iter()
                    .find(|c| c.label() == l.module)
                    .map_or_else(|| l.title.clone(), |c| c.name.clone());
                let room = if l.room.is_empty() { String::new() } else { format!(", {}", l.room) };
                let text = format!("{}–{} {name}{room}", l.start, l.end);
                let (fill, fg) = if l.running(now) {
                    (p.primary_container, p.on_primary_container)
                } else {
                    (p.container_high, p.on_surface)
                };
                egui::Frame::new().fill(fill).corner_radius(8).inner_margin(Margin::symmetric(10, 4)).show(ui, |ui| {
                    ui.label(RichText::new(text).small().color(fg));
                });
            }
        });
        ui.add_space(10.0);
    }
}

impl App {
    /// The grades and ECTS of every module, the weighted average and the credit points earned.
    fn grades_ui(&mut self, ui: &mut egui::Ui) {
        use crate::config::{GradeScale, Status};
        let p = fx::pal(ui);
        let lang = self.lang;
        let scale = self.config.features.grade_scale;
        let (low, high) = scale.range();
        ui.horizontal(|ui| {
            ui.label(fx::title(lang.grades_title(), 20.0).color(p.on_surface));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let scales = [
                    (GradeScale::Swiss, lang.scale_swiss().to_string()),
                    (GradeScale::German, lang.scale_german().to_string()),
                ];
                if fx::segmented(ui, "grade-scale", &mut self.config.features.grade_scale, &scales) {
                    self.settings_changed();
                }
            });
        });
        ui.add_space(6.0);
        let mut changed = false;
        fx::card(ui, |ui| {
            if self.config.courses.is_empty() {
                ui.label(RichText::new(lang.no_modules_yet()).color(p.on_surface_variant));
                return;
            }
            egui::Grid::new("grades").num_columns(5).spacing(vec2(14.0, 8.0)).striped(false).show(ui, |ui| {
                for head in
                    [lang.module_label(), lang.semester_label(), "ECTS", lang.grade_label(), lang.status_label()]
                {
                    ui.label(RichText::new(head).small().color(p.on_surface_variant));
                }
                ui.end_row();
                for (i, course) in self.config.courses.iter_mut().enumerate() {
                    ui.label(RichText::new(&course.name).color(p.on_surface));
                    changed |= ui
                        .add(
                            egui::TextEdit::singleline(&mut course.semester)
                                .hint_text("HS 2026")
                                .desired_width(96.0)
                                .min_size(vec2(96.0, 0.0))
                                .margin(Margin::symmetric(8, 4)),
                        )
                        .changed();
                    changed |= number_field(ui, ("ects", i), &mut course.ects, 0.0, 60.0, "6");
                    let hint = if scale == GradeScale::Swiss { "5.25" } else { "1.7" };
                    changed |= number_field(ui, ("grade", i), &mut course.grade, low, high, hint);
                    let status = match course.grade {
                        Some(g) if scale.passed(g) => (lang.passed(), p.success),
                        Some(_) => (lang.failed(), p.error),
                        None => match course.status {
                            Status::Passed => (lang.passed(), p.success),
                            Status::Failed => (lang.failed(), p.error),
                            Status::Open => (lang.open_status(), p.on_surface_variant),
                        },
                    };
                    if course.grade.is_some() {
                        ui.label(RichText::new(status.0).color(status.1));
                    } else {
                        // Without a grade (pass/fail modules): set by hand.
                        egui::ComboBox::from_id_salt(("status", i))
                            .selected_text(RichText::new(status.0).color(status.1))
                            .width(120.0)
                            .show_ui(ui, |ui| {
                                for (s, label) in [
                                    (Status::Open, lang.open_status()),
                                    (Status::Passed, lang.passed()),
                                    (Status::Failed, lang.failed()),
                                ] {
                                    changed |= ui.selectable_value(&mut course.status, s, label).changed();
                                }
                            });
                    }
                    ui.end_row();
                }
            });
            ui.add_space(8.0);
            super::settings::divider(ui);
            let courses: Vec<&crate::config::Course> = self.config.courses.iter().collect();
            let (average, earned, _) = crate::config::grade_summary(&courses, scale);
            let target = self.config.features.ects_target.max(1.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new(lang.average()).color(p.on_surface_variant));
                let text = average.map_or("–".to_string(), |a| format!("{a:.2}"));
                let color = match average {
                    Some(a) if !scale.passed(a) => p.error,
                    _ => p.primary,
                };
                ui.label(fx::title(text, 18.0).color(color));
                ui.add_space(20.0);
                ui.label(RichText::new(lang.ects_earned()).color(p.on_surface_variant));
                ui.label(fx::title(format!("{earned:.0} / {target:.0}"), 18.0).color(p.on_surface));
                let mut target_value = Some(self.config.features.ects_target);
                ui.label(RichText::new(lang.ects_target()).small().color(p.outline));
                if number_field(ui, "ects-target", &mut target_value, 1.0, 600.0, "180") {
                    self.config.features.ects_target = target_value.unwrap_or(180.0);
                    changed = true;
                }
            });
            fx::progress_bar(ui, Id::new("ects-progress"), Some((earned / target).clamp(0.0, 1.0)));
            // Per semester.
            let mut semesters: Vec<String> =
                self.config.courses.iter().map(|c| c.semester.clone()).filter(|s| !s.is_empty()).collect();
            semesters.sort();
            semesters.dedup();
            if semesters.len() > 1 {
                ui.add_space(4.0);
                ui.horizontal_wrapped(|ui| {
                    for s in &semesters {
                        let these: Vec<&crate::config::Course> =
                            self.config.courses.iter().filter(|c| c.semester == *s).collect();
                        let (avg, ects, _) = crate::config::grade_summary(&these, scale);
                        let avg = avg.map_or("–".to_string(), |a| format!("{a:.2}"));
                        ui.label(
                            RichText::new(format!("{s}: {avg} · {ects:.0} ECTS")).small().color(p.on_surface_variant),
                        );
                        ui.add_space(10.0);
                    }
                });
            }
        });
        if changed {
            self.settings_changed();
        }
    }
}

/// A number field (with «,» or «.»), kept while typing; `None` when empty. Out of range
/// shows red and isn't taken. Returns true when the value changed.
fn number_field(
    ui: &mut egui::Ui,
    id: impl std::hash::Hash + std::fmt::Debug,
    value: &mut Option<f32>,
    low: f32,
    high: f32,
    hint: &str,
) -> bool {
    let p = fx::pal(ui);
    let id = Id::new(id);
    let shown = value.map(|v| format!("{}", (v * 100.0).round() / 100.0)).unwrap_or_default();
    let mut text = ui.data(|d| d.get_temp::<String>(id)).unwrap_or_else(|| shown.clone());
    let parsed = text.trim().replace(',', ".").parse::<f32>().ok();
    let valid = text.trim().is_empty() || parsed.is_some_and(|v| (low..=high).contains(&v));
    let field = egui::TextEdit::singleline(&mut text)
        .hint_text(hint)
        .desired_width(64.0)
        .margin(Margin::symmetric(8, 4))
        .text_color_opt((!valid).then_some(p.error));
    let response = ui.add(field);
    let mut changed = false;
    if response.changed() {
        let new = text.trim().replace(',', ".").parse::<f32>().ok().filter(|v| (low..=high).contains(v));
        if text.trim().is_empty() {
            changed = value.take().is_some();
        } else if new.is_some() && new != *value {
            *value = new;
            changed = true;
        }
    }
    if !response.has_focus() {
        text = value.map(|v| format!("{}", (v * 100.0).round() / 100.0)).unwrap_or_default();
    }
    ui.data_mut(|d| d.insert_temp(id, text));
    changed
}
