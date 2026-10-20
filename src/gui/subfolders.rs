//! Subfolders in the window: the subfolders of the selected module in the overview (how
//! many files each holds, the words learned for it), creating them, moving the files already
//! in a module into them (with a preview first and undo afterwards), learning from the files
//! already in them, and the list all modules share.

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, channel};

use eframe::egui::{self, Align, Id, Layout, Margin, RichText, vec2};
use egui_phosphor::regular as icon;

use super::dialogs::{actions, dialog_frame};
use super::{App, Note};
use crate::config::{LearnedFolder, Subfolder};
use crate::fx::{self, ButtonStyle};
use crate::i18n::Lang;
use crate::matcher::normalise;
use crate::resort::{self, Planned, Report};
use crate::subfolder_learn;

/// How often subfolder words are learned by themselves (when that is switched on).
const LEARN_EVERY: f64 = 30.0 * 60.0;
/// How long the file counts of a module's subfolders are kept before they are read again.
const COUNTS_FOR: f64 = 5.0;

/// What a subfolder button asks for.
#[derive(Clone, Copy, PartialEq)]
pub(super) enum SubfolderAction {
    None,
    CreateAll,
    CreateFor(usize),
    /// Sort the files of one module (or all) into their subfolders.
    Resort(Option<usize>),
    Learn,
    OpenList,
}

/// A module's subfolders as last read: (name, files in it or `None` if it isn't there yet),
/// and how many files lie directly in the module folder.
struct Counts {
    /// The module folder they were read from.
    module: PathBuf,
    read_at: f64,
    folders: Vec<(String, Option<usize>)>,
    loose: usize,
}

#[derive(Default)]
pub(super) struct ResortState {
    planning: Option<Receiver<Vec<Planned>>>,
    preview: Option<Vec<Planned>>,
    applying: Option<Receiver<Report>>,
    /// Learning subfolder words; `true` when it started by itself.
    learning: Option<(Receiver<Vec<LearnedFolder>>, bool)>,
    /// When to learn by itself next.
    next_learn: Option<f64>,
    /// The shared list is open.
    list_open: bool,
    counts: Option<Counts>,
}

/// A learned word with a «×» that takes it out. Returns true when that was clicked.
fn word_chip(ui: &mut egui::Ui, lang: Lang, word: &str) -> bool {
    let p = fx::pal(ui);
    let mut remove = false;
    egui::Frame::new().fill(p.secondary_container).corner_radius(8).inner_margin(Margin::symmetric(8, 2)).show(
        ui,
        |ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            ui.label(RichText::new(word).small().color(p.on_secondary_container));
            let x = ui.add(
                egui::Label::new(RichText::new(icon::X).small().color(p.on_secondary_container))
                    .sense(egui::Sense::click()),
            );
            if x.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(lang.remove_word_hint()).clicked() {
                remove = true;
            }
        },
    );
    remove
}

fn has_learned(learned: &[LearnedFolder], folder: &str) -> bool {
    learned.iter().any(|l| normalise(&l.name) == normalise(folder) && !l.words.is_empty())
}

/// The learned words of `folder` as chips; returns a word to take out.
fn learned_chips(ui: &mut egui::Ui, lang: Lang, learned: &[LearnedFolder], folder: &str) -> Option<String> {
    let words = learned.iter().find(|l| normalise(&l.name) == normalise(folder)).map(|l| l.words.clone())?;
    if words.is_empty() {
        return None;
    }
    let mut remove = None;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
        ui.label(RichText::new(lang.learned_words()).small().color(fx::pal(ui).on_surface_variant));
        for word in &words {
            if word_chip(ui, lang, word) {
                remove = Some(word.clone());
            }
        }
    });
    remove
}

