//! Entries of the activity list (also used by the right-click results window) and the
//! snackbar that offers to undo the latest sort.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use chrono::{DateTime, Local};
use eframe::egui::{self, Align, Color32, CornerRadius, FontId, Layout, Rect, RichText, Sense, vec2};
use egui_phosphor::regular as icon;

use crate::fx::{self, ButtonStyle, Palette};
use crate::i18n::Lang;
use crate::sorter::{LeftReason, Outcome, SortResult};
use crate::undo::UndoError;

pub(crate) const CARD_APPEAR_SECS: f64 = 0.35;
pub(crate) const CARD_FLASH_SECS: f64 = 0.9;

/// A background step for an entry has finished.
pub(crate) enum Finished {
    /// Undo: where the file is now.
    Undo(Result<PathBuf, UndoError>),
    /// Assigned to a module by hand: the new result.
    Assigned(Result<SortResult, UndoError>),
}

/// Things worth telling the user that aren't about a single file.
#[derive(Debug, Clone)]
pub(crate) enum Note {
    Started(String),
    Stopped,
    /// Sorting the files already in the folder: all of them, or only the last `days` days.
    SortingExisting(Option<u32>),
    Imported {
        added: usize,
        known: usize,
    },
    KeywordsAdded {
        module: String,
        words: Vec<String>,
    },
    NoNewKeywords {
        module: String,
        files: usize,
    },
    NoFilesToScan {
        module: String,
        folder: String,
    },
    ScanFailed,
    /// Keywords learned from a file the user assigned by hand (`undone` once taken back).
    Learned {
        module: String,
        words: Vec<String>,
        undone: bool,
    },
    ConfigUnreadable {
        path: String,
        error: String,
    },
    /// Moving the study folder, the copy in the cloud.
    Cloud {
        title: String,
        details: String,
        warn: bool,
    },
}

#[derive(Debug, Clone)]
pub(crate) enum FileState {
    Waiting,
    Vanished,
    Done(SortResult),
    Undoing(SortResult),
    /// Being moved into the module the user chose.
    Assigning(SortResult),
    Undone {
        back: PathBuf,
    },
    UndoFailed {
        result: SortResult,
        error: UndoError,
    },
}

#[derive(Debug, Clone)]
pub(crate) enum CardKind {
    File { name: String, state: FileState },
    Note(Note),
}

/// One entry in the activity list. File entries change state in place (waiting → moved → put back).
pub(crate) struct Card {
    pub(crate) key: u64,
    pub(crate) kind: CardKind,
    pub(crate) born: f64,
    pub(crate) changed: f64,
    clock: String,
}

fn next_key() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

impl Card {
    pub(crate) fn new(kind: CardKind, now: f64) -> Card {
        Card { key: next_key(), kind, born: now, changed: now, clock: Local::now().format("%H:%M").to_string() }
    }

    /// An entry from an earlier session (shown without animation).
    pub(crate) fn past(kind: CardKind, time: DateTime<Local>) -> Card {
        let clock = if time.date_naive() == Local::now().date_naive() {
            time.format("%H:%M").to_string()
        } else {
            time.format("%d.%m. %H:%M").to_string()
        };
        Card { key: next_key(), kind, born: f64::NEG_INFINITY, changed: f64::NEG_INFINITY, clock }
    }

    pub(crate) fn set_state(&mut self, state: FileState, now: f64) {
        if let CardKind::File { state: current, .. } = &mut self.kind {
            *current = state;
            self.changed = now;
        }
    }

    /// The sorted file's result, while it can still be undone.
    pub(crate) fn undoable(&self) -> Option<&SortResult> {
        match &self.kind {
            CardKind::File { state: FileState::Done(result), .. } if result.sorted() => Some(result),
            _ => None,
        }
    }

