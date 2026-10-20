//! «Modules directly in the study folder»: the setting, and moving the module folders out
//! of their study programs' folders, with «Undo».

use std::sync::mpsc::{Receiver, channel};

use eframe::egui::{self, Id, RichText};
use egui_phosphor::regular as icon;

use super::dialogs::{actions, dialog_frame};
use super::settings::setting_row;
use super::{App, Note};
use crate::archive::Moved;
use crate::config::Course;
use crate::flatten::{self, Done, Kept, Step};
use crate::fx::{self, ButtonStyle};

pub(super) enum FlattenDone {
    Up(Vec<(Step, Result<Moved, String>)>),
    Back(Vec<(Done, Result<Moved, String>)>),
}

struct FlattenDialog {
    steps: Vec<Step>,
    kept: Vec<(Course, Kept)>,
}

#[derive(Default)]
pub(super) struct FlattenState {
    dialog: Option<FlattenDialog>,
    running: Option<Receiver<FlattenDone>>,
}

impl App {
    /// The switch in the settings; switched on, it offers to move the existing modules up.
    pub(super) fn flat_setting_ui(&mut self, ui: &mut egui::Ui) -> bool {
        let lang = self.lang;
        let mut changed = false;
        setting_row(ui, icon::FOLDER_SIMPLE, lang.flat_modules(), lang.flat_modules_hint(), |ui| {
            let busy = self.flatten.running.is_some();
            if busy {
                ui.add(egui::Spinner::new());
            }
            if ui.add_enabled_ui(!busy, |ui| fx::toggle(ui, &mut self.config.flat_modules)).inner.changed() {
                changed = true;
                if self.config.flat_modules {
                    self.offer_flatten();
                }
            }
        });
        changed
    }

    fn offer_flatten(&mut self) {
        let (steps, kept) = flatten::plan(&self.config);
        if !steps.is_empty() || !kept.is_empty() {
            self.flatten.dialog = Some(FlattenDialog { steps, kept });
        }
    }