/// The editable list of subfolders: name, own words, learned words, delete; «+ Subfolder»
/// and «Defaults».
fn subfolder_list_ui(
    ui: &mut egui::Ui,
    lang: Lang,
    list: &mut Vec<Subfolder>,
    learned: &mut [LearnedFolder],
    changed: &mut bool,
) {
    let p = fx::pal(ui);
    let mut remove = None;
    ui.add(egui::Label::new(RichText::new(lang.subfolder_list_hint()).small().color(p.on_surface_variant)).wrap());
    ui.add_space(6.0);
    for (i, folder) in list.iter_mut().enumerate() {
        ui.horizontal(|ui| {
            ui.label(RichText::new(icon::FOLDER).color(p.on_surface_variant));
            let name = egui::TextEdit::singleline(&mut folder.name)
                .hint_text(lang.subfolder_name_hint())
                .desired_width(150.0)
                .margin(Margin::symmetric(10, 6));
            *changed |= ui.add(name).changed();
            // The words as one text ("zusammenfassung, summary"), kept while typing.
            let id = Id::new(("subfolder-words", i));
            let joined = folder.keywords.join(", ");
            let mut text = ui.data(|d| d.get_temp::<String>(id)).unwrap_or_else(|| joined.clone());
            let hint = match folder.builtin {
                Some(kind) => lang.subfolder_builtin_words(kind),
                None => lang.subfolder_words_hint(),
            };
            let field = egui::TextEdit::singleline(&mut text)
                .hint_text(hint)
                .desired_width(ui.available_width() - 48.0)
                .margin(Margin::symmetric(10, 6));
            let response = ui.add(field);
            if response.changed() {
                folder.keywords = text.split(',').map(|w| w.trim().to_string()).filter(|w| !w.is_empty()).collect();
                *changed = true;
            }
            if !response.has_focus() {
                text = folder.keywords.join(", ");
            }
            ui.data_mut(|d| d.insert_temp(id, text));
            if fx::icon_button(ui, icon::TRASH).on_hover_text(lang.delete()).clicked() {
                remove = Some(i);
            }
        });
        if has_learned(learned, &folder.name) {
            ui.horizontal(|ui| {
                ui.add_space(26.0);
                if let Some(word) = learned_chips(ui, lang, learned, &folder.name) {
                    subfolder_learn::remove_word(learned, &folder.name, &word);
                    *changed = true;
                }
            });
        }
    }
    if let Some(i) = remove {
        list.remove(i);
        *changed = true;
    }
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing = vec2(8.0, 8.0);
        if fx::button(ui, ButtonStyle::Text, icon::PLUS, lang.add_subfolder()).clicked() {
            list.push(Subfolder { name: String::new(), keywords: Vec::new(), builtin: None });
            *changed = true;
        }
        if fx::button(ui, ButtonStyle::Text, icon::ARROW_COUNTER_CLOCKWISE, lang.defaults()).clicked() {
            *list = Subfolder::defaults();
            for (folder, name) in list.iter_mut().zip(lang.subfolder_defaults()) {
                folder.name = name.to_string();
            }
            *changed = true;
        }
    });
}

impl App {
    pub(super) fn resort_busy(&self) -> bool {
        self.resort.planning.is_some() || self.resort.applying.is_some()
    }

    pub(super) fn subfolder_action(&mut self, action: SubfolderAction) {
        match action {
            SubfolderAction::None => {}
            SubfolderAction::CreateAll => {
                self.save_quietly();
                let made = resort::create_all(&self.config);
                self.note(Note::Folders { title: self.lang.folders_created(made), details: String::new() });
                self.after_folder_change();
            }
            SubfolderAction::CreateFor(i) => self.create_subfolders_for(i),
            SubfolderAction::Resort(module) => {
                self.save_quietly();
                let (tx, rx) = channel();
                let (config, ctx) = (self.config.clone(), self.ctx.clone());
                std::thread::spawn(move || {
                    let _ = tx.send(resort::plan_for(&config, module));
                    ctx.request_repaint();
                });
                self.resort.planning = Some(rx);
            }
            SubfolderAction::Learn => self.start_learning(false),
            SubfolderAction::OpenList => self.resort.list_open = true,
        }
    }