    /// The file's result while it can be put into a(nother) module by hand: sorted, or left
    /// because no module fit (or it was open, or it stayed after an undo).
    pub(crate) fn assignable(&self) -> Option<&SortResult> {
        match &self.kind {
            CardKind::File { state: FileState::Done(result), .. }
                if result.sorted()
                    || matches!(
                        result.outcome,
                        Outcome::Left(LeftReason::NoMatch | LeftReason::StillOpen { .. } | LeftReason::Kept)
                    ) =>
            {
                Some(result)
            }
            _ => None,
        }
    }

    /// Where this entry's file is now, for "show in folder".
    pub(crate) fn location(&self) -> Option<&Path> {
        let CardKind::File { state, .. } = &self.kind else { return None };
        match state {
            FileState::Done(result)
            | FileState::Undoing(result)
            | FileState::Assigning(result)
            | FileState::UndoFailed { result, .. } => match &result.outcome {
                Outcome::Moved { destination, .. } | Outcome::Copied { destination, .. } => Some(destination),
                Outcome::DuplicateRemoved { existing, .. } => Some(existing),
                Outcome::Left(LeftReason::AlreadyCopied { copy, .. }) => Some(copy),
                Outcome::Left(_) => None,
            },
            FileState::Undone { back, .. } => Some(back),
            _ => None,
        }
    }
}

/// What an entry shows.
struct Look {
    /// Empty: a spinner instead of an icon.
    glyph: &'static str,
    container: Color32,
    on_container: Color32,
    title: String,
    line: Option<String>,
    detail: Option<String>,
}

pub(crate) fn note_title(lang: Lang, note: &Note) -> String {
    match note {
        Note::Started(folder) => {
            let path = Path::new(folder);
            lang.started(&path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| folder.clone()))
        }
        Note::Stopped => lang.stopped_watching().into(),
        Note::SortingExisting(days) => lang.sorting_existing(*days),
        Note::Imported { added, known } => lang.imported(*added, *known),
        Note::KeywordsAdded { module, words } => lang.keywords_added(words.len(), module),
        Note::NoNewKeywords { module, files } => lang.no_new_keywords(module, *files),
        Note::NoFilesToScan { module, .. } => lang.no_files_to_scan(module),
        Note::ScanFailed => lang.scan_failed().into(),
        Note::Learned { module, words, undone } => lang.learned(module, words.len(), *undone),
        Note::ConfigUnreadable { path, error } => lang.config_unreadable(path, error),
        Note::Cloud { title, .. } => title.clone(),
    }
}

