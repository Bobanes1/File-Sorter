//! «Google Drive (without the Google app)» in the cloud settings: connecting, the Drive
//! folder, the sync status, and syncing in the background.

use std::time::{Duration, Instant};

use eframe::egui::{self, Margin, RichText};
use egui_phosphor::regular as icon;

use super::settings::{divider, setting_row};
use super::{App, Note};
use crate::cloud::gdrive::{self, Job, Place, Report, SignIn, Summary, Worker};
use crate::cloud::{CloudMode, Provider};
use crate::fx::{self, ButtonStyle};

/// Between syncs (in the background while saving resources: less often).
const EVERY: Duration = Duration::from_secs(15 * 60);
const EVERY_LEAN: Duration = Duration::from_secs(60 * 60);
/// After sorting, wait this long for more before syncing.
const AFTER_SORT: Duration = Duration::from_secs(30);

#[derive(Default)]
pub(super) struct DriveState {
    worker: Option<Worker>,
    sign_in: Option<SignIn>,
    /// The sign-in page, if rclone named it.
    link: Option<String>,
    /// Fetching rclone; then sign in.
    installing: bool,
    /// When it last synced (time of day) and what it did.
    last: Option<(String, Summary)>,
    error: Option<String>,
    next: Option<Instant>,
    /// Files were sorted: sync soon.
    sorted_at: Option<Instant>,
    /// Signed in and rclone there, and when that was looked at (not at every frame).
    ready: Option<(bool, Instant)>,
    /// The Drive folder being typed (taken once the field is left, not at every letter).
    folder_edit: Option<String>,
}

impl App {
    fn drive_place(&self) -> Option<Place> {
        self.config.state_dir.as_deref().map(Place::new)
    }

    /// Signed in, with rclone there (looked at every few seconds).
    fn drive_ready(&mut self) -> bool {
        if let Some((ready, at)) = self.gdrive.ready
            && at.elapsed() < Duration::from_secs(10)
        {
            return ready;
        }
        let ready = self.drive_place().is_some_and(|p| p.signed_in() && p.rclone().is_some());
        self.gdrive.ready = Some((ready, Instant::now()));
        ready
    }

    fn drive_worker(&mut self) -> Option<&Worker> {
        if self.gdrive.worker.is_none() {
            let place = self.drive_place()?;
            let ctx = self.ctx.clone();
            self.gdrive.worker = Some(Worker::start(place, move || ctx.request_repaint()));
        }
        self.gdrive.worker.as_ref()
    }

    /// The study folder is in the Google Drive app's folder already (which syncs it).
    fn in_drive_app_folder(&self) -> bool {
        self.config.cloud.mode != CloudMode::Off && self.config.cloud.provider == Some(Provider::GoogleDrive)
    }

    fn drive_folder(&self) -> String {
        let folder = self.config.gdrive.folder.trim();
        if folder.is_empty() { crate::cloud::study_folder_name(&self.study_root()) } else { folder.to_string() }
    }

    /// Something was sorted: sync with Drive soon.
    pub(super) fn drive_after_sort(&mut self) {
        if self.config.gdrive.on {
            self.gdrive.sorted_at = Some(Instant::now());
        }
    }

    fn start_drive_sync(&mut self) {
        let local = self.study_root();
        let folder = self.drive_folder();
        let first = self.config.gdrive.synced_pair != gdrive::pair(&local, &folder);
        if let Some(worker) = self.drive_worker() {
            worker.send(Job::Sync { local, folder, first });
        }
        self.gdrive.sorted_at = None;
        self.gdrive.next = Some(Instant::now() + if self.lean { EVERY_LEAN } else { EVERY });
    }

    fn start_drive_sign_in(&mut self) {
        let Some(place) = self.drive_place() else { return };
        self.gdrive.error = None;
        if place.rclone().is_none() {
            self.gdrive.installing = true;
            if let Some(worker) = self.drive_worker() {
                worker.send(Job::Install);
            }
            return;
        }
        match place.sign_in() {
            Ok(sign_in) => {
                self.gdrive.sign_in = Some(sign_in);
                self.gdrive.link = None;
            }
            Err(e) => self.gdrive.error = Some(e),
        }
    }