    /// Create the subfolders in one module.
    pub(super) fn create_subfolders_for(&mut self, index: usize) {
        let Some(course) = self.config.courses.get(index).cloned() else { return };
        let made = resort::create_for(&self.config, &course);
        self.note(Note::Folders { title: self.lang.folders_created(made), details: course.label() });
        self.after_folder_change();
    }

    /// Folders were made or files moved: count again, and learn soon if that happens by itself.
    fn after_folder_change(&mut self) {
        self.resort.counts = None;
        if self.config.features.subfolder_auto_learn {
            self.resort.next_learn = Some(self.now() + 5.0);
        }
    }

    /// Learn subfolder words from the files in the subfolders, on a background thread.
    fn start_learning(&mut self, by_itself: bool) {
        if self.resort.learning.is_some() {
            return;
        }
        let (tx, rx) = channel();
        let (config, ctx) = (self.config.clone(), self.ctx.clone());
        let index = self.study_indexer().map(|i| i.index.clone());
        std::thread::spawn(move || {
            let starts =
                index.and_then(|i| i.read().ok().map(|i| i.starts(subfolder_learn::TEXT_WORDS))).unwrap_or_default();
            let _ = tx.send(subfolder_learn::learn(&config, &starts));
            ctx.request_repaint();
        });
        self.resort.learning = Some((rx, by_itself));
    }

    /// Take in a finished plan, move or learning; learn by itself when it's time.
    pub(super) fn resort_logic(&mut self) {
        if let Some(rx) = &self.resort.planning
            && let Ok(planned) = rx.try_recv()
        {
            self.resort.planning = None;
            if planned.is_empty() {
                self.note(Note::Folders { title: self.lang.nothing_to_resort().into(), details: String::new() });
            } else {
                self.resort.preview = Some(planned);
            }
        }
        if let Some(rx) = &self.resort.applying
            && let Ok(report) = rx.try_recv()
        {
            self.resort.applying = None;
            self.after_resort(report);
        }
        if let Some((rx, by_itself)) = &self.resort.learning
            && let Ok(learned) = rx.try_recv()
        {
            let by_itself = *by_itself;
            self.resort.learning = None;
            self.after_learning(learned, by_itself);
        }
        let features = &self.config.features;
        let now = self.now();
        if features.subfolders && features.subfolder_auto_learn && !self.config.courses.is_empty() {
            // The first time a little after the start, when the index is loaded.
            let next = *self.resort.next_learn.get_or_insert(now + 20.0);
            if now >= next && self.resort.learning.is_none() {
                self.resort.next_learn = Some(now + LEARN_EVERY);
                self.start_learning(true);
            }
        } else {
            self.resort.next_learn = None;
        }
    }

    fn after_learning(&mut self, learned: Vec<LearnedFolder>, by_itself: bool) {
        let before = self.config.features.subfolder_learned.clone();
        let added = subfolder_learn::added_words(&before, &learned);
        // By itself: quietly, unless there is something new.
        if by_itself && added.is_empty() {
            if learned != before {
                self.config.features.subfolder_learned = learned;
                self.save_quietly();
            }
            return;
        }
        let folders: std::collections::BTreeSet<&str> = added.iter().map(|(f, _)| f.as_str()).collect();
        let title = self.lang.folder_words_learned(added.len(), folders.len());
        let details = folders
            .iter()
            .map(|f| {
                let words: Vec<&str> = added.iter().filter(|(g, _)| g == f).map(|(_, w)| w.as_str()).collect();
                format!("{f}: {}", words.join(", "))
            })
            .collect::<Vec<_>>()
            .join("\n");
        self.config.features.subfolder_learned = learned;
        self.save_quietly();
        if added.is_empty() {
            self.note(Note::Folders { title, details });
        } else {
            self.note(Note::FolderWords { title, details, before, undone: false });
        }
    }

