//! Subfolders in the window: the editable list in the settings, creating the subfolders in
//! the modules, and moving files already in the modules into them (with a preview first
//! and undo afterwards).

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, channel};

use eframe::egui::{self, Id, Margin, RichText, vec2};
use egui_phosphor::regular as icon;

use super::dialogs::{actions, dialog_frame};
use super::{App, Note};
use crate::config::Subfolder;
use crate::fx::{self, ButtonStyle};
use crate::i18n::Lang;
use crate::resort::{self, Planned, Report};

/// What the subfolder list in the settings asks for.
#[derive(Clone, Copy, PartialEq)]
pub(super) enum SubfolderAction {
    None,
    CreateAll,
    Resort,
}

#[derive(Default)]
pub(super) struct ResortState {
    planning: Option<Receiver<Vec<Planned>>>,
    preview: Option<Vec<Planned>>,
    applying: Option<Receiver<Report>>,
}

/// The editable list of subfolders: name, own words, delete; «+ Subfolder», «Defaults», and
/// the buttons to create them and to sort existing files into them.
pub(super) fn subfolder_list_ui(
    ui: &mut egui::Ui,
    lang: Lang,
    list: &mut Vec<Subfolder>,
    busy: bool,
    changed: &mut bool,
) -> SubfolderAction {
    let p = fx::pal(ui);
    let mut action = SubfolderAction::None;
    let mut remove = None;
    ui.horizontal(|ui| {
        ui.add_space(40.0);
        ui.add(egui::Label::new(RichText::new(lang.subfolder_list_hint()).small().color(p.on_surface_variant)).wrap());
    });
    ui.add_space(4.0);
    for (i, folder) in list.iter_mut().enumerate() {
        ui.horizontal(|ui| {
            ui.add_space(40.0);
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
    }
    if let Some(i) = remove {
        list.remove(i);
        *changed = true;
    }
    ui.horizontal(|ui| {
        ui.add_space(40.0);
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
    ui.horizontal_wrapped(|ui| {
        ui.add_space(40.0);
        ui.spacing_mut().item_spacing = vec2(8.0, 8.0);
        if fx::button(ui, ButtonStyle::Tonal, icon::FOLDER_PLUS, lang.create_in_all_modules()).clicked() {
            action = SubfolderAction::CreateAll;
        }
        let resort = ui.add_enabled_ui(!busy, |ui| {
            fx::button(ui, ButtonStyle::Tonal, icon::ARROWS_DOWN_UP, lang.resort_existing())
        });
        if resort.inner.on_hover_text(lang.resort_existing_hint()).clicked() {
            action = SubfolderAction::Resort;
        }
        if busy {
            ui.add(egui::Spinner::new());
        }
    });
    ui.add_space(8.0);
    action
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
            }
            SubfolderAction::Resort => {
                self.save_quietly();
                let (tx, rx) = channel();
                let (config, ctx) = (self.config.clone(), self.ctx.clone());
                std::thread::spawn(move || {
                    let _ = tx.send(resort::plan(&config));
                    ctx.request_repaint();
                });
                self.resort.planning = Some(rx);
            }
        }
    }

    /// Create the subfolders in one module (from its Edit dialog).
    pub(super) fn create_subfolders_for(&mut self, index: usize) {
        let Some(course) = self.config.courses.get(index).cloned() else { return };
        let made = resort::create_for(&self.config, &course);
        self.note(Note::Folders { title: self.lang.folders_created(made), details: course.label() });
    }

    /// Take in a finished plan or move.
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
