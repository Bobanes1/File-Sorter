//! The settings tab. Every change is saved a moment later.

use eframe::egui::{self, Align, Layout, Margin, RichText, Sense, Stroke, vec2};
use egui_phosphor::regular as icon;

use super::{App, known_types, section_title};
use crate::config::{DEFAULT_EXTENSIONS, FileAction, OnStart, SortRange, ThemeChoice};
use crate::fx::{self, ButtonStyle};
use crate::i18n::Lang;

/// One setting: icon, title and explanation on the left, its control on the right.
pub(super) fn setting_row(
    ui: &mut egui::Ui,
    glyph: &str,
    title: &str,
    hint: &str,
    control: impl FnOnce(&mut egui::Ui),
) {
    let p = fx::pal(ui);
    ui.horizontal(|ui| {
        ui.set_min_height(56.0);
        ui.allocate_ui_with_layout(vec2(32.0, 24.0), Layout::left_to_right(Align::Center), |ui| {
            ui.label(RichText::new(glyph).size(22.0).color(p.on_surface_variant));
        });
        ui.add_space(8.0);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            control(ui);
            ui.add_space(16.0);
            ui.with_layout(Layout::top_down(Align::Min), |ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                ui.add_space(6.0);
                ui.label(RichText::new(title).size(15.0).color(p.on_surface));
                ui.add(egui::Label::new(RichText::new(hint).color(p.on_surface_variant)).wrap());
            });
        });
    });
}

pub(super) fn divider(ui: &mut egui::Ui) {
    let p = fx::pal(ui);
    let rect = ui.available_rect_before_wrap();
    let y = ui.cursor().top() + 2.0;
    ui.painter().hline((rect.left() + 48.0)..=rect.right(), y, Stroke::new(1.0, p.outline_variant));
    ui.add_space(5.0);
}

/// A row that is itself a radio choice (for "Move" / "Copy").
pub(super) fn radio_row(ui: &mut egui::Ui, selected: bool, title: &str, hint: &str, id: &str) -> bool {
    let p = fx::pal(ui);
    let response = ui
        .scope(|ui| {
            ui.horizontal(|ui| {
                ui.set_min_height(56.0);
                ui.add_space(4.0);
                fx::radio(ui, selected, ui.make_persistent_id(id));
                ui.add_space(12.0);
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    ui.add_space(6.0);
                    ui.label(RichText::new(title).size(15.0).color(p.on_surface));
                    ui.add(egui::Label::new(RichText::new(hint).color(p.on_surface_variant)).wrap());
                });
            })
        })
        .response;
    let rect = response.rect.expand2(vec2(4.0, 2.0));
    let click = ui.interact(rect, ui.make_persistent_id((id, "row")), Sense::click());
    fx::state_layer(ui, &click, rect, 12, p.on_surface);
    fx::ripple(ui, &click, rect, p.primary);
    click.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() && !selected
}

