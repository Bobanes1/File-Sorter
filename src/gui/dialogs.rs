//! Dialogs: add/edit and delete a module, errors, and the keyword preview.

use eframe::egui::{self, Align, Id, Layout, Margin, RichText, Sense, vec2};
use egui_phosphor::regular as icon;

use super::{App, ErrorBox};
use crate::config::{Course, scan_study_folder};
use crate::fx::{self, ButtonStyle};
use crate::watcher::StartError;

/// A Material dialog: rounded, tinted surface with a soft shadow.
pub(super) fn dialog_frame(ctx: &egui::Context) -> egui::Frame {
    let p = fx::pal_ctx(ctx);
    egui::Frame::new()
        .fill(p.container_high)
        .corner_radius(egui::CornerRadius::same(28))
        .inner_margin(Margin::same(24))
        .shadow(egui::Shadow { offset: [0, 6], blur: 28, spread: 0, color: egui::Color32::from_black_alpha(70) })
}

/// Dialog buttons, right-aligned as in Material.
pub(super) fn actions(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui)) {
    ui.add_space(16.0);
    ui.with_layout(Layout::right_to_left(Align::Center), add);
}

impl App {
    pub(super) fn dialogs_ui(&mut self, ctx: &egui::Context) {
        self.course_dialog(ctx);
        self.delete_dialog(ctx);
        self.keyword_dialog(ctx);
        self.error_dialog(ctx);
    }

