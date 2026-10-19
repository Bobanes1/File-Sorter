//! The «Features» section of the settings: one switch per feature, with its own options
//! indented underneath while it is on.

use eframe::egui::{self, Margin, RichText, vec2};
use egui_phosphor::regular as icon;

use super::App;
use super::settings::{divider, setting_row};
use crate::config::{Features, UpdateMode};
use crate::fx::{self, ButtonStyle};
use crate::i18n::Lang;

impl App {
    /// Returns true if a setting changed.
    pub(super) fn features_ui(&mut self, ui: &mut egui::Ui) -> bool {
        let lang = self.lang;
        let f = &mut self.config.features;
        let mut changed = false;
        changed |= switch(ui, icon::BRAIN, lang.feature_learn(), lang.feature_learn_hint(), &mut f.learn);
        divider(ui);
        changed |= switch(ui, icon::GLOBE, lang.feature_origin(), lang.feature_origin_hint(), &mut f.origin);
        divider(ui);
        let (title, hint) = (lang.feature_subfolders(), lang.feature_subfolders_hint());
        changed |= switch(ui, icon::TREE_STRUCTURE, title, hint, &mut f.subfolders);
        if f.subfolders {
            changed |= subfolder_names_ui(ui, lang, f);
        }
        divider(ui);
        changed |= switch(ui, icon::FILE_ZIP, lang.feature_unzip(), lang.feature_unzip_hint(), &mut f.unzip);
        divider(ui);
        let (title, hint) = (lang.feature_versions(), lang.feature_versions_hint());
        changed |= switch(ui, icon::CLOCK_COUNTER_CLOCKWISE, title, hint, &mut f.versions);
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
        changed
    }
}

/// A feature's row with its switch. Returns true if it was switched.
fn switch(ui: &mut egui::Ui, glyph: &str, title: &str, hint: &str, on: &mut bool) -> bool {
    let mut changed = false;
    setting_row(ui, glyph, title, hint, |ui| changed = fx::toggle(ui, on).changed());
    changed
}

/// The four subfolder names, editable, with «Defaults» in the window's language.
fn subfolder_names_ui(ui: &mut egui::Ui, lang: Lang, f: &mut Features) -> bool {
    let p = fx::pal(ui);
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.add_space(40.0);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(8.0, 8.0);
            for name in &mut f.subfolder_names {
                let field = egui::TextEdit::singleline(name).desired_width(120.0).margin(Margin::symmetric(10, 6));
                changed |= ui.add(field).changed();
            }
            if fx::button(ui, ButtonStyle::Text, icon::ARROW_COUNTER_CLOCKWISE, lang.defaults()).clicked() {
                f.subfolder_names = lang.subfolder_defaults().map(str::to_string);
                changed = true;
            }
        });
    });
    ui.horizontal(|ui| {
        ui.add_space(40.0);
        ui.add(egui::Label::new(RichText::new(lang.subfolder_names_hint()).small().color(p.on_surface_variant)).wrap());
    });
    ui.add_space(8.0);
    changed
}