    fn after_resort(&mut self, report: Report) {
        let lang = self.lang;
        for (from, to) in &report.moved {
            self.relocate_cards(from, to, &[(from.clone(), to.clone())]);
        }
        self.cloud_forget(report.moved.iter().map(|(from, _)| from.clone()).collect());
        if let Some(indexer) = self.study_indexer() {
            indexer.refresh();
        }
        let mut details: Vec<String> = Vec::new();
        if !report.open.is_empty() {
            details.push(lang.resort_open(report.open.len()));
        }
        details.extend(report.failed.iter().map(|(file, e)| format!("{}: {e}", file.display())));
        self.note(Note::Resorted {
            title: lang.resorted(report.moved.len()),
            details: details.join("\n"),
            moved: report.moved,
            undone: false,
        });
        self.after_folder_change();
    }

    /// «Undo» on the note: the files go back to the module folders.
    pub(super) fn undo_resort(&mut self, moved: Vec<(PathBuf, PathBuf)>) {
        let (back, stuck) = resort::undo(&moved, &self.config);
        for (from, to) in &back {
            self.relocate_cards(from, to, &[(from.clone(), to.clone())]);
        }
        self.cloud_forget(back.iter().map(|(from, _)| from.clone()).collect());
        if !stuck.is_empty() {
            let details = stuck.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join("\n");
            self.note(Note::Folders { title: self.lang.resort_not_back(stuck.len()), details });
        }
        self.resort.counts = None;
    }

    /// The subfolders of a module as they are on disk now (read again every few seconds).
    fn module_counts(&mut self, index: usize) -> &Counts {
        let now = self.now();
        let folder = self.config.course_folder(&self.config.courses[index]);
        let stale = self.resort.counts.as_ref().is_none_or(|c| c.module != folder || now - c.read_at > COUNTS_FOR);
        if stale {
            let config = &self.config;
            let existing = crate::categories::existing_subfolders(&folder, &config.features.versions_folder);
            let mut names: Vec<String> = config
                .features
                .subfolder_list
                .iter()
                .map(|f| crate::categories::safe_folder_name(&f.name))
                .filter(|n| !n.is_empty())
                .collect();
            for dir in &existing {
                if !names.iter().any(|n| n.eq_ignore_ascii_case(dir)) {
                    names.push(dir.clone());
                }
            }
            let folders = names
                .into_iter()
                .map(|name| {
                    let dir = folder.join(&name);
                    let count = dir.is_dir().then(|| subfolder_learn::files_in(&dir, config).len());
                    (name, count)
                })
                .collect();
            let loose = std::fs::read_dir(&folder)
                .map(|entries| {
                    entries
                        .flatten()
                        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
                        .filter(|e| crate::sorter::is_candidate(&e.path(), config))
                        .count()
                })
                .unwrap_or(0);
            self.resort.counts = Some(Counts { module: folder, read_at: now, folders, loose });
        }
        self.resort.counts.as_ref().expect("just read")
    }