fn folder_name(path: &Path) -> String {
    path.parent().and_then(Path::file_name).map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

fn look(p: &Palette, lang: Lang, card: &Card) -> Look {
    let neutral = (p.container_highest, p.on_surface_variant);
    let good = (p.success_container, p.on_success_container);
    let warn = (p.warning_container, p.on_warning_container);
    let warn_colors = warn;
    let bad = (p.error_container, p.on_error_container);
    let blue = (p.primary_container, p.on_primary_container);
    let make = |glyph, (container, on_container): (Color32, Color32), title: String, line, detail| Look {
        glyph,
        container,
        on_container,
        title,
        line,
        detail,
    };
    match &card.kind {
        CardKind::File { name, state } => {
            let name = name.clone();
            match state {
                FileState::Waiting => make("", blue, name, Some(lang.waiting_for_download().into()), None),
                FileState::Vanished => make(icon::X_CIRCLE, neutral, name, Some(lang.vanished().into()), None),
                FileState::Undoing(_) => make("", neutral, name, Some(lang.undoing().into()), None),
                FileState::Assigning(_) => make("", blue, name, Some(lang.assigning().into()), None),
                FileState::Undone { back, .. } => {
                    make(icon::ARROW_U_UP_LEFT, neutral, name, Some(lang.undone(&folder_name(back))), None)
                }
                FileState::UndoFailed { error, .. } => {
                    make(icon::WARNING_CIRCLE, warn, name, Some(lang.undo_failed(error)), None)
                }
                FileState::Done(result) => {
                    let saved_as = result.renamed_to().map(|n| lang.saved_as(&n));
                    let with_reason = |reason| match &saved_as {
                        Some(note) => Some(format!("{note} · {reason}")),
                        None => Some(reason),
                    };
                    match &result.outcome {
                        Outcome::Moved { course, reason, .. } => {
                            make(icon::CHECK, good, name, Some(lang.moved_to(course)), with_reason(lang.reason(reason)))
                        }
                        Outcome::Copied { course, reason, .. } => {
                            make(icon::COPY, good, name, Some(lang.copied_to(course)), with_reason(lang.reason(reason)))
                        }
                        Outcome::DuplicateRemoved { course, .. } => {
                            make(icon::COPY_SIMPLE, blue, name, Some(lang.duplicate_removed(course)), None)
                        }
                        Outcome::Left(reason) => {
                            let (glyph, colors) = match reason {
                                LeftReason::NoMatch => (icon::QUESTION, warn),
                                LeftReason::InUse { .. } | LeftReason::StillOpen { .. } => {
                                    (icon::HOURGLASS_MEDIUM, warn)
                                }
                                LeftReason::Error(_) => (icon::X, bad),
                                LeftReason::AlreadyCopied { .. } => (icon::COPY, neutral),
                                _ => (icon::INFO, neutral),
                            };
                            make(glyph, colors, name, Some(lang.left_reason(reason)), None)
                        }
                    }
                }
            }
        }
        CardKind::Note(note) => {
            let title = note_title(lang, note);
            match note {
                Note::Started(folder) => make(icon::PLAY, blue, title, None, Some(folder.clone())),
                Note::Stopped => make(icon::STOP, neutral, title, None, None),
                Note::SortingExisting(_) => make(icon::BROOM, blue, title, None, None),
                Note::Imported { .. } => make(icon::FOLDERS, blue, title, None, None),
                Note::KeywordsAdded { words, .. } => make(icon::SPARKLE, blue, title, Some(words.join(" · ")), None),
                Note::NoNewKeywords { .. } => make(icon::SPARKLE, neutral, title, None, None),
                Note::NoFilesToScan { folder, .. } => {
                    make(icon::FOLDER_OPEN, warn, title, Some(lang.folder_empty(folder)), None)
                }
                Note::ScanFailed | Note::ConfigUnreadable { .. } => make(icon::X, bad, title, None, None),
                Note::Learned { words, undone, .. } => {
                    let words = words.iter().map(|w| format!("«{w}»")).collect::<Vec<_>>().join(" · ");
                    make(icon::BRAIN, if *undone { neutral } else { blue }, title, Some(words), None)
                }
                Note::Cloud { details, warn, .. } => {
                    let details = (!details.is_empty()).then(|| details.clone());
                    if *warn {
                        make(icon::CLOUD_WARNING, warn_colors, title, details, None)
                    } else {
                        make(icon::CLOUD_CHECK, blue, title, details, None)
                    }
                }
            }
        }
    }
}

/// What the user did with an entry.
pub(crate) enum RowAction {
    None,
    Open(PathBuf),
    Undo,
    /// Put the file into this module (index into the module list).
    Assign(usize),
}

/// One entry as a Material list item: icon in a tinted circle, title, status, details,
/// time, an undo button and a menu to put the file into a(nother) module (`modules`, by
/// label; empty for none). Slides in when new and flashes when its state changes.
pub(crate) fn activity_row(ui: &mut egui::Ui, lang: Lang, card: &Card, now: f64, modules: &[String]) -> RowAction {
    let p = fx::pal(ui);
    let look = look(&p, lang, card);
    let motion = fx::motion(ui.ctx());
    let appear = if motion { fx::ease_out_cubic(((now - card.born) / CARD_APPEAR_SECS) as f32) } else { 1.0 };
    let flash = if motion && card.changed > card.born {
        (1.0 - (now - card.changed) / CARD_FLASH_SECS).clamp(0.0, 1.0) as f32
    } else {
        0.0
    };
    let mut action = RowAction::None;
    let id = egui::Id::new(("activity-row", card.key));
    // The whole entry is clickable ("show in folder"). Its click area is registered before the
    // content (with last frame's size), so the undo button lies on top of it.
    let row = card
        .location()
        .zip(ui.ctx().data(|d| d.get_temp::<Rect>(id)))
        .map(|(location, rect)| (location, ui.interact(rect, id, Sense::click())));
    let width = ui.available_width();
    let inner = ui.scope(|ui| {
        ui.set_opacity(appear);
        let offset = (1.0 - appear) * 24.0;
        let frame = egui::Frame::new()
            .fill(fx::lerp_color(Color32::TRANSPARENT, look.container.gamma_multiply(0.6), flash))
            .corner_radius(CornerRadius::same(12))
            .inner_margin(egui::Margin { left: (12.0 + offset) as i8, right: 8, top: 8, bottom: 8 });
        frame
            .show(ui, |ui| {
                ui.set_width(width - 20.0);
                ui.horizontal_top(|ui| {
                    // Leading icon in a tinted circle.
                    let (circle, _) = ui.allocate_exact_size(vec2(40.0, 40.0), Sense::hover());
                    if look.glyph.is_empty() {
                        ui.painter().circle_filled(circle.center(), 20.0, look.container);
                        let spinner = Rect::from_center_size(circle.center(), vec2(22.0, 22.0));
                        ui.put(spinner, egui::Spinner::new().size(20.0).color(look.on_container));
                    } else {
                        fx::avatar(ui.painter(), circle.center(), 20.0, look.glyph, look.container, look.on_container);
                    }
                    ui.add_space(4.0);
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 2.0;
                        ui.allocate_ui_with_layout(
                            vec2(ui.available_width(), 20.0),
                            Layout::right_to_left(Align::Center),
                            |ui| {
                                let learned = matches!(card.kind, CardKind::Note(Note::Learned { undone: false, .. }));
                                if (card.undoable().is_some() || learned)
                                    && fx::button(ui, ButtonStyle::Text, icon::ARROW_U_UP_LEFT, lang.undo()).clicked()
                                {
                                    action = RowAction::Undo;
                                }
                                if let Some(result) = card.assignable().filter(|_| !modules.is_empty())
                                    && let Some(i) = assign_menu(ui, lang, result, modules)
                                {
                                    action = RowAction::Assign(i);
                                }
                                ui.label(RichText::new(&card.clock).small().color(p.on_surface_variant));
                                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                                    ui.add(
                                        egui::Label::new(fx::title(&look.title, 14.0).color(p.on_surface)).truncate(),
                                    )
                                    .on_hover_text(&look.title);
                                });
                            },
                        );
                        if let Some(line) = &look.line {
                            ui.add(egui::Label::new(RichText::new(line).color(p.on_surface_variant)).wrap());
                        }
                        if let Some(detail) = &look.detail {
                            ui.add(egui::Label::new(RichText::new(detail).small().color(p.outline)).wrap());
                        }
                    });
                });
            })
            .response
    });
    let rect = inner.inner.rect;
    ui.ctx().data_mut(|d| d.insert_temp(id, rect));
    if let Some((location, response)) = row {
        fx::state_layer(ui, &response, rect, 12, p.on_surface);
        let response = response.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(lang.open_folder_hint());
        if response.clicked() && matches!(action, RowAction::None) {
            action = RowAction::Open(location.to_path_buf());
        }
    }
    action
}

