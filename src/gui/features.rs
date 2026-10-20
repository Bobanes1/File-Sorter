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
        let busy = self.resort_busy();
        let f = &mut self.config.features;
        let mut changed = false;
        let mut action = super::subfolders::SubfolderAction::None;
        changed |= switch(ui, icon::BRAIN, lang.feature_learn(), lang.feature_learn_hint(), &mut f.learn);
        divider(ui);
        changed |= switch(ui, icon::GLOBE, lang.feature_origin(), lang.feature_origin_hint(), &mut f.origin);
        divider(ui);
        let (title, hint) = (lang.feature_subfolders(), lang.feature_subfolders_hint());
        changed |= switch(ui, icon::TREE_STRUCTURE, title, hint, &mut f.subfolders);
        if f.subfolders {
            action = super::subfolders::subfolder_list_ui(ui, lang, &mut f.subfolder_list, busy, &mut changed);
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
        self.subfolder_action(action);
        changed
    }
}

/// A feature's row with its switch. Returns true if it was switched.
fn switch(ui: &mut egui::Ui, glyph: &str, title: &str, hint: &str, on: &mut bool) -> bool {
    let mut changed = false;
    setting_row(ui, glyph, title, hint, |ui| changed = fx::toggle(ui, on).changed());
    changed
}