    fn course_dialog(&mut self, ctx: &egui::Context) {
        let lang = self.lang;
        if self.dialog.is_none() {
            return;
        }
        let subjects = self.subjects();
        let mut close = false;
        let mut ok = false;
        let title =
            if self.dialog.as_ref().unwrap().editing.is_some() { lang.edit_module() } else { lang.add_module() };
        let modal = egui::Modal::new(Id::new("course-dialog")).frame(dialog_frame(ctx)).show(ctx, |ui| {
            let p = fx::pal(ui);
            ui.set_width(420.0);
            let d = self.dialog.as_mut().unwrap();
            ui.label(fx::title(title, 22.0).color(p.on_surface));
            ui.add_space(12.0);
            ui.label(RichText::new(lang.study_program()).small().color(p.on_surface_variant));
            ui.horizontal(|ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut d.subject).desired_width(330.0).margin(Margin::symmetric(12, 8)),
                );
                egui::ComboBox::from_id_salt("subject-choices").selected_text("").width(28.0).show_ui(ui, |ui| {
                    for s in &subjects {
                        if ui.selectable_label(d.subject == *s, s).clicked() {
                            d.subject = s.clone();
                        }
                    }
                });
            });
            ui.add_space(6.0);
            ui.label(RichText::new(lang.module()).small().color(p.on_surface_variant));
            let name = ui.add(
                egui::TextEdit::singleline(&mut d.name).desired_width(f32::INFINITY).margin(Margin::symmetric(12, 8)),
            );
            if !d.focused {
                name.request_focus();
                d.focused = true;
            }
            if name.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                ok = true;
            }
            let valid = !d.name.trim().is_empty();
            actions(ui, |ui| {
                if ui.add_enabled_ui(valid, |ui| fx::button(ui, ButtonStyle::Filled, "", lang.ok())).inner.clicked() {
                    ok = true;
                }
                if fx::button(ui, ButtonStyle::Text, "", lang.cancel()).clicked() {
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

    fn delete_dialog(&mut self, ctx: &egui::Context) {
        let lang = self.lang;
        let Some(i) = self.confirm_delete else { return };
        let label = self.config.courses.get(i).map(Course::label).unwrap_or_default();
        let modal = egui::Modal::new(Id::new("confirm-delete")).frame(dialog_frame(ctx)).show(ctx, |ui| {
            let p = fx::pal(ui);
            ui.set_width(380.0);
            ui.label(fx::title(lang.delete_module(), 22.0).color(p.on_surface));
            ui.add_space(8.0);
            ui.label(RichText::new(lang.delete_confirm(&label)).color(p.on_surface_variant));
            let mut result = (false, false);
            actions(ui, |ui| {
                result.0 = fx::button(ui, ButtonStyle::Text, "", lang.delete()).clicked();
                result.1 = fx::button(ui, ButtonStyle::Text, "", lang.cancel()).clicked();
            });
            result
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

    /// The found keywords as a checklist; "Add" puts the ticked ones into the module.
    fn keyword_dialog(&mut self, ctx: &egui::Context) {
        let lang = self.lang;
        let Some(preview) = &mut self.keyword_preview else { return };
        let mut add = false;
        let mut cancel = false;
        let modal = egui::Modal::new(Id::new("keyword-preview")).frame(dialog_frame(ctx)).show(ctx, |ui| {
            let p = fx::pal(ui);
            ui.set_width(460.0);
            ui.label(fx::title(lang.kw_preview_title(&preview.course.label()), 22.0).color(p.on_surface));
            ui.add_space(6.0);
            ui.label(RichText::new(lang.kw_preview_text(preview.suggestion.files_scanned)).color(p.on_surface_variant));
            ui.add_space(8.0);
            let total = preview.suggestion.files_scanned;
            for (i, keyword) in preview.suggestion.keywords.iter().enumerate() {
                let chosen = &mut preview.chosen[i];
                let response = ui
                    .horizontal(|ui| {
                        ui.set_min_height(48.0);
                        let (box_rect, _) = ui.allocate_exact_size(vec2(24.0, 24.0), Sense::hover());
                        let t = ui.ctx().animate_bool_with_time(
                            Id::new(("kw-check", i)),
                            *chosen,
                            fx::anim_time(ui.ctx(), 0.12),
                        );
                        let color = fx::lerp_color(p.on_surface_variant, p.primary, t);
                        let square = egui::Rect::from_center_size(box_rect.center(), vec2(18.0, 18.0));
                        ui.painter().rect_stroke(square, 3, egui::Stroke::new(2.0, color), egui::StrokeKind::Inside);
                        if t > 0.0 {
                            ui.painter().rect_filled(square, 3, p.primary.gamma_multiply(t));
                            ui.painter().text(
                                square.center(),
                                egui::Align2::CENTER_CENTER,
                                icon::CHECK,
                                egui::FontId::proportional(14.0),
                                p.on_primary.gamma_multiply(t),
                            );
                        }
                        ui.add_space(8.0);
                        ui.vertical(|ui| {
                            ui.spacing_mut().item_spacing.y = 0.0;
                            ui.add_space(4.0);
                            ui.label(fx::title(&keyword.term, 15.0).color(p.on_surface));
                            let detail =
                                format!("{} · {}", lang.kw_kind(keyword.kind), lang.in_files(keyword.files, total));
                            ui.label(RichText::new(detail).small().color(p.on_surface_variant));
                        });
                    })
                    .response;
                let rect = response.rect.expand2(vec2(6.0, 0.0));
                let click = ui.interact(rect, Id::new(("kw-row", i)), Sense::click());
                fx::state_layer(ui, &click, rect, 8, p.on_surface);
                if click.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                    *chosen = !*chosen;
                }
            }
            let count = preview.chosen.iter().filter(|c| **c).count();
            actions(ui, |ui| {
                let label = lang.kw_add(count);
                if ui
                    .add_enabled_ui(count > 0, |ui| fx::button(ui, ButtonStyle::Filled, icon::PLUS, &label))
                    .inner
                    .clicked()
                {
                    add = true;
                }
                if fx::button(ui, ButtonStyle::Text, "", lang.cancel()).clicked() {
                    cancel = true;
                }
            });
        });
        if add {
            let preview = self.keyword_preview.take().unwrap();
            let words: Vec<String> = preview
                .suggestion
                .keywords
                .into_iter()
                .zip(preview.chosen)
                .filter(|(_, chosen)| *chosen)
                .map(|(k, _)| k.term)
                .collect();
            self.add_keywords(&preview.course, words);
        } else if cancel || modal.should_close() {
            self.keyword_preview = None;
        }
    }

    fn error_dialog(&mut self, ctx: &egui::Context) {
        let lang = self.lang;
        let Some(error) = &self.error else { return };
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
        let modal = egui::Modal::new(Id::new("error")).frame(dialog_frame(ctx)).show(ctx, |ui| {
            let p = fx::pal(ui);
            ui.set_width(400.0);
            ui.vertical_centered(|ui| {
                ui.label(RichText::new(icon::WARNING_CIRCLE).size(28.0).color(p.error));
                ui.label(fx::title(title, 22.0).color(p.on_surface));
            });
            ui.add_space(8.0);
            ui.label(RichText::new(text).color(p.on_surface_variant));
            let mut ok = false;
            actions(ui, |ui| ok = fx::button(ui, ButtonStyle::Text, "", lang.ok()).clicked());
            ok
        });
        if modal.inner || modal.should_close() {
            self.error = None;
        }
    }
}