/// «Assign ▾» (or «Other module ▾» for a sorted file) with the list of modules.
fn assign_menu(ui: &mut egui::Ui, lang: Lang, result: &SortResult, modules: &[String]) -> Option<usize> {
    let p = fx::pal(ui);
    let text = if result.sorted() { lang.other_module() } else { lang.assign() };
    let button = fx::button(ui, ButtonStyle::Text, icon::CARET_DOWN, text);
    let button = button.on_hover_text(lang.assign_hint());
    let current = match &result.outcome {
        Outcome::Moved { course, .. } | Outcome::Copied { course, .. } | Outcome::DuplicateRemoved { course, .. } => {
            Some(course.as_str())
        }
        Outcome::Left(_) => None,
    };
    let mut chosen = None;
    egui::Popup::menu(&button).width(280.0).show(|ui| {
        ui.set_min_width(260.0);
        ui.label(RichText::new(lang.assign_title()).small().color(p.on_surface_variant));
        egui::ScrollArea::vertical().max_height(300.0).show(ui, |ui| {
            for (i, label) in modules.iter().enumerate() {
                let here = current == Some(label.as_str());
                let row = ui.add_enabled(!here, egui::Button::selectable(here, label).truncate());
                if row.clicked() {
                    chosen = Some(i);
                    ui.close();
                }
            }
        });
    });
    chosen
}

