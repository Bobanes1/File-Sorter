//! A banner when a new version of File Sorter is out: «Update now» / «Later», or, with
//! automatic updates, «Restart» once it was installed.

use std::sync::atomic::Ordering;
use std::sync::mpsc::{Receiver, channel};

use eframe::egui::{self, Align, Layout, Margin, RichText};
use egui_phosphor::regular as icon;

use super::App;
use crate::config::UpdateMode;
use crate::fx::{self, ButtonStyle};
use crate::update::{self, Release};

/// How often to look while the window stays open, and how soon after the start.
const CHECK_EVERY: f64 = 24.0 * 60.0 * 60.0;
const FIRST_CHECK_AFTER: f64 = 15.0;

enum Message {
    Checked(Result<Option<Release>, String>),
    Installed(Release, Result<(), String>),
}

#[derive(Default)]
pub(super) struct UpdateState {
    rx: Option<Receiver<Message>>,
    last_check: Option<f64>,
    available: Option<Release>,
    /// «Later» was clicked for this version.
    dismissed: Option<String>,
    installing: bool,
    /// Installed (automatically): runs at the next start.
    installed: Option<String>,
    /// Restart right after installing (the user clicked «Update now»).
    restart_after: bool,
    error: Option<String>,
}

impl App {
    pub(super) fn update_logic(&mut self) {
        let now = self.now();
        if let Some(rx) = &self.update.rx
            && let Ok(message) = rx.try_recv()
        {
            self.update.rx = None;
            match message {
                Message::Checked(Ok(Some(release))) => {
                    log::info!("File Sorter {} is available", release.version);
                    let auto = self.config.features.update_install == UpdateMode::Auto;
                    if auto && cfg!(windows) && release.download.is_some() {
                        self.start_install(release, false);
                    } else {
                        self.update.available = Some(release);
                    }
                }
                Message::Checked(Ok(None)) => {}
                Message::Checked(Err(e)) => log::info!("Could not look for updates: {e}"),
                Message::Installed(release, result) => {
                    self.update.installing = false;
                    match result {
                        Ok(()) if self.update.restart_after => {
                            log::info!("Updated to {}; restarting", release.version);
                            match update::restart() {
                                Ok(()) => {
                                    self.quit.store(true, Ordering::SeqCst);
                                    self.ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                                }
                                Err(e) => {
                                    log::info!("Could not restart: {e}");
                                    self.update.installed = Some(release.version);
                                }
                            }
                        }
                        Ok(()) => {
                            log::info!("Updated to {}; it runs from the next start", release.version);
                            self.update.installed = Some(release.version);
                            self.update.available = None;
                        }
                        Err(e) => {
                            log::info!("Could not install {}: {e}", release.version);
                            self.update.error = Some(e);
                            self.update.available = Some(release);
                        }
                    }
                }
            }
        }
        let due = match self.update.last_check {
            None => now > FIRST_CHECK_AFTER,
            Some(t) => now - t > CHECK_EVERY,
        };
        if self.config.features.update_check && due && self.update.rx.is_none() && !self.update.installing {
            self.update.last_check = Some(now);
            let (tx, rx) = channel();
            let ctx = self.ctx.clone();
            std::thread::spawn(move || {
                let _ = tx.send(Message::Checked(update::check()));
                ctx.request_repaint();
            });
            self.update.rx = Some(rx);
        }
        if self.config.features.update_check && self.update.last_check.is_none() {
            self.ctx.request_repaint_after(std::time::Duration::from_secs_f64(FIRST_CHECK_AFTER));
        }
    }

    fn start_install(&mut self, release: Release, restart: bool) {
        let Some(dir) = self.config.state_dir.clone() else { return };
        self.update.installing = true;
        self.update.restart_after = restart;
        self.update.error = None;
        let (tx, rx) = channel();
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            let result = update::install(&release, &dir);
            let _ = tx.send(Message::Installed(release, result));
            ctx.request_repaint();
        });
        self.update.rx = Some(rx);
    }

    /// The banner under the top bar, while there is something to say.
    pub(super) fn update_banner(&mut self, ui: &mut egui::Ui) {
        let lang = self.lang;
        let installed = self.update.installed.clone();
        let available =
            self.update.available.clone().filter(|r| self.update.dismissed.as_deref() != Some(r.version.as_str()));
        if !self.config.features.update_check || (installed.is_none() && available.is_none() && !self.update.installing)
        {
            return;
        }
        let p = fx::pal(ui);
        egui::Panel::top("update-banner")
            .frame(egui::Frame::new().fill(p.surface).inner_margin(Margin { left: 20, right: 20, top: 4, bottom: 4 }))
            .show_separator_line(false)
            .show(ui, |ui| {
                egui::Frame::new()
                    .fill(p.primary_container)
                    .corner_radius(egui::CornerRadius::same(12))
                    .inner_margin(Margin::symmetric(16, 6))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(icon::ARROW_CIRCLE_UP).size(20.0).color(p.on_primary_container));
                            let text = if let Some(version) = &installed {
                                lang.update_installed(version)
                            } else if self.update.installing {
                                lang.update_installing()
                            } else if let Some(error) = &self.update.error {
                                lang.update_failed(error)
                            } else {
                                lang.update_available(
                                    &available.as_ref().map(|r| r.version.clone()).unwrap_or_default(),
                                )
                            };
                            ui.add(egui::Label::new(RichText::new(text).color(p.on_primary_container)).wrap());
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                if installed.is_some() {
                                    if fx::button(ui, ButtonStyle::Filled, "", lang.restart_now()).clicked()
                                        && update::restart().is_ok()
                                    {
                                        self.quit.store(true, Ordering::SeqCst);
                                        self.ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                                    }
                                } else if self.update.installing {
                                    ui.add(egui::Spinner::new());
                                } else if let Some(release) = available {
                                    if fx::button(ui, ButtonStyle::Text, "", lang.later()).clicked() {
                                        self.update.dismissed = Some(release.version.clone());
                                    }
                                    if fx::button(ui, ButtonStyle::Text, "", lang.whats_new()).clicked() {
                                        crate::desktop::open(std::path::Path::new(&release.page));
                                    }
                                    if cfg!(windows)
                                        && release.download.is_some()
                                        && fx::button(ui, ButtonStyle::Filled, "", lang.update_now()).clicked()
                                    {
                                        self.start_install(release, true);
                                    }
                                }
                            });
                        });
                    });
            });
    }
}