    /// Signing in, reports from the worker, and syncing when it is time.
    pub(super) fn drive_logic(&mut self) {
        if let Some(sign_in) = &mut self.gdrive.sign_in {
            if let Some(link) = sign_in.link() {
                self.gdrive.link = Some(link);
            }
            if let Some(result) = sign_in.poll() {
                self.gdrive.sign_in = None;
                self.gdrive.ready = None;
                match result {
                    Ok(()) => {
                        self.config.gdrive.on = true;
                        if self.config.gdrive.folder.trim().is_empty() {
                            self.config.gdrive.folder = self.drive_folder();
                        }
                        self.settings_changed();
                        self.start_drive_sync();
                    }
                    Err(e) => self.gdrive.error = Some(e),
                }
            }
            self.ctx.request_repaint_after(Duration::from_millis(300));
        }
        let reports: Vec<Report> = self.gdrive.worker.as_ref().map(|w| w.rx.try_iter().collect()).unwrap_or_default();
        for report in reports {
            match report {
                Report::Installed(Ok(())) => {
                    self.gdrive.installing = false;
                    self.start_drive_sign_in();
                }
                Report::Installed(Err(e)) => {
                    self.gdrive.installing = false;
                    self.gdrive.error = Some(e);
                }
                Report::Synced { pair, result } => match result {
                    Ok(summary) => {
                        if summary.down + summary.up > 0 {
                            self.note(Note::Cloud {
                                title: self.lang.gdrive_synced_note(summary.down, summary.up),
                                details: String::new(),
                                warn: false,
                            });
                        }
                        if summary.conflicts > 0 {
                            let title = self.lang.gdrive_conflict_note(summary.conflicts);
                            self.note(Note::Cloud { title, details: String::new(), warn: true });
                        }
                        let time = chrono::Local::now().format("%H:%M").to_string();
                        self.gdrive.last = Some((time, summary));
                        self.gdrive.error = None;
                        if self.config.gdrive.synced_pair != pair {
                            self.config.gdrive.synced_pair = pair;
                            self.settings_changed();
                        }
                    }
                    Err(e) => {
                        // Told once per reason, not at every try.
                        if self.gdrive.error.as_deref() != Some(e.as_str()) {
                            let title = self.lang.gdrive_failed(&e);
                            self.note(Note::Cloud { title, details: String::new(), warn: true });
                        }
                        self.gdrive.error = Some(e);
                    }
                },
            }
        }
        if !self.config.gdrive.on || self.in_drive_app_folder() || !self.drive_ready() {
            return;
        }
        if self.gdrive.worker.as_ref().is_some_and(Worker::busy) {
            return;
        }
        let now = Instant::now();
        let due = self.gdrive.next.is_none_or(|t| now >= t)
            || self.gdrive.sorted_at.is_some_and(|t| now.duration_since(t) >= AFTER_SORT);
        if due {
            self.start_drive_sync();
        }
        // Wake up for the next one (also while hidden).
        let next = [self.gdrive.next, self.gdrive.sorted_at.map(|t| t + AFTER_SORT)]
            .into_iter()
            .flatten()
            .min()
            .map(|t| t.saturating_duration_since(now));
        if let Some(wait) = next {
            self.ctx.request_repaint_after(wait.max(Duration::from_secs(1)));
        }
    }

