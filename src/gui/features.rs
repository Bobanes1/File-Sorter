//! The «Features» section of the settings: one switch per feature, with its own options
//! indented underneath while it is on.

use eframe::egui::{self, Margin, RichText};
use egui_phosphor::regular as icon;

use super::App;
use super::settings::{divider, setting_row};
use crate::config::UpdateMode;
use crate::fx;

impl App {
    /// Returns true if a setting changed.
    pub(super) fn features_ui(&mut self, ui: &mut egui::Ui) -> bool {
        let lang = self.lang;
        let laya_dir = crate::laya::dir(&self.config);
        let f = &mut self.config.features;
        let mut changed = false;
        changed |= switch(ui, icon::BRAIN, lang.feature_learn(), lang.feature_learn_hint(), &mut f.learn);
        divider(ui);
        let (title, hint) = (lang.feature_classifier(), lang.feature_classifier_hint());
        changed |= switch(ui, icon::GRAPH, title, hint, &mut f.classifier);
        divider(ui);
        let laya = crate::laya::status(laya_dir.as_deref());
        if switch(ui, icon::ROBOT, lang.feature_laya(), &lang.feature_laya_hint(&laya), &mut f.laya) {
            if !f.laya {
                crate::laya::forget();
            } else if let Some(dir) = laya_dir {
                // Start the download right away (`App::logic` keeps it up to date later).
                crate::laya::keep_up(dir, f.update_check, true);
            }
            changed = true;
        }
        if let crate::laya::Status::Downloading(done, total) = laya {
            ui.horizontal(|ui| {
                ui.add_space(40.0);
                ui.vertical(|ui| {
                    // Sliding until the first bytes arrive, then how far it is.
                    let fraction = (done > 0).then(|| done as f32 / total as f32);
                    fx::progress_bar(ui, egui::Id::new("laya-download"), fraction);
                    ui.add_space(2.0);
                    let text = lang.laya_download_progress(done / 1_000_000, total / 1_000_000);
                    ui.label(RichText::new(text).small().color(fx::pal(ui).on_surface_variant));
                });
            });
            ui.add_space(4.0);
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(500));
        }
        divider(ui);
        let (title, hint) = (lang.feature_ask_unsure(), lang.feature_ask_unsure_hint());
        changed |= switch(ui, icon::SEAL_QUESTION, title, hint, &mut f.ask_when_unsure);
        divider(ui);
        let (title, hint) = (lang.feature_timetable_hint(), lang.timetable_sorting_note());
        changed |= switch(ui, icon::CHALKBOARD_TEACHER, title, hint, &mut f.timetable_hint);
        divider(ui);
        changed |= switch(ui, icon::GLOBE, lang.feature_origin(), lang.feature_origin_hint(), &mut f.origin);
        divider(ui);
        let (title, hint) = (lang.feature_subfolders(), lang.feature_subfolders_hint());
        changed |= switch(ui, icon::TREE_STRUCTURE, title, hint, &mut f.subfolders);
        if f.subfolders {
            // The list, creating and sorting in are in the overview, at the module.
            ui.horizontal(|ui| {
                ui.add_space(40.0);
                ui.label(RichText::new(lang.subfolders_managed_in_overview()).small().color(fx::pal(ui).primary));
            });
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.add_space(40.0);
                ui.vertical(|ui| {
                    let (title, hint) = (lang.feature_auto_learn(), lang.feature_auto_learn_hint());
                    changed |= switch(ui, icon::BRAIN, title, hint, &mut f.subfolder_auto_learn);
                });
            });
        }
        divider(ui);
        let ocr_hint = match crate::ocr::available() {
            crate::ocr::Engine::Missing => lang.feature_ocr_missing(),
            _ => lang.feature_ocr_hint(),
        };
        if switch(ui, icon::SCAN, lang.feature_ocr(), ocr_hint, &mut f.ocr) {
            crate::ocr::set_enabled(f.ocr);
            changed = true;
        }
        divider(ui);
        changed |= switch(ui, icon::FILE_ZIP, lang.feature_unzip(), lang.feature_unzip_hint(), &mut f.unzip);
        divider(ui);
        let (title, hint) = (lang.feature_versions(), lang.feature_versions_hint());
        changed |= switch(ui, icon::CLOCK_COUNTER_CLOCKWISE, title, hint, &mut f.versions);
        if f.versions {
            ui.horizontal(|ui| {
                ui.add_space(40.0);
                ui.label(RichText::new(lang.versions_folder_hint()).color(fx::pal(ui).on_surface_variant));
                let field = egui::TextEdit::singleline(&mut f.versions_folder)
                    .desired_width(200.0)
                    .margin(Margin::symmetric(10, 6));
                changed |= ui.add(field).changed();
            });
            ui.add_space(8.0);
        }
        divider(ui);
        changed |= switch(ui, icon::MAGNIFYING_GLASS, lang.feature_search(), lang.feature_search_hint(), &mut f.search);
        divider(ui);
        let (title, hint) = (lang.feature_deadlines(), lang.feature_deadlines_hint());
        changed |= switch(ui, icon::CALENDAR_CHECK, title, hint, &mut f.deadlines);
        if f.deadlines {
            ui.horizontal(|ui| {
                ui.add_space(40.0);
                ui.vertical(|ui| {
                    let (title, hint) = (lang.feature_reminders(), lang.feature_reminders_hint());
                    changed |= switch(ui, icon::BELL_RINGING, title, hint, &mut f.reminders);
                });
            });
            if f.reminders {
                ui.horizontal_wrapped(|ui| {
                    ui.add_space(80.0);
                    ui.spacing_mut().item_spacing = egui::vec2(6.0, 6.0);
                    for lead in crate::reminders::Lead::ALL {
                        let on = f.reminder_leads.contains(&lead);
                        if fx::chip(ui, on, lang.lead(lead)).clicked() {
                            if on {
                                f.reminder_leads.retain(|l| *l != lead);
                            } else {
                                f.reminder_leads.push(lead);
                            }
                            changed = true;
                        }
                    }
                });
                ui.add_space(8.0);
            }
        }
        divider(ui);
        changed |= switch(ui, icon::CALENDAR_DOTS, lang.feature_weekly(), lang.feature_weekly_hint(), &mut f.weekly);
        divider(ui);
        let (title, hint) = (lang.feature_update(), lang.feature_update_hint());
        changed |= switch(ui, icon::ARROW_CIRCLE_UP, title, hint, &mut f.update_check);
        if f.update_check {
            ui.horizontal(|ui| {
                ui.add_space(40.0);
                let options = [
                    (UpdateMode::Ask, lang.update_ask().to_string()),
                    (UpdateMode::Auto, lang.update_auto().to_string()),
                ];
                changed |= fx::segmented(ui, "update-mode", &mut f.update_install, &options);
            });
            ui.add_space(8.0);
        }
        changed
    }
}

/// A feature's row with its switch. Returns true if it was switched.
fn switch(ui: &mut egui::Ui, glyph: &str, title: &str, hint: &str, on: &mut bool) -> bool {
    let mut changed = false;
    setting_row(ui, glyph, title, hint, |ui| changed = fx::toggle(ui, on).changed());
    changed
}