    pub(super) fn flatten_dialog_ui(&mut self, ctx: &egui::Context) {
        let lang = self.lang;
        let Some(dialog) = &self.flatten.dialog else { return };
        let root = self.study_root();
        let mut start = false;
        let mut close = false;
        let modal = egui::Modal::new(Id::new("flatten-dialog")).frame(dialog_frame(ctx)).show(ctx, |ui| {
            let p = fx::pal(ui);
            ui.set_width(480.0);
            ui.label(fx::title(lang.flatten_title(), 22.0).color(p.on_surface));
            ui.add_space(8.0);
            if !dialog.steps.is_empty() {
                let text = lang.flatten_text(dialog.steps.len());
                ui.add(egui::Label::new(RichText::new(text).color(p.on_surface_variant)).wrap());
                ui.add_space(8.0);
            }
            egui::ScrollArea::vertical().max_height(300.0).show(ui, |ui| {
                for step in &dialog.steps {
                    let to = step.to.strip_prefix(&root).unwrap_or(&step.to).display().to_string();
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(icon::FOLDER).color(p.primary));
                        ui.label(RichText::new(step.course.label()).color(p.on_surface));
                        ui.label(RichText::new(format!("→  {to}")).color(p.on_surface_variant));
                    });
                }
                if !dialog.kept.is_empty() {
                    ui.add_space(8.0);
                    ui.label(RichText::new(lang.flatten_kept()).small().color(p.on_surface_variant));
                    for (course, why) in &dialog.kept {
                        let why = match why {
                            Kept::OwnFolder => lang.flatten_own_folder().to_string(),
                            Kept::Taken(path) => {
                                lang.flatten_taken(&path.strip_prefix(&root).unwrap_or(path).display().to_string())
                            }
                        };
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(icon::WARNING).color(p.warning));
                            ui.label(RichText::new(course.label()).color(p.on_surface));
                            ui.label(RichText::new(why).small().color(p.on_surface_variant));
                        });
                    }
                }
            });
            actions(ui, |ui| {
                if dialog.steps.is_empty() {
                    close |= fx::button(ui, ButtonStyle::Filled, "", lang.ok()).clicked();
                    return;
                }
                start |= fx::button(ui, ButtonStyle::Filled, "", &lang.flatten_move(dialog.steps.len())).clicked();
                close |= fx::button(ui, ButtonStyle::Text, "", lang.flatten_skip()).clicked();
            });
        });
        if start {
            let steps = self.flatten.dialog.take().map(|d| d.steps).unwrap_or_default();
            let (tx, rx) = channel();
            let ctx = self.ctx.clone();
            std::thread::spawn(move || {
                let results = steps
                    .into_iter()
                    .map(|step| {
                        let result = flatten::move_up(&step);
                        (step, result)
                    })
                    .collect();
                let _ = tx.send(FlattenDone::Up(results));
                ctx.request_repaint();
            });
            self.flatten.running = Some(rx);
        } else if close || modal.should_close() {
            self.flatten.dialog = None;
        }
    }

    /// «Undo» on the note: the modules go back into their programs' folders.
    pub(super) fn undo_flatten(&mut self, done: Vec<Done>) {
        let (tx, rx) = channel();
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            let results = done
                .into_iter()
                .rev()
                .map(|d| {
                    let result = flatten::move_back(&d.to, &d.from);
                    (d, result)
                })
                .collect();
            let _ = tx.send(FlattenDone::Back(results));
            ctx.request_repaint();
        });
        self.flatten.running = Some(rx);
    }

    /// Take in what the background step did.
    pub(super) fn flatten_logic(&mut self) {
        let Some(rx) = &self.flatten.running else { return };
        let Ok(finished) = rx.try_recv() else { return };
        self.flatten.running = None;
        let lang = self.lang;
        let mut problems = Vec::new();
        match finished {
            FlattenDone::Up(results) => {
                let mut done = Vec::new();
                for (step, result) in results {
                    match result {
                        Ok(moved) => {
                            self.after_folder_move(&moved, &mut problems);
                            let old = step.course.label();
                            if let Some(course) = self.config.courses.iter_mut().find(|c| **c == step.course) {
                                course.subject.clear();
                                let new = course.label();
                                self.relabel_module(&old, &new);
                            }
                            done.push(Done {
                                name: step.course.name.clone(),
                                subject: step.course.subject.clone(),
                                from: step.from,
                                to: step.to,
                            });
                        }
                        Err(e) => problems.push(format!("{}: {e}", step.course.label())),
                    }
                }
                let title = lang.flatten_done(done.len());
                let names: Vec<String> = done.iter().map(|d| d.name.clone()).collect();
                let mut details = names.join(" · ");
                if !problems.is_empty() {
                    details = format!("{details}\n{}", problems.join("\n"));
                }
                if !done.is_empty() {
                    self.note(Note::Flattened { title, details, done, undone: false });
                } else if !problems.is_empty() {
                    self.note(Note::Cloud { title, details, warn: true });
                }
            }
            FlattenDone::Back(results) => {
                for (d, result) in results {
                    match result {
                        Ok(moved) => {
                            self.after_folder_move(&moved, &mut problems);
                            if let Some(course) =
                                self.config.courses.iter_mut().find(|c| c.subject.is_empty() && c.name == d.name)
                            {
                                let old = course.label();
                                course.subject = d.subject.clone();
                                let new = course.label();
                                self.relabel_module(&old, &new);
                            }
                        }
                        Err(e) => problems.push(format!("{}: {e}", d.name)),
                    }
                }
                self.config.flat_modules = false;
                if !problems.is_empty() {
                    let title = lang.flatten_undone().to_string();
                    self.note(Note::Cloud { title, details: problems.join("\n"), warn: true });
                }
            }
        }
        self.select(None);
        self.sort_courses();
        self.save_quietly();
    }
}