impl App {
    pub(super) fn settings_ui(&mut self, ui: &mut egui::Ui) {
        let p = fx::pal(ui);
        let lang = self.lang;
        let mut changed = false;
        // Comfortable reading width, centred.
        let width = ui.available_width().min(760.0);
        let margin = ((ui.available_width() - width) / 2.0).max(0.0);
        ui.horizontal(|ui| {
            ui.add_space(margin);
            ui.vertical(|ui| {
                ui.set_width(width);

                // "Saved ✓" fades out after each automatic save.
                let shown = (1.0 - (self.now() - self.saved_at - 1.2) / 0.6).clamp(0.0, 1.0) as f32;
                ui.horizontal(|ui| {
                    ui.set_min_height(24.0);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if shown > 0.0 {
                            let text = format!("{} {}", icon::CHECK_CIRCLE, lang.saved());
                            ui.label(RichText::new(text).color(p.success.gamma_multiply(shown)));
                        }
                    });
                });

                section_title(ui, lang.general());
                fx::card(ui, |ui| {
                    setting_row(ui, icon::GLOBE, lang.language(), lang.language_hint(), |ui| {
                        let mut choice = self.config.language.as_deref().map(Lang::from_locale);
                        let options = [
                            (None, lang.automatic().to_string()),
                            (Some(Lang::De), "Deutsch".to_string()),
                            (Some(Lang::En), "English".to_string()),
                        ];
                        if fx::segmented(ui, "language", &mut choice, &options) {
                            self.set_language(choice);
                            changed = true;
                        }
                    });
                    divider(ui);
                    setting_row(ui, icon::PALETTE, lang.appearance(), lang.appearance_hint(), |ui| {
                        let options = [
                            (ThemeChoice::System, lang.automatic().to_string()),
                            (ThemeChoice::Light, lang.light().to_string()),
                            (ThemeChoice::Dark, lang.dark().to_string()),
                        ];
                        if fx::segmented(ui, "theme", &mut self.config.theme, &options) {
                            fx::apply_theme(&self.ctx, self.config.theme);
                            changed = true;
                        }
                    });
                    divider(ui);
                    setting_row(ui, icon::SPARKLE, lang.animations(), lang.animations_hint(), |ui| {
                        if fx::toggle(ui, &mut self.config.animations).changed() {
                            fx::set_motion(&self.ctx, self.config.animations);
                            changed = true;
                        }
                    });
                });
                ui.add_space(16.0);

                section_title(ui, lang.sorting());
                fx::card(ui, |ui| {
                    ui.label(RichText::new(lang.file_action()).small().color(p.on_surface_variant));
                    for (action, title, hint, id) in [
                        (FileAction::Move, lang.action_move(), lang.action_move_hint(), "action-move"),
                        (FileAction::Copy, lang.action_copy(), lang.action_copy_hint(), "action-copy"),
                    ] {
                        if radio_row(ui, self.config.action == action, title, hint, id) {
                            self.config.action = action;
                            changed = true;
                        }
                    }
                    divider(ui);
                    setting_row(
                        ui,
                        icon::FILE_MAGNIFYING_GLASS,
                        lang.read_contents(),
                        lang.read_contents_hint(),
                        |ui| {
                            changed |= fx::toggle(ui, &mut self.config.read_file_contents).changed();
                        },
                    );
                    divider(ui);
                    setting_row(
                        ui,
                        icon::BROOM,
                        lang.sort_existing_on_start(),
                        lang.sort_existing_on_start_hint(),
                        |ui| {
                            let options = [
                                (OnStart::Off, lang.none().to_string()),
                                (OnStart::Day, lang.range_short(SortRange::Day).to_string()),
                                (OnStart::Week, lang.range_short(SortRange::Week).to_string()),
                                (OnStart::All, lang.all().to_string()),
                            ];
                            changed |=
                                fx::segmented(ui, "sort-on-start", &mut self.config.sort_existing_on_start, &options);
                        },
                    );
                    divider(ui);
                    changed |= self.flat_setting_ui(ui);
                    divider(ui);
                    setting_row(
                        ui,
                        icon::APP_WINDOW,
                        lang.auto_results_window(),
                        lang.auto_results_window_hint(),
                        |ui| {
                            changed |= fx::toggle(ui, &mut self.config.auto_results_window).changed();
                        },
                    );
                    divider(ui);
                    changed |= self.file_types_ui(ui);
                });
                ui.add_space(16.0);

                section_title(ui, lang.features());
                fx::card(ui, |ui| {
                    changed |= self.features_ui(ui);
                });
                ui.add_space(16.0);

                section_title(ui, lang.startup());
                fx::card(ui, |ui| {
                    if crate::desktop::autostart_supported() {
                        setting_row(ui, icon::POWER, lang.autostart(), lang.autostart_hint(), |ui| {
                            if fx::toggle(ui, &mut self.config.autostart).changed() {
                                crate::desktop::apply_autostart_setting(self.config.autostart);
                                changed = true;
                            }
                        });
                        divider(ui);
                    }
                    setting_row(ui, icon::PLAY_CIRCLE, lang.watch_on_open(), lang.watch_on_open_hint(), |ui| {
                        changed |= fx::toggle(ui, &mut self.config.watch_on_open).changed();
                    });
                    if crate::tray::supported() {
                        divider(ui);
                        setting_row(ui, icon::TRAY, lang.background(), lang.background_hint(), |ui| {
                            changed |= fx::toggle(ui, &mut self.config.background).changed();
                        });
                    }
                    divider(ui);
                    setting_row(ui, icon::LEAF, lang.lean_background(), lang.lean_background_hint(), |ui| {
                        changed |= fx::toggle(ui, &mut self.config.lean_background).changed();
                    });
                });
                ui.add_space(16.0);

                self.cloud_settings_ui(ui);

                section_title(ui, lang.semester());
                fx::card(ui, |ui| self.archive_settings_ui(ui));
                ui.add_space(16.0);

                if crate::desktop::menu_supported() {
                    section_title(ui, lang.right_click());
                    fx::card(ui, |ui| {
                        setting_row(
                            ui,
                            icon::CURSOR_CLICK,
                            lang.context_menu_setting(),
                            lang.context_menu_hint(),
                            |ui| {
                                if fx::toggle(ui, &mut self.config.context_menu).changed() {
                                    self.apply_context_menu();
                                    changed = true;
                                }
                            },
                        );
                        divider(ui);
                        setting_row(ui, icon::APP_WINDOW, lang.results_window(), lang.results_window_hint(), |ui| {
                            changed |= fx::toggle(ui, &mut self.config.results_window).changed();
                        });
                    });
                    ui.add_space(16.0);
                }

                section_title(ui, lang.about());
                fx::card(ui, |ui| {
                    ui.horizontal(|ui| {
                        let (logo, _) = ui.allocate_exact_size(vec2(48.0, 48.0), Sense::hover());
                        fx::avatar(
                            ui.painter(),
                            logo.center(),
                            24.0,
                            icon::GRADUATION_CAP,
                            p.primary_container,
                            p.on_primary_container,
                        );
                        ui.add_space(8.0);
                        ui.vertical(|ui| {
                            ui.label(
                                fx::title(format!("File Sorter {}", env!("CARGO_PKG_VERSION")), 16.0)
                                    .color(p.on_surface),
                            );
                            ui.add(
                                egui::Label::new(RichText::new(lang.about_text()).color(p.on_surface_variant)).wrap(),
                            );
                        });
                    });
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if fx::button(ui, ButtonStyle::Tonal, icon::MAGIC_WAND, lang.setup_again()).clicked() {
                            self.wizard = Some(super::wizard::Wizard::new());
                        }
                        let folder = self.config_path.parent().map(std::path::Path::to_path_buf).unwrap_or_default();
                        if fx::button(ui, ButtonStyle::Tonal, icon::FILE_TEXT, lang.open_log()).clicked() {
                            crate::desktop::open(&folder.join("file-sorter.log"));
                        }
                        if fx::button(ui, ButtonStyle::Text, icon::FOLDER_OPEN, lang.open_settings_folder()).clicked() {
                            crate::desktop::open(&folder);
                        }
                    });
                });
            });
        });

        if changed {
            self.settings_changed();
        }
    }

    /// Chips for the common file types, a field for others, and "Defaults". Returns true if changed.
    fn file_types_ui(&mut self, ui: &mut egui::Ui) -> bool {
        let p = fx::pal(ui);
        let lang = self.lang;
        let mut changed = false;
        setting_row(ui, icon::FILES, lang.file_types(), lang.file_types_hint(), |_| {});
        ui.horizontal(|ui| {
            ui.add_space(40.0);
            // Wrapped lines start at the indent too.
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(8.0, 8.0);
                for ext in known_types() {
                    let on = self.config.extensions.iter().any(|e| e == ext);
                    if fx::chip(ui, on, ext).clicked() {
                        if on {
                            self.config.extensions.retain(|e| e != ext);
                        } else {
                            self.config.extensions.push(ext.to_string());
                        }
                        changed = true;
                    }
                }
            });
        });
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.add_space(40.0);
            ui.label(RichText::new(lang.more_types()).color(p.on_surface_variant));
            let field = egui::TextEdit::singleline(&mut self.more_types)
                .hint_text(".svg, .ods")
                .desired_width(200.0)
                .margin(Margin::symmetric(10, 6));
            if ui.add(field).changed() {
                self.apply_more_types();
                changed = true;
            }
            if fx::button(ui, ButtonStyle::Text, icon::ARROW_COUNTER_CLOCKWISE, lang.defaults()).clicked() {
                self.config.extensions = DEFAULT_EXTENSIONS.iter().map(|e| e.to_string()).collect();
                self.more_types.clear();
                changed = true;
            }
        });
        if self.config.extensions.is_empty() {
            ui.horizontal(|ui| {
                ui.add_space(40.0);
                ui.label(RichText::new(format!("{} {}", icon::WARNING_CIRCLE, lang.no_types())).color(p.warning));
            });
        }
        changed
    }
}