    /// The «Subfolders» side of the selected module in the overview.
    pub(super) fn module_subfolders_ui(&mut self, ui: &mut egui::Ui, index: usize) {
        let p = fx::pal(ui);
        let lang = self.lang;
        if !self.config.features.subfolders {
            ui.add_space(4.0);
            ui.add(egui::Label::new(RichText::new(lang.subfolders_off()).color(p.on_surface_variant)).wrap());
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if fx::toggle(ui, &mut self.config.features.subfolders).changed() {
                    self.settings_changed();
                }
                ui.label(RichText::new(lang.use_subfolders()).color(p.on_surface));
            });
            return;
        }
        let counts = self.module_counts(index);
        let (folders, loose) = (counts.folders.clone(), counts.loose);
        let module = self.config.course_folder(&self.config.courses[index]);
        let mut remove_word: Option<(String, String)> = None;
        let mut open = None;
        let busy = self.resort_busy();
        let learning = self.resort.learning.is_some();
        let missing = folders.iter().any(|(_, c)| c.is_none());
        let mut action = SubfolderAction::None;
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
            let create = ui.add_enabled_ui(missing, |ui| {
                fx::button(ui, ButtonStyle::Tonal, icon::FOLDER_PLUS, lang.create_here())
            });
            if create.inner.on_hover_text(lang.create_here_hint()).clicked() {
                action = SubfolderAction::CreateFor(index);
            }
            let resort = ui.add_enabled_ui(!busy && loose > 0, |ui| {
                fx::button(ui, ButtonStyle::Tonal, icon::ARROWS_DOWN_UP, lang.resort_here())
            });
            if resort.inner.on_hover_text(lang.resort_here_hint()).clicked() {
                action = SubfolderAction::Resort(Some(index));
            }
            let learn =
                ui.add_enabled_ui(!learning, |ui| fx::button(ui, ButtonStyle::Text, icon::BRAIN, lang.learn_folders()));
            if learn.inner.on_hover_text(lang.learn_folders_hint()).clicked() {
                action = SubfolderAction::Learn;
            }
            if fx::icon_button(ui, icon::LIST_BULLETS).on_hover_text(lang.edit_list()).clicked() {
                action = SubfolderAction::OpenList;
            }
            if busy || learning {
                ui.add(egui::Spinner::new());
            }
        });
        ui.label(RichText::new(lang.loose_files(loose)).small().color(p.on_surface_variant));
        ui.add_space(4.0);
        // The subfolders as small tiles in two columns: name, files, learned words.
        let learned = self.config.features.subfolder_learned.clone();
        egui::ScrollArea::vertical().id_salt("module-subfolders").max_height(190.0).auto_shrink([false, true]).show(
            ui,
            |ui| {
                ui.columns(2, |columns| {
                    for (k, (name, count)) in folders.iter().enumerate() {
                        let ui = &mut columns[k % 2];
                        let tile =
                            egui::Frame::new().fill(p.card).corner_radius(10).inner_margin(Margin::symmetric(10, 6));
                        tile.show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 6.0;
                                let (glyph, color) = match count {
                                    Some(_) => (icon::FOLDER, p.primary),
                                    None => (icon::FOLDER_DASHED, p.outline),
                                };
                                ui.label(RichText::new(glyph).size(16.0).color(color));
                                let text = RichText::new(name).family(fx::medium()).color(p.on_surface);
                                let response = ui.add(egui::Label::new(text).truncate().sense(egui::Sense::click()));
                                if count.is_some()
                                    && response
                                        .on_hover_cursor(egui::CursorIcon::PointingHand)
                                        .on_hover_text(lang.open_this_folder())
                                        .clicked()
                                {
                                    open = Some(module.join(name));
                                }
                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    let info = match count {
                                        Some(n) => lang.file_count(*n),
                                        None => lang.subfolder_not_created().to_string(),
                                    };
                                    ui.label(RichText::new(info).small().color(p.on_surface_variant));
                                });
                            });
                            if let Some(word) = learned_chips(ui, lang, &learned, name) {
                                remove_word = Some((name.clone(), word));
                            }
                        });
                        ui.add_space(2.0);
                    }
                });
            },
        );
        if let Some((folder, word)) = remove_word {
            subfolder_learn::remove_word(&mut self.config.features.subfolder_learned, &folder, &word);
            self.settings_changed();
        }
        if let Some(dir) = open {
            crate::desktop::open(&dir);
        }
        self.subfolder_action(action);
    }

    /// The list all modules share, with the words learned for each folder.
    pub(super) fn subfolder_list_dialog(&mut self, ctx: &egui::Context) {
        if !self.resort.list_open {
            return;
        }
        let lang = self.lang;
        let busy = self.resort_busy();
        let learning = self.resort.learning.is_some();
        let mut changed = false;
        let mut action = SubfolderAction::None;
        let mut close = false;
        let features = &mut self.config.features;
        let modal = egui::Modal::new(Id::new("subfolder-list")).frame(dialog_frame(ctx)).show(ctx, |ui| {
            let p = fx::pal(ui);
            ui.set_width(620.0);
            ui.horizontal(|ui| {
                ui.label(fx::title(lang.subfolder_list_title(), 22.0).color(p.on_surface));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    changed |= fx::toggle(ui, &mut features.subfolders).changed();
                    ui.label(RichText::new(lang.use_subfolders()).color(p.on_surface_variant));
                });
            });
            ui.add_space(8.0);
            egui::ScrollArea::vertical().max_height(380.0).show(ui, |ui| {
                subfolder_list_ui(
                    ui,
                    lang,
                    &mut features.subfolder_list,
                    &mut features.subfolder_learned,
                    &mut changed,
                );
            });
            ui.add_space(8.0);
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(8.0, 8.0);
                if fx::button(ui, ButtonStyle::Tonal, icon::FOLDER_PLUS, lang.create_in_all_modules()).clicked() {
                    action = SubfolderAction::CreateAll;
                }
                let resort = ui.add_enabled_ui(!busy, |ui| {
                    fx::button(ui, ButtonStyle::Tonal, icon::ARROWS_DOWN_UP, lang.resort_existing())
                });
                if resort.inner.on_hover_text(lang.resort_existing_hint()).clicked() {
                    action = SubfolderAction::Resort(None);
                }
                let learn = ui.add_enabled_ui(!learning, |ui| {
                    fx::button(ui, ButtonStyle::Text, icon::BRAIN, lang.learn_folders())
                });
                if learn.inner.on_hover_text(lang.learn_folders_hint()).clicked() {
                    action = SubfolderAction::Learn;
                }
                if busy || learning {
                    ui.add(egui::Spinner::new());
                }
            });
            actions(ui, |ui| {
                if fx::button(ui, ButtonStyle::Filled, "", lang.close()).clicked() {
                    close = true;
                }
            });
        });
        if changed {
            self.resort.counts = None;
            self.settings_changed();
        }
        // Moving files gets its own preview: this list closes for it.
        if close || modal.should_close() || matches!(action, SubfolderAction::Resort(_)) {
            self.resort.list_open = false;
        }
        self.subfolder_action(action);
    }

    /// The preview: how many files go where, then «Sort in».
    pub(super) fn resort_dialog_ui(&mut self, ctx: &egui::Context) {
        let lang = self.lang;
        let Some(planned) = &self.resort.preview else { return };
        let mut start = false;
        let mut close = false;
        let modal = egui::Modal::new(Id::new("resort-dialog")).frame(dialog_frame(ctx)).show(ctx, |ui| {
            let p = fx::pal(ui);
            ui.set_width(480.0);
            ui.label(fx::title(lang.resort_existing(), 22.0).color(p.on_surface));
            ui.add_space(8.0);
            ui.add(
                egui::Label::new(RichText::new(lang.resort_explained(planned.len())).color(p.on_surface_variant))
                    .wrap(),
            );
            ui.add_space(8.0);
            egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                let mut last_module = String::new();
                for (module, sub, count) in resort::summary(planned) {
                    if module != last_module {
                        ui.add_space(4.0);
                        ui.label(RichText::new(&module).family(fx::medium()).color(p.on_surface));
                        last_module = module;
                    }
                    ui.label(
                        RichText::new(format!("    {}  {sub} · {}", icon::FOLDER, lang.file_count(count)))
                            .color(p.on_surface_variant),
                    );
                }
            });
            actions(ui, |ui| {
                if fx::button(ui, ButtonStyle::Filled, "", lang.resort_start()).clicked() {
                    start = true;
                }
                if fx::button(ui, ButtonStyle::Text, "", lang.cancel()).clicked() {
                    close = true;
                }
            });
        });
        if start {
            let planned = self.resort.preview.take().unwrap_or_default();
            let (tx, rx) = channel();
            let (config, ctx) = (self.config.clone(), self.ctx.clone());
            std::thread::spawn(move || {
                let _ = tx.send(resort::apply(&planned, &config));
                ctx.request_repaint();
            });
            self.resort.applying = Some(rx);
        } else if close || modal.should_close() {
            self.resort.preview = None;
        }
    }
}
