//! The setup at the first start (and from the settings): downloads folder, study folder
//! (in a cloud folder if wanted), modules, and whether to start watching. Can be skipped.

use eframe::egui::{self, Id, Margin, RichText};
use egui_phosphor::regular as icon;

use super::App;
use super::dialogs::{actions, dialog_frame};
use crate::cloud::CloudMode;
use crate::config::expand_user;
use crate::fx::{self, ButtonStyle};

pub(super) struct Wizard {
    step: usize,
    start_watching: bool,
    /// Modules found when the study folder was read (none yet: `None`).
    found: Option<usize>,
    /// The layout was guessed from the study folder (once, when the modules step opens).
    layout_guessed: bool,
}

impl Wizard {
    pub(super) fn new() -> Wizard {
        Wizard { step: 0, start_watching: true, found: None, layout_guessed: false }
    }
}

const STEPS: usize = 3;

impl App {
    pub(super) fn wizard_ui(&mut self, ctx: &egui::Context) {
        let lang = self.lang;
        let Some(step) = self.wizard.as_ref().map(|w| w.step) else { return };
        let mut next = false;
        let mut back = false;
        let mut skip = false;
        let mut changed = false;
        egui::Modal::new(Id::new("wizard")).frame(dialog_frame(ctx)).show(ctx, |ui| {
            let p = fx::pal(ui);
            ui.set_width(520.0);
            ui.label(RichText::new(lang.setup_step(step + 1, STEPS)).small().color(p.on_surface_variant));
            ui.add_space(4.0);
            // Steps as dots.
            ui.horizontal(|ui| {
                for i in 0..STEPS {
                    let (rect, _) = ui.allocate_exact_size(egui::vec2(28.0, 6.0), egui::Sense::hover());
                    let color = if i <= step { p.primary } else { p.outline_variant };
                    ui.painter().rect_filled(rect, egui::CornerRadius::same(3), color);
                }
            });
            ui.add_space(12.0);
            match step {
                0 => {
                    ui.label(fx::title(lang.setup_welcome(), 22.0).color(p.on_surface));
                    ui.add_space(6.0);
                    ui.add(
                        egui::Label::new(RichText::new(lang.setup_downloads_text()).color(p.on_surface_variant)).wrap(),
                    );
                    ui.add_space(12.0);
                    changed |= self.folder_field(ui, true);
                }
                1 => {
                    ui.label(fx::title(lang.setup_study_title(), 22.0).color(p.on_surface));
                    ui.add_space(6.0);
                    ui.add(egui::Label::new(RichText::new(lang.setup_study_text()).color(p.on_surface_variant)).wrap());
                    ui.add_space(12.0);
                    changed |= self.folder_field(ui, false);
                    changed |= self.cloud_suggestions(ui);
                }
                _ => {
                    ui.label(fx::title(lang.setup_modules_title(), 22.0).color(p.on_surface));
                    ui.add_space(6.0);
                    ui.add(
                        egui::Label::new(RichText::new(lang.setup_modules_text()).color(p.on_surface_variant)).wrap(),
                    );
                    ui.add_space(12.0);
                    if let Some(w) = &mut self.wizard
                        && !w.layout_guessed
                    {
                        w.layout_guessed = true;
                        if self.config.courses.is_empty() {
                            let root = expand_user(self.config.destination_root.trim());
                            self.config.flat_modules =
                                crate::config::looks_flat(&root, &self.config.features.subfolder_list);
                        }
                    }
                    ui.label(RichText::new(lang.layout_label()).small().color(p.on_surface_variant));
                    let layouts = [(false, lang.layout_programs().to_string()), (true, lang.layout_flat().to_string())];
                    changed |= fx::segmented(ui, "wizard-layout", &mut self.config.flat_modules, &layouts);
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if fx::button(ui, ButtonStyle::Tonal, icon::FOLDERS, lang.import_modules()).clicked() {
                            let found = self.scan_modules();
                            let total = found.len();
                            crate::config::merge_courses(&mut self.config.courses, found);
                            self.sort_courses();
                            if let Some(w) = &mut self.wizard {
                                w.found = Some(total);
                            }
                            changed = true;
                        }
                        if let Some(found) = self.wizard.as_ref().and_then(|w| w.found) {
                            ui.label(RichText::new(lang.setup_found(found)).color(p.on_surface_variant));
                        }
                    });
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new(lang.setup_modules_count(self.config.courses.len())).small().color(p.outline),
                    );
                    ui.add_space(12.0);
                    if let Some(w) = &mut self.wizard {
                        ui.checkbox(&mut w.start_watching, lang.setup_start_watching());
                    }
                    if crate::desktop::autostart_supported()
                        && ui.checkbox(&mut self.config.autostart, lang.autostart()).changed()
                    {
                        crate::desktop::apply_autostart_setting(self.config.autostart);
                        changed = true;
                    }
                }
            }
            actions(ui, |ui| {
                let last = step + 1 == STEPS;
                let text = if last { lang.setup_finish() } else { lang.next() };
                if fx::button(ui, ButtonStyle::Filled, "", text).clicked() {
                    next = true;
                }
                if step > 0 && fx::button(ui, ButtonStyle::Text, "", lang.back()).clicked() {
                    back = true;
                }
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    if fx::button(ui, ButtonStyle::Text, "", lang.setup_skip()).clicked() {
                        skip = true;
                    }
                });
            });
        });
        if changed {
            self.settings_changed();
        }
        let Some(wizard) = &mut self.wizard else { return };
        if back {
            wizard.step -= 1;
        } else if next && wizard.step + 1 < STEPS {
            wizard.step += 1;
        } else if next {
            let start = wizard.start_watching;
            self.wizard = None;
            self.save_quietly();
            if start && !self.running() && !self.config.courses.is_empty() {
                self.toggle_watching();
            }
        } else if skip {
            self.wizard = None;
            self.save_quietly();
        }
    }

    /// The downloads folder (`downloads`) or the study folder, with «Change».
    fn folder_field(&mut self, ui: &mut egui::Ui, downloads: bool) -> bool {
        let lang = self.lang;
        let mut changed = false;
        ui.horizontal(|ui| {
            let value = if downloads { &mut self.config.watch_folder } else { &mut self.config.destination_root };
            let field = egui::TextEdit::singleline(value).desired_width(380.0).margin(Margin::symmetric(12, 8));
            changed |= ui.add(field).changed();
            if fx::button(ui, ButtonStyle::Tonal, icon::FOLDER_OPEN, lang.change()).clicked()
                && let Some(folder) = Self::pick_folder(value)
            {
                *value = folder;
                changed = true;
            }
        });
        // (A study folder typed outside the cloud folder switches the cloud off by itself.)
        let path = expand_user(if downloads { &self.config.watch_folder } else { &self.config.destination_root });
        if downloads && !path.is_dir() {
            let p = fx::pal(ui);
            ui.label(RichText::new(lang.folder_missing()).small().color(p.error));
        }
        changed
    }

    /// «Study folder in OneDrive / Dropbox / Google Drive», for the cloud folders found.
    fn cloud_suggestions(&mut self, ui: &mut egui::Ui) -> bool {
        let lang = self.lang;
        let roots = self.cloud.roots.clone().unwrap_or_default();
        if roots.is_empty() {
            return false;
        }
        let p = fx::pal(ui);
        let mut changed = false;
        ui.add_space(12.0);
        ui.label(RichText::new(lang.setup_in_cloud()).small().color(p.on_surface_variant));
        let name = crate::cloud::study_folder_name(&expand_user(&self.config.destination_root));
        for root in roots {
            let target = root.path.join(&name);
            let chosen = expand_user(&self.config.destination_root) == target;
            let label = lang.provider_label(root.provider, root.account.as_deref(), &target.display().to_string());
            let label = format!("{label}  {}  {}", icon::ARROW_RIGHT, target.display());
            if ui.radio(chosen, label).clicked() && !chosen {
                // Nothing to move yet at the first start: the study folder simply lives there.
                self.config.destination_root = target.display().to_string();
                self.connect(CloudMode::Move, &root);
                changed = true;
            }
        }
        changed
    }
}