    /// The Google Drive part of the cloud settings.
    pub(super) fn drive_settings_ui(&mut self, ui: &mut egui::Ui) {
        let lang = self.lang;
        let p = fx::pal(ui);
        let Some(place) = self.drive_place() else { return };
        let signed_in = self.drive_ready();
        let busy = self.gdrive.worker.as_ref().is_some_and(Worker::busy);
        let mut changed = false;
        setting_row(ui, icon::GOOGLE_DRIVE_LOGO, lang.gdrive_title(), lang.gdrive_hint(), |ui| {
            if signed_in {
                ui.label(
                    RichText::new(format!("{}  {}", icon::CHECK_CIRCLE, lang.gdrive_connected())).color(p.success),
                );
            } else if self.gdrive.installing || self.gdrive.sign_in.is_some() {
                ui.add(egui::Spinner::new());
            } else {
                let blocked = self.in_drive_app_folder();
                if ui
                    .add_enabled_ui(!blocked, |ui| {
                        fx::button(ui, ButtonStyle::Tonal, icon::SIGN_IN, lang.gdrive_connect())
                    })
                    .inner
                    .clicked()
                {
                    self.start_drive_sign_in();
                }
            }
        });
        if self.in_drive_app_folder() && !signed_in {
            ui.label(RichText::new(lang.gdrive_app_folder()).small().color(p.on_surface_variant));
        }
        if self.gdrive.installing {
            ui.label(RichText::new(lang.gdrive_installing()).small().color(p.primary));
        }
        if self.gdrive.sign_in.is_some() {
            ui.horizontal(|ui| {
                ui.label(RichText::new(lang.gdrive_signing_in()).small().color(p.primary));
                if let Some(link) = self.gdrive.link.clone()
                    && fx::button(ui, ButtonStyle::Text, icon::ARROW_SQUARE_OUT, lang.gdrive_open_sign_in()).clicked()
                {
                    crate::desktop::open_url(&link);
                }
                if fx::button(ui, ButtonStyle::Text, "", lang.cancel()).clicked() {
                    self.gdrive.sign_in = None;
                }
            });
        }
        if signed_in {
            divider(ui);
            ui.horizontal(|ui| {
                ui.add_space(40.0);
                ui.label(RichText::new(lang.gdrive_folder()).color(p.on_surface));
                let current = self.drive_folder();
                let edit = self.gdrive.folder_edit.get_or_insert(current.clone());
                let field = egui::TextEdit::singleline(edit).desired_width(220.0).margin(Margin::symmetric(10, 6));
                let response = ui.add_enabled(!busy, field);
                if !response.has_focus() {
                    let typed = self.gdrive.folder_edit.take().unwrap_or_default();
                    let typed = typed.trim().trim_matches('/').to_string();
                    if !typed.is_empty() && typed != current {
                        self.config.gdrive.folder = typed;
                        changed = true;
                    }
                }
            });
            divider(ui);
            setting_row(ui, icon::ARROWS_CLOCKWISE, lang.gdrive_sync_on(), lang.gdrive_sync_on_hint(), |ui| {
                changed |= fx::toggle(ui, &mut self.config.gdrive.on).changed();
            });
            ui.horizontal(|ui| {
                ui.add_space(40.0);
                if busy {
                    ui.add(egui::Spinner::new().size(14.0));
                    ui.label(RichText::new(lang.gdrive_syncing()).small().color(p.primary));
                } else if let Some(e) = &self.gdrive.error {
                    ui.label(RichText::new(lang.gdrive_failed(e)).small().color(p.error));
                } else if let Some((time, s)) = &self.gdrive.last {
                    let text = lang.gdrive_last(time, s.down, s.up, s.conflicts);
                    ui.label(RichText::new(text).small().color(p.on_surface_variant));
                } else {
                    ui.label(RichText::new(lang.gdrive_never()).small().color(p.on_surface_variant));
                }
            });
            ui.horizontal(|ui| {
                ui.add_space(32.0);
                let sync = ui.add_enabled_ui(!busy && !self.in_drive_app_folder(), |ui| {
                    fx::button(ui, ButtonStyle::Tonal, icon::ARROWS_CLOCKWISE, lang.gdrive_sync_now())
                });
                if sync.inner.clicked() {
                    self.gdrive.error = None;
                    self.start_drive_sync();
                }
                if fx::button(ui, ButtonStyle::Text, icon::CLOCK_COUNTER_CLOCKWISE, lang.gdrive_backups()).clicked() {
                    let backups = place.backups();
                    let _ = std::fs::create_dir_all(&backups);
                    crate::desktop::open(&backups);
                }
                if ui
                    .add_enabled_ui(!busy, |ui| {
                        fx::button(ui, ButtonStyle::Text, icon::SIGN_OUT, lang.gdrive_disconnect())
                    })
                    .inner
                    .clicked()
                {
                    place.sign_out();
                    self.gdrive.ready = None;
                    self.config.gdrive.on = false;
                    self.config.gdrive.synced_pair.clear();
                    self.gdrive.last = None;
                    self.gdrive.error = None;
                    changed = true;
                }
            });
        } else if let Some(e) = &self.gdrive.error {
            ui.label(RichText::new(lang.gdrive_failed(e)).small().color(p.error));
        }
        if changed {
            self.settings_changed();
        }
    }
}
