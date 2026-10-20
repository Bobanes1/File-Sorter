//! The AI model Laya in the window: setting it up (in the background, step by step), the
//! row in the settings, and asking once whether to set it up.

use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant};

use eframe::egui::{self, Id, RichText};
use egui_phosphor::regular as icon;

use super::dialogs::{actions, dialog_frame};
use super::settings::setting_row;
use super::{App, Note};
use crate::fx::{self, ButtonStyle};
use crate::laya::{Place, Step};

enum SetupMessage {
    Step(Step),
    Done(Result<(), String>),
}

#[derive(Default)]
pub(super) struct LayaState {
    running: Option<Receiver<SetupMessage>>,
    step: Option<Step>,
    error: Option<String>,
    /// Asking once whether to set it up (after an update, for those who had the setup before).
    offer: bool,
    /// How much room it takes, and when that was read.
    size: Option<(u64, Instant)>,
}

impl App {
    fn laya_place(&self) -> Option<Place> {
        self.config.state_dir.as_deref().map(Place::new)
    }

    pub(super) fn laya_installed(&self) -> bool {
        self.laya_place().is_some_and(|p| p.installed())
    }

    /// Set Laya up in the background.
    pub(super) fn start_laya_setup(&mut self) {
        let Some(place) = self.laya_place() else { return };
        if self.laya.running.is_some() {
            return;
        }
        let (tx, rx) = channel();
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            let progress = {
                let (tx, ctx) = (tx.clone(), ctx.clone());
                move |step: Step| {
                    log::info!("Setting up Laya: step {} of 4 ({step:?})", step.number());
                    let _ = tx.send(SetupMessage::Step(step));
                    ctx.request_repaint();
                }
            };
            let result = crate::laya::install(&place, progress);
            let _ = tx.send(SetupMessage::Done(result));
            ctx.request_repaint();
        });
        self.laya.running = Some(rx);
        self.laya.error = None;
        self.laya.step = None;
    }

    pub(super) fn laya_logic(&mut self) {
        // Asked once (the setup asks those who start fresh).
        if !self.config.laya_offered && self.wizard.is_none() && !self.config.courses.is_empty() {
            self.config.laya_offered = true;
            if !self.laya_installed() {
                self.laya.offer = true;
            }
            self.settings_changed();
        }
        let Some(rx) = &self.laya.running else { return };
        let messages: Vec<SetupMessage> = rx.try_iter().collect();
        for message in messages {
            match message {
                SetupMessage::Step(step) => self.laya.step = Some(step),
                SetupMessage::Done(result) => {
                    self.laya.running = None;
                    self.laya.step = None;
                    self.laya.size = None;
                    match result {
                        Ok(()) => {
                            self.config.features.laya = true;
                            crate::laya::configure(
                                self.config.state_dir.as_deref().unwrap_or(std::path::Path::new(".")),
                                true,
                            );
                            self.settings_changed();
                            self.note(Note::Cloud {
                                title: self.lang.laya_ready().into(),
                                details: String::new(),
                                warn: false,
                            });
                        }
                        Err(e) => {
                            log::info!("Laya could not be set up: {e}");
                            let title = self.lang.laya_failed().to_string();
                            self.note(Note::Cloud { title, details: e.clone(), warn: true });
                            self.laya.error = Some(e);
                        }
                    }
                }
            }
        }
        if self.laya.running.is_some() {
            self.ctx.request_repaint_after(Duration::from_millis(500));
        }
    }

    /// The row in Settings → Features.
    pub(super) fn laya_setting_ui(&mut self, ui: &mut egui::Ui) -> bool {
        let lang = self.lang;
        let p = fx::pal(ui);
        let installed = self.laya_installed();
        let running = self.laya.running.is_some();
        let mut changed = false;
        let mut remove = false;
        let mut start = false;
        setting_row(ui, icon::SPARKLE, lang.laya_title(), lang.laya_hint(), |ui| {
            if installed {
                if fx::toggle(ui, &mut self.config.features.laya).changed() {
                    changed = true;
                }
            } else if running {
                ui.add(egui::Spinner::new());
            } else if fx::button(ui, ButtonStyle::Tonal, icon::DOWNLOAD_SIMPLE, lang.laya_download()).clicked() {
                start = true;
            }
        });
        ui.horizontal(|ui| {
            ui.add_space(40.0);
            if running {
                let step = self.laya.step.unwrap_or(Step::Tool);
                ui.label(RichText::new(lang.laya_step(step)).small().color(p.primary));
            } else if installed {
                let size = match self.laya.size {
                    Some((size, at)) if at.elapsed() < Duration::from_secs(60) => size,
                    _ => {
                        let size = self.laya_place().map_or(0, |p| p.size());
                        self.laya.size = Some((size, Instant::now()));
                        size
                    }
                };
                let gb = size as f64 / 1e9;
                ui.label(RichText::new(lang.laya_installed(gb)).small().color(p.on_surface_variant));
                if fx::button(ui, ButtonStyle::Text, icon::TRASH, lang.laya_remove()).clicked() {
                    remove = true;
                }
            } else if let Some(e) = &self.laya.error {
                ui.add(egui::Label::new(RichText::new(lang.laya_error(e)).small().color(p.error)).wrap());
            }
        });
        if start {
            self.start_laya_setup();
        }
        if remove && let Some(place) = self.laya_place() {
            if let Err(e) = place.remove() {
                log::info!("Laya could not be removed: {e}");
            }
            self.laya.size = None;
            changed = true;
        }
        if changed {
            let dir = self.config.state_dir.clone().unwrap_or_default();
            crate::laya::configure(&dir, self.config.features.laya);
        }
        changed
    }

    /// Asked once after the update: set Laya up?
    pub(super) fn laya_offer_ui(&mut self, ctx: &egui::Context) {
        if !self.laya.offer {
            return;
        }
        let lang = self.lang;
        let mut answer = None;
        let modal = egui::Modal::new(Id::new("laya-offer")).frame(dialog_frame(ctx)).show(ctx, |ui| {
            let p = fx::pal(ui);
            ui.set_width(460.0);
            ui.label(fx::title(lang.laya_offer_title(), 22.0).color(p.on_surface));
            ui.add_space(8.0);
            ui.add(egui::Label::new(RichText::new(lang.laya_hint()).color(p.on_surface_variant)).wrap());
            actions(ui, |ui| {
                if fx::button(ui, ButtonStyle::Filled, icon::DOWNLOAD_SIMPLE, lang.laya_download()).clicked() {
                    answer = Some(true);
                }
                if fx::button(ui, ButtonStyle::Text, "", lang.not_now()).clicked() {
                    answer = Some(false);
                }
            });
        });
        if modal.should_close() && answer.is_none() {
            answer = Some(false);
        }
        if let Some(yes) = answer {
            self.laya.offer = false;
            if yes {
                self.start_laya_setup();
            }
        }
    }
}
