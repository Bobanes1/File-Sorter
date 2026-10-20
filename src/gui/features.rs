//! The «Features» section of the settings: one switch per feature, with its own options
//! indented underneath while it is on.

use eframe::egui::{self, Id, Margin, RichText};
use egui_phosphor::regular as icon;

use super::App;
use super::settings::{divider, setting_row};
use crate::config::UpdateMode;
use crate::deadlines::Category;
use crate::fx;
use crate::fx::ButtonStyle;
use crate::i18n::Lang;

impl App {
    /// Returns true if a setting changed.
    pub(super) fn features_ui(&mut self, ui: &mut egui::Ui) -> bool {
        let lang = self.lang;
        let mut changed = self.laya_setting_ui(ui);
        divider(ui);
        let f = &mut self.config.features;
        changed |= switch(ui, icon::BRAIN, lang.feature_learn(), lang.feature_learn_hint(), &mut f.learn);
        divider(ui);
        let (title, hint) = (lang.feature_classifier(), lang.feature_classifier_hint());
        changed |= switch(ui, icon::GRAPH, title, hint, &mut f.classifier);
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
            ui.horizontal(|ui| {
                ui.add_space(40.0);
                ui.vertical(|ui| changed |= categories_ui(ui, lang, &mut f.deadline_categories));
            });
            ui.add_space(8.0);
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

/// The categories deadlines are put into: name, words, «be there», delete; «+ Category» and
/// «Defaults». Returns true if one changed.
fn categories_ui(ui: &mut egui::Ui, lang: Lang, list: &mut Vec<Category>) -> bool {
    let p = fx::pal(ui);
    let mut changed = false;
    let mut remove = None;
    ui.label(RichText::new(lang.categories_title()).strong().color(p.on_surface));
    ui.add(egui::Label::new(RichText::new(lang.categories_hint()).small().color(p.on_surface_variant)).wrap());
    ui.add_space(4.0);
    let only_one = list.len() <= 1;
    for (i, c) in list.iter_mut().enumerate() {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            // The name as shown (the built-in one in this language until it is changed).
            let shown = lang.category_name(c);
            let mut name = if c.name.is_empty() { shown.clone() } else { c.name.clone() };
            let field = egui::TextEdit::singleline(&mut name).desired_width(150.0).margin(Margin::symmetric(10, 6));
            if ui.add(field).changed() {
                c.name = if name.trim() == shown && c.builtin.is_some() { String::new() } else { name };
                changed = true;
            }
            let id = Id::new(("category-words", i));
            let width = (ui.available_width() - 150.0).max(120.0);
            changed |= words_field(ui, id, lang.category_words_hint(), &mut c.keywords, width);
            let be_there = fx::chip(ui, c.appointment, lang.category_appointment());
            if be_there.on_hover_text(lang.category_appointment_hint()).clicked() {
                c.appointment = !c.appointment;
                changed = true;
            }
            let trash = ui.add_enabled_ui(!only_one, |ui| fx::icon_button(ui, icon::TRASH)).inner;
            if trash.on_hover_text(lang.delete()).clicked() {
                remove = Some(i);
            }
        });
    }
    if let Some(i) = remove {
        list.remove(i);
        changed = true;
    }
    ui.horizontal(|ui| {
        if fx::button(ui, ButtonStyle::Text, icon::PLUS, lang.category_add()).clicked() {
            let new = Category::custom(lang.category_new_name(), list);
            list.push(new);
            changed = true;
        }
        if *list != Category::defaults()
            && fx::button(ui, ButtonStyle::Text, icon::ARROW_COUNTER_CLOCKWISE, lang.defaults()).clicked()
        {
            *list = Category::defaults();
            changed = true;
        }
    });
    changed
}

/// Words as one text («exkursion, ausflug»), kept as typed while the field has the focus.
pub(super) fn words_field(ui: &mut egui::Ui, id: Id, hint: &str, words: &mut Vec<String>, width: f32) -> bool {
    let mut text = ui.data(|d| d.get_temp::<String>(id)).unwrap_or_else(|| words.join(", "));
    let field =
        egui::TextEdit::singleline(&mut text).hint_text(hint).desired_width(width).margin(Margin::symmetric(10, 6));
    let response = ui.add(field);
    let changed = response.changed();
    if changed {
        *words = text.split(',').map(|w| w.trim().to_string()).filter(|w| !w.is_empty()).collect();
    }
    if !response.has_focus() {
        text = words.join(", ");
    }
    ui.data_mut(|d| d.insert_temp(id, text));
    changed
}
