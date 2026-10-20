//! «Finish semester»: pick the modules that are over and move them into the archive;
//! archived modules are listed in the settings and can be brought back.

use std::sync::mpsc::{Receiver, channel};

use eframe::egui::{self, Id, Margin, RichText};
use egui_phosphor::regular as icon;

use super::dialogs::{actions, dialog_frame};
use super::settings::{divider, setting_row};
use super::{App, Note};
use crate::archive::{self, Archived, Moved};
use crate::config::Course;
use crate::fx::{self, ButtonStyle};

/// What a finished background step reports.
pub(super) enum ArchiveDone {
    Archived(Vec<ArchiveResult>),
    Restored(Box<Archived>, Result<Moved, String>),
}

/// One module and how archiving it went.
pub(super) type ArchiveResult = (Course, Result<(Archived, Moved), String>);

pub(super) struct ArchiveDialog {
    semester: String,
    chosen: Vec<bool>,
}

#[derive(Default)]
pub(super) struct ArchiveState {
    dialog: Option<ArchiveDialog>,
    running: Option<Receiver<ArchiveDone>>,
}

impl App {
    /// The «Semester» section of the settings.
    pub(super) fn archive_settings_ui(&mut self, ui: &mut egui::Ui) {
        let lang = self.lang;
        let p = fx::pal(ui);
        let busy = self.archive.running.is_some();
        setting_row(ui, icon::ARCHIVE, lang.finish_semester(), lang.finish_semester_hint(), |ui| {
            let button = ui.add_enabled_ui(!busy && !self.config.courses.is_empty(), |ui| {
                fx::button(ui, ButtonStyle::Tonal, "", lang.finish_semester_button())
            });
            if button.inner.clicked() {
                let today = chrono::Local::now().date_naive();
                self.archive.dialog = Some(ArchiveDialog {
                    semester: archive::current_semester(today),
                    chosen: vec![false; self.config.courses.len()],
                });
            }
            if busy {
                ui.add(egui::Spinner::new());
            }
        });
        if self.config.archived.is_empty() {
            return;
        }
        let mut semesters: Vec<String> = self.config.archived.iter().map(|a| a.semester.clone()).collect();
        semesters.sort();
        semesters.dedup();
        let mut restore = None;
        for semester in semesters.iter().rev() {
            divider(ui);
            ui.horizontal(|ui| {
                ui.add_space(40.0);
                ui.label(RichText::new(semester).family(fx::medium()).color(p.on_surface));
            });
            for (i, a) in self.config.archived.iter().enumerate().filter(|(_, a)| &a.semester == semester) {
                ui.horizontal(|ui| {
                    ui.add_space(52.0);
                    ui.label(RichText::new(a.course.label()).color(p.on_surface_variant));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let enabled = !busy;
                        let clicked = ui
                            .add_enabled_ui(enabled, |ui| {
                                fx::button(ui, ButtonStyle::Text, icon::ARROW_COUNTER_CLOCKWISE, lang.restore())
                            })
                            .inner
                            .clicked();
                        if clicked {
                            restore = Some(i);
                        }
                        if fx::button(ui, ButtonStyle::Text, icon::FOLDER_OPEN, "")
                            .on_hover_text(lang.open_folder_hint())
                            .clicked()
                        {
                            crate::desktop::open(&a.folder);
                        }
                    });
                });
            }
        }
        if let Some(i) = restore {
            let archived = self.config.archived[i].clone();
            let (tx, rx) = channel();
            let ctx = self.ctx.clone();
            std::thread::spawn(move || {
                let result = archive::restore(&archived);
                let _ = tx.send(ArchiveDone::Restored(Box::new(archived), result));
                ctx.request_repaint();
            });
            self.archive.running = Some(rx);
        }
    }

    pub(super) fn archive_dialog_ui(&mut self, ctx: &egui::Context) {
        let lang = self.lang;
        let Some(dialog) = &mut self.archive.dialog else { return };
        let mut close = false;
        let mut start = false;
        let modal = egui::Modal::new(Id::new("archive-dialog")).frame(dialog_frame(ctx)).show(ctx, |ui| {
            let p = fx::pal(ui);
            ui.set_width(460.0);
            ui.label(fx::title(lang.finish_semester(), 22.0).color(p.on_surface));
            ui.add_space(8.0);
            ui.add(egui::Label::new(RichText::new(lang.archive_explained()).color(p.on_surface_variant)).wrap());
            ui.add_space(12.0);
            ui.label(RichText::new(lang.semester_name()).small().color(p.on_surface_variant));
            ui.add(
                egui::TextEdit::singleline(&mut dialog.semester).desired_width(160.0).margin(Margin::symmetric(12, 8)),
            );
            ui.add_space(8.0);
            ui.label(RichText::new(lang.modules_to_archive()).small().color(p.on_surface_variant));
            egui::ScrollArea::vertical().max_height(280.0).show(ui, |ui| {
                for (i, course) in self.config.courses.iter().enumerate() {
                    if let Some(on) = dialog.chosen.get_mut(i) {
                        ui.checkbox(on, course.label());
                    }
                }
            });
            actions(ui, |ui| {
                let count = dialog.chosen.iter().filter(|c| **c).count();
                let ready = count > 0 && !crate::categories::safe_folder_name(&dialog.semester).is_empty();
                let label = lang.archive_count(count);
                if ui.add_enabled_ui(ready, |ui| fx::button(ui, ButtonStyle::Filled, "", &label)).inner.clicked() {
                    start = true;
                }
                if fx::button(ui, ButtonStyle::Text, "", lang.cancel()).clicked() {
                    close = true;
                }
            });
        });
        if start {
            let semester = dialog.semester.trim().to_string();
            let courses: Vec<Course> =
                self.config.courses.iter().zip(&dialog.chosen).filter(|(_, on)| **on).map(|(c, _)| c.clone()).collect();
            let config = self.config.clone();
            let (tx, rx) = channel();
            let ctx = self.ctx.clone();
            std::thread::spawn(move || {
                let results = courses
                    .into_iter()
                    .map(|course| {
                        let result = archive::archive(&config, &course, &semester);
                        (course, result)
                    })
                    .collect();
                let _ = tx.send(ArchiveDone::Archived(results));
                ctx.request_repaint();
            });
            self.archive.running = Some(rx);
            self.archive.dialog = None;
        } else if close || modal.should_close() {
            self.archive.dialog = None;
        }
    }

    /// Take in what the background step did.
    pub(super) fn archive_logic(&mut self) {
        let Some(rx) = &self.archive.running else { return };
        let Ok(done) = rx.try_recv() else { return };
        self.archive.running = None;
        let lang = self.lang;
        match done {
            ArchiveDone::Archived(results) => {
                let mut done_names = Vec::new();
                let mut problems = Vec::new();
                for (course, result) in results {
                    match result {
                        Ok((archived, moved)) => {
                            self.after_folder_move(&moved, &mut problems);
                            self.config.courses.retain(|c| *c != course);
                            done_names.push(course.label());
                            self.config.archived.push(archived);
                        }
                        Err(e) => problems.push(format!("{}: {e}", course.label())),
                    }
                }
                self.select(None);
                let title = lang.archived_note(done_names.len());
                let mut details = done_names.join(" · ");
                if !problems.is_empty() {
                    details = format!("{details}\n{}", problems.join("\n"));
                }
                self.note(Note::Cloud { title, details, warn: !problems.is_empty() });
            }
            ArchiveDone::Restored(archived, result) => match result {
                Ok(moved) => {
                    let mut problems = Vec::new();
                    self.after_folder_move(&moved, &mut problems);
                    self.config.archived.retain(|a| *a != *archived);
                    if !self.config.courses.contains(&archived.course) {
                        self.config.courses.push(archived.course.clone());
                        self.sort_courses();
                    }
                    let title = lang.restored_note(&archived.course.label());
                    self.note(Note::Cloud { title, details: problems.join("\n"), warn: !problems.is_empty() });
                }
                Err(e) => {
                    let title = lang.restore_failed(&archived.course.label());
                    self.note(Note::Cloud { title, details: e, warn: true });
                }
            },
        }
        self.save_quietly();
    }

    /// A module's folder moved: the history and the activity list follow it; files that
    /// stayed (open in another program) are listed.
    pub(super) fn after_folder_move(&mut self, moved: &Moved, problems: &mut Vec<String>) {
        let Some(report) = &moved.report else { return };
        if let Some(dir) = &self.config.state_dir
            && let Err(e) = crate::history::record_relocation(dir, &moved.from, &moved.to, &report.renamed)
        {
            log::info!("Could not note the move in the history: {e}");
        }
        self.relocate_cards(&moved.from, &moved.to, &report.renamed);
        let gone: Vec<_> = moved.files.iter().filter(|f| !f.exists()).cloned().collect();
        self.cloud_forget(gone);
        for (path, why) in &report.left {
            problems.push(self.lang.stayed_behind(&path.display().to_string(), why));
        }
    }
}