/// The snackbar at the bottom: a short message and, after a sort, "Undo".
pub(crate) struct Snackbar {
    /// The entry it is about.
    pub(crate) card: u64,
    pub(crate) text: String,
    pub(crate) can_undo: bool,
    pub(crate) shown: f64,
}

pub(crate) const SNACKBAR_SECS: f64 = 6.0;

/// Draw the snackbar above everything; returns true when "Undo" was clicked.
/// Returns `None` once it has disappeared.
pub(crate) fn snackbar_ui(ctx: &egui::Context, lang: Lang, bar: &mut Snackbar, now: f64) -> Option<bool> {
    let age = now - bar.shown;
    if age > SNACKBAR_SECS + 0.3 {
        return None;
    }
    let motion = fx::motion(ctx);
    let enter = if motion { fx::ease_out_back((age / 0.35) as f32) } else { 1.0 };
    let leave = if motion { (1.0 - ((age - SNACKBAR_SECS) / 0.3).max(0.0)) as f32 } else { 1.0 };
    let p = fx::pal_ctx(ctx);
    let screen = ctx.content_rect();
    let width = (screen.width() - 32.0).min(560.0);
    let lift = (1.0 - enter) * 40.0;
    let pos = egui::pos2(screen.center().x - width / 2.0, screen.bottom() - 72.0 + lift);
    let mut clicked = false;
    let area = egui::Area::new(egui::Id::new("snackbar")).order(egui::Order::Foreground).fixed_pos(pos);
    let response = area.show(ctx, |ui| {
        ui.set_opacity(leave.clamp(0.0, 1.0));
        egui::Frame::new()
            .fill(p.inverse_surface)
            .corner_radius(CornerRadius::same(8))
            .inner_margin(egui::Margin { left: 16, right: 8, top: 6, bottom: 6 })
            .shadow(egui::Shadow { offset: [0, 3], blur: 10, spread: 0, color: Color32::from_black_alpha(60) })
            .show(ui, |ui| {
                ui.set_width(width - 24.0);
                ui.horizontal(|ui| {
                    ui.set_min_height(36.0);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if bar.can_undo {
                            let text = RichText::new(lang.undo()).family(fx::medium()).color(p.inverse_primary);
                            if ui
                                .add(egui::Button::new(text).frame(false))
                                .on_hover_cursor(egui::CursorIcon::PointingHand)
                                .clicked()
                            {
                                clicked = true;
                            }
                        }
                        ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                            ui.add(egui::Label::new(RichText::new(&bar.text).color(p.inverse_on_surface)).truncate());
                        });
                    });
                });
            });
    });
    // Keep it while the mouse is over it.
    if response.response.hovered() && age > 0.5 {
        bar.shown = now - 0.5;
    }
    ctx.request_repaint();
    Some(clicked)
}

/// A small counter in a pill.
pub(crate) fn badge(painter: &egui::Painter, center: egui::Pos2, text: &str, fill: Color32, fg: Color32) {
    let galley = painter.layout_no_wrap(text.to_string(), FontId::new(12.0, fx::medium()), fg);
    let rect = Rect::from_center_size(center, vec2(galley.size().x + 12.0, 20.0));
    painter.rect_filled(rect, CornerRadius::same(10), fill);
    painter.galley(rect.center() - galley.size() / 2.0, galley, fg);
}
