//! The cloud card in the settings and what it drives: moving the study folder into
//! OneDrive, Dropbox, Google Drive… (or back), the copy in the cloud, and modules shared
//! with the user's other computers.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, SystemTime};

use eframe::egui::{self, Id, RichText};
use egui_phosphor::regular as icon;

use super::cards::{CardKind, FileState, Note, Snackbar};
use super::dialogs::{actions, dialog_frame};
use super::settings::{divider, radio_row, setting_row};
use super::{App, section_title};
use crate::cloud::detect::{self, CloudRoot, Provider};
use crate::cloud::mirror::MirrorReport;
use crate::cloud::relocate::{self, Plan, Progress};
use crate::cloud::shared::{self, Remote, SHARED_FILE, Shared, Step};
use crate::cloud::worker::{CloudEvent, Job, Worker};
use crate::cloud::{CloudMode, CloudSettings, cloud_study_root, computer_name, is_inside, study_folder_name};
use crate::config::expand_user;
use crate::fx::{self, ButtonStyle};
use crate::sorter::{Outcome, SortResult};

/// How often the shared modules are checked for changes made on other computers.
const SHARED_CHECK_SECS: f64 = 5.0;
/// Module edits here are shared once they have rested this long (while typing keywords).
const SHARED_SETTLE_MS: i64 = 2_000;
/// How long closing the window waits for the file being moved right now.
const FINISH_ON_EXIT: Duration = Duration::from_secs(15);

pub(super) struct CloudState {
    worker: Worker,
    events: Receiver<CloudEvent>,
    /// Cloud folders on this computer (`None` while still looking).
    roots: Option<Vec<CloudRoot>>,
    detecting: Option<Receiver<(Vec<CloudRoot>, Option<Plan>)>>,
    /// The choice in the card, applied with its button.
    choice: CloudMode,
    /// The cloud folder picked in the card.
    pick: Option<CloudRoot>,
    moving: Option<Moving>,
    /// Files still at the study folder's old place after moving it (open at the time, or stopped).
    leftover: Option<(PathBuf, Plan)>,
    /// The copy is being brought up to date right now.
    syncing: bool,
    /// The last full pass over the copy: when (window time) and what it did.
    last_pass: Option<(f64, MirrorReport)>,
    /// The copy the background thread keeps up to date (to tell it when that changes).
    mirroring: Option<(PathBuf, PathBuf)>,
    computer: String,
    shared_checked: f64,
    shared_modified: Option<SystemTime>,
    /// What the shared file held when this computer last read or wrote it.
    shared_snapshot: Option<Shared>,
    /// When the modules here last changed without being shared yet (ms since 1970).
    local_changed_at: Option<i64>,
    /// The modules as they were at `local_changed_at`, to notice the next change.
    local_edit: Option<Shared>,
    /// `modified` of a shared file found damaged: replaced only if it stays so until the next check.
    shared_damaged: Option<SystemTime>,
    /// No new try before this (window time): after a failed write or an unreadable file.
    shared_wait_until: f64,
    /// Writing the shared file failed last time (logged once, not on every try).
    shared_failing: bool,
    dialog: Option<CloudDialog>,
}

struct Moving {
    progress: Progress,
    /// Back to this computer.
    back: bool,
    /// Study folder and cloud settings before, restored if the move fails before anything moved.
    before: (String, CloudSettings),
}

enum CloudDialog {
    Move { root: CloudRoot, from: PathBuf, to: PathBuf, plan: Plan, existing: Plan },
    Mirror { root: CloudRoot, to: PathBuf, plan: Plan },
    Back { from: PathBuf, to: PathBuf, then: CloudMode },
    Shared(Shared),
    Message(String),
}

impl CloudState {
    pub(super) fn new(ctx: &egui::Context, state_dir: Option<PathBuf>, settings: &CloudSettings) -> CloudState {
        let (tx, events) = channel();
        let repaint = ctx.clone();
        let worker = Worker::start(
            state_dir,
            Arc::new(move |event| {
                let _ = tx.send(event);
                repaint.request_repaint();
            }),
        );
        // Looking at the drives can take a moment (a network drive that doesn't answer).
        let (found_tx, detecting) = channel();
        let repaint = ctx.clone();
        let old_place = (settings.mode == CloudMode::Move && !settings.local_root.trim().is_empty())
            .then(|| expand_user(settings.local_root.trim()));
        std::thread::spawn(move || {
            let roots = detect::detect();
            let leftover = old_place.map(|p| relocate::plan(&p)).filter(|plan| plan.files > 0);
            let _ = found_tx.send((roots, leftover));
            repaint.request_repaint();
        });
        CloudState {
            worker,
            events,
            roots: None,
            detecting: Some(detecting),
            choice: settings.mode,
            pick: None,
            moving: None,
            leftover: None,
            syncing: false,
            last_pass: None,
            mirroring: None,
            computer: computer_name(),
            shared_checked: f64::NEG_INFINITY,
            shared_modified: None,
            shared_snapshot: None,
            local_changed_at: None,
            local_edit: None,
            shared_damaged: None,
            shared_wait_until: f64::NEG_INFINITY,
            shared_failing: false,
            dialog: None,
        }
    }

    pub(super) fn moving(&self) -> bool {
        self.moving.is_some()
    }

    /// Closing: stop a running move or copy between two files, and wait for the current one.
    pub(super) fn finish(&self) {
        self.worker.finish(FINISH_ON_EXIT);
    }
}

fn glyph(provider: Provider) -> &'static str {
    match provider {
        Provider::Dropbox => icon::DROPBOX_LOGO,
        Provider::GoogleDrive => icon::GOOGLE_DRIVE_LOGO,
        Provider::ICloud => icon::APPLE_LOGO,
        Provider::Other => icon::FOLDER_SIMPLE,
        Provider::OneDrive | Provider::OneDriveBusiness => icon::CLOUD,
    }
}

fn same_folder(a: &Path, b: &Path) -> bool {
    is_inside(a, b) && is_inside(b, a)
}

impl App {
    fn study_name(&self) -> String {
        study_folder_name(&self.study_root())
    }

    /// The service the study folder is connected to, by name ("OneDrive", or the folder's name).
    fn cloud_name(&self) -> String {
        let folder = expand_user(self.config.cloud.folder.trim());
        match self.config.cloud.provider {
            Some(provider) if provider != Provider::Other => self.lang.provider(provider).to_string(),
            _ => {
                folder.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| self.lang.cloud().into())
            }
        }
    }

    fn pick_name(&self, root: &CloudRoot) -> String {
        match root.provider {
            Provider::Other => root.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
            provider => self.lang.provider(provider).to_string(),
        }
    }

    /// The small label next to the study folder on the overview ("OneDrive", "Copy in Dropbox").
    pub(super) fn cloud_chip(&self) -> Option<String> {
        match self.config.cloud.mode {
            CloudMode::Off => None,
            CloudMode::Move => Some(self.lang.cloud_chip(&self.cloud_name(), false)),
            CloudMode::Mirror => Some(self.lang.cloud_chip(&self.cloud_name(), true)),
        }
    }

    // ---------- every frame ----------

    pub(super) fn cloud_logic(&mut self) {
        if let Some(rx) = &self.cloud.detecting {
            let (roots, leftover) = match rx.try_recv() {
                Ok(found) => found,
                Err(std::sync::mpsc::TryRecvError::Empty) => return self.cloud_logic_rest(),
                Err(std::sync::mpsc::TryRecvError::Disconnected) => (Vec::new(), None), // looking failed
            };
            self.cloud.detecting = None;
            self.cloud.leftover = leftover.map(|plan| (expand_user(self.config.cloud.local_root.trim()), plan));
            self.cloud.pick = self.current_root(&roots).or_else(|| roots.first().cloned());
            self.cloud.roots = Some(roots);
        }
        self.cloud_logic_rest();
    }

    fn cloud_logic_rest(&mut self) {
        while let Ok(event) = self.cloud.events.try_recv() {
            self.cloud_event(event);
        }
        // The study folder was changed by hand to a place outside the cloud folder.
        if self.config.cloud.mode == CloudMode::Move
            && self.cloud.moving.is_none()
            && !is_inside(&self.study_root(), &expand_user(self.config.cloud.folder.trim()))
        {
            self.config.cloud.mode = CloudMode::Off;
            self.cloud.choice = CloudMode::Off;
        }
        let wanted = match (self.config.cloud.mode, &self.cloud.moving) {
            (CloudMode::Mirror, None) => cloud_study_root(&self.config).map(|copy| (self.study_root(), copy)),
            _ => None,
        };
        if wanted != self.cloud.mirroring {
            self.cloud.worker.send(Job::SetMirror(wanted.clone()));
            self.cloud.mirroring = wanted;
        }
        self.shared_logic();
        if self.config.cloud.share_settings && self.config.cloud.mode != CloudMode::Off {
            self.ctx.request_repaint_after(Duration::from_secs_f64(SHARED_CHECK_SECS));
        }
    }

    /// The configured cloud folder, as one of the detected ones if it is.
    fn current_root(&self, roots: &[CloudRoot]) -> Option<CloudRoot> {
        let folder = self.config.cloud.folder.trim();
        if folder.is_empty() {
            return None;
        }
        let path = expand_user(folder);
        roots.iter().find(|r| same_folder(&r.path, &path)).cloned().or_else(|| {
            Some(CloudRoot { provider: self.config.cloud.provider.unwrap_or(Provider::Other), path, account: None })
        })
    }

    fn cloud_event(&mut self, event: CloudEvent) {
        let lang = self.lang;
        match event {
            CloudEvent::Progress(progress) => {
                if let Some(moving) = &mut self.cloud.moving {
                    moving.progress = progress;
                }
            }
            CloudEvent::Relocated { from, to, result } => {
                let moving = self.cloud.moving.take();
                let back = moving.as_ref().is_some_and(|m| m.back);
                match result {
                    Ok(report) => {
                        self.relocate_cards(&from, &to, &report.renamed);
                        // `moved` includes the renamed ones.
                        let files = report.moved + report.duplicates;
                        let title =
                            if back { lang.relocated_back(files) } else { lang.relocated(&self.cloud_name(), files) };
                        let details = lang.relocate_details(report.duplicates, report.renamed.len(), report.left.len());
                        let warn = !report.left.is_empty() || report.cancelled;
                        self.note(Note::Cloud { title, details, warn });
                        if report.cancelled {
                            self.note(Note::Cloud {
                                title: lang.relocate_cancelled(files),
                                details: String::new(),
                                warn,
                            });
                        }
                        let rest = relocate::plan(&from);
                        // Files still at the old place (open, failed, stopped) can be moved later.
                        self.cloud.leftover = (rest.files > 0).then_some((from, rest));
                    }
                    Err(error) => {
                        if let Some(Moving { before: (root, settings), .. }) = moving {
                            self.config.destination_root = root;
                            self.cloud.choice = settings.mode;
                            self.config.cloud = settings;
                            self.save_quietly();
                        }
                        self.note(Note::Cloud {
                            title: lang.relocate_failed(&error),
                            details: String::new(),
                            warn: true,
                        });
                    }
                }
            }
            CloudEvent::MirrorStarted => self.cloud.syncing = true,
            CloudEvent::Mirrored { report, full } => {
                if !report.conflicts.is_empty() {
                    let names: Vec<String> = report.conflicts.iter().map(|p| crate::sorter::file_name(p)).collect();
                    let title = lang.mirror_conflicts(names.len());
                    self.note(Note::Cloud { title, details: names.join(" · "), warn: true });
                }
                if full {
                    self.cloud.syncing = false;
                    self.cloud.last_pass = Some((self.now(), report));
                } else if let Some((_, last)) = &mut self.cloud.last_pass {
                    last.copied += report.copied;
                    last.updated += report.updated;
                }
            }
        }
    }

    /// Entries in the activity list follow their files to the study folder's new place.
    fn relocate_cards(&mut self, from: &Path, to: &Path, renamed: &[(PathBuf, PathBuf)]) {
        self.deadlines_follow(|path| crate::history::relocated(path, from, to, renamed));
        for card in &mut self.cards {
            if let CardKind::File { state: FileState::Done(result) | FileState::UndoFailed { result, .. }, .. } =
                &mut card.kind
            {
                for path in result.study_paths_mut() {
                    if let Some(new) = crate::history::relocated(path, from, to, renamed) {
                        *path = new;
                    }
                }
            }
        }
    }

    /// A file was just sorted: its copy in the cloud follows right away.
    pub(super) fn cloud_after_sort(&self, result: &SortResult) {
        if self.config.cloud.mode == CloudMode::Mirror
            && let Outcome::Moved { destination, .. } | Outcome::Copied { destination, .. } = &result.outcome
        {
            self.cloud.worker.send(Job::MirrorFile(destination.clone()));
            // An unpacked ZIP file or an older version moved aside: a whole pass is simplest.
            if !result.extras.is_empty() {
                self.cloud.worker.send(Job::MirrorNow);
            }
        }
    }

    /// Sorting was undone: the file's copy in the cloud goes too (unless it was changed there).
    pub(super) fn cloud_after_undo(&self, result: &SortResult) {
        if self.config.cloud.mode == CloudMode::Mirror
            && let Outcome::Moved { destination, .. } | Outcome::Copied { destination, .. } = &result.outcome
        {
            self.cloud.worker.send(Job::Forget(destination.clone()));
            if !result.extras.is_empty() {
                self.cloud.worker.send(Job::MirrorNow);
            }
        }
    }

    // ---------- modules shared between computers ----------

    fn shared_logic(&mut self) {
        let settings = &self.config.cloud;
        if !settings.share_settings
            || settings.mode == CloudMode::Off
            || self.cloud.moving.is_some()
            || matches!(self.cloud.dialog, Some(CloudDialog::Shared(_)))
            // The module dialogs hold a module's place in the list, which taking over changes.
            || self.dialog.is_some()
            || self.confirm_delete.is_some()
        {
            return;
        }
        let Some(root) = cloud_study_root(&self.config) else { return };
        if !root.parent().is_some_and(Path::is_dir) {
            return; // cloud folder not there right now
        }
        let now = self.now();
        let now_ms = chrono::Local::now().timestamp_millis();
        if self.cloud.shared_snapshot.as_ref().is_some_and(|s| !s.matches(&self.config)) {
            let current = Shared::from_config(&self.config, 0, "");
            if self.cloud.local_edit.as_ref() != Some(&current) {
                self.cloud.local_changed_at = Some(now_ms);
                self.cloud.local_edit = Some(current);
            }
        }
        if now < self.cloud.shared_wait_until {
            return;
        }
        // Edits are shared once they have rested, and nothing is taken over while the user is
        // still typing (a clock set back counts as rested).
        let local_due = match self.cloud.local_changed_at {
            Some(t) if (0..SHARED_SETTLE_MS).contains(&(now_ms - t)) => {
                self.ctx.request_repaint_after(Duration::from_millis(SHARED_SETTLE_MS.unsigned_abs()));
                return;
            }
            Some(_) => true,
            None => false,
        };
        if !local_due && now - self.cloud.shared_checked < SHARED_CHECK_SECS {
            return;
        }
        self.cloud.shared_checked = now;
        let file = root.join(SHARED_FILE);
        let modified = shared::modified(&file);
        let first = self.cloud.shared_snapshot.is_none();
        if !first && !local_due && modified == self.cloud.shared_modified {
            return; // nothing new on either side
        }
        let remote = match shared::load(&file) {
            Remote::Ok(remote) => Some(remote),
            Remote::Missing => None,
            Remote::NewerFormat => {
                self.cloud.shared_modified = modified; // left alone until it changes
                return;
            }
            // May hold another computer's change: tried again later, never overwritten.
            Remote::Unreadable => {
                self.cloud.shared_wait_until = now + SHARED_CHECK_SECS;
                return;
            }
            // Maybe still being replaced by the cloud app: only a file that stays damaged
            // until the next check is replaced (`shared::write` keeps a copy of it).
            Remote::Damaged if modified.is_none() || self.cloud.shared_damaged != modified => {
                self.cloud.shared_damaged = modified;
                self.cloud.shared_wait_until = now + SHARED_CHECK_SECS;
                return;
            }
            Remote::Damaged => None,
        };
        // Only now: a file that couldn't be read is looked at again at the next check.
        self.cloud.shared_modified = modified;
        let seen = self.config.cloud.shared_seen;
        let base = self.cloud.shared_snapshot.as_ref();
        match shared::decide(&self.config, seen, base, self.cloud.local_changed_at, remote.as_ref()) {
            Step::Nothing => {
                if let Some(remote) = remote {
                    self.config.cloud.shared_seen = remote.updated_at;
                    self.cloud.shared_snapshot = Some(remote);
                }
                self.cloud.local_changed_at = None;
                self.cloud.local_edit = None;
            }
            // Two different sets of modules meet for the first time: ask which to keep.
            Step::Apply(remote) if first && seen == 0 && !self.config.courses.is_empty() => {
                self.cloud.dialog = Some(CloudDialog::Shared(remote));
            }
            Step::Apply(remote) => self.take_shared(remote),
            Step::Merge(merged) => {
                self.take_shared(merged);
                self.write_shared(&file, now_ms);
            }
            Step::Write => self.write_shared(&file, now_ms),
        }
    }

    fn take_shared(&mut self, remote: Shared) {
        let changed = remote.apply_to(&mut self.config);
        self.sort_courses();
        self.config.cloud.shared_seen = remote.updated_at;
        self.cloud.local_changed_at = None;
        self.cloud.local_edit = None;
        if changed && remote.updated_by != self.cloud.computer {
            let text = self.lang.snack_shared_from(&remote.updated_by);
            self.snackbar = Some(Snackbar { card: 0, text, can_undo: false, shown: self.now() });
        }
        self.cloud.shared_snapshot = Some(remote);
    }

    /// Write ours over the shared file last seen (`shared_seen`).
    fn write_shared(&mut self, file: &Path, now_ms: i64) {
        let seen = self.config.cloud.shared_seen;
        // `updated_at` also tells the files apart: never the same as the one replaced.
        let now_ms = if now_ms == seen { now_ms + 1 } else { now_ms };
        let ours = Shared { based_on: seen, ..Shared::from_config(&self.config, now_ms, &self.cloud.computer) };
        match shared::write(file, &ours) {
            Ok(()) => {
                if self.cloud.shared_failing {
                    log::info!("Sharing the modules works again");
                }
                self.cloud.shared_failing = false;
                self.config.cloud.shared_seen = now_ms;
                self.cloud.shared_modified = shared::modified(file);
                self.cloud.shared_snapshot = Some(ours);
                self.cloud.local_changed_at = None;
                self.cloud.local_edit = None;
            }
            Err(e) => {
                if !self.cloud.shared_failing {
                    log::info!(
                        "Could not share the modules in {} (tried again every few seconds): {e}",
                        file.display()
                    );
                }
                self.cloud.shared_failing = true;
                // Not on every frame: each try puts a temporary file into the synced folder.
                self.cloud.shared_wait_until = self.now() + SHARED_CHECK_SECS;
                // Still to be shared, also when it was a merge just taken over.
                self.cloud.local_changed_at.get_or_insert(now_ms);
            }
        }
    }

    // ---------- applying the choice ----------

    fn apply_cloud_choice(&mut self) {
        let lang = self.lang;
        let current = self.config.cloud.mode;
        let choice = self.cloud.choice;
        let study = self.study_root();
        match (choice, self.cloud.pick.clone()) {
            (CloudMode::Off, _) => match current {
                CloudMode::Off => {}
                CloudMode::Mirror => {
                    let text = lang.snack_mirror_off(&self.cloud_name());
                    self.config.cloud.mode = CloudMode::Off;
                    self.snackbar = Some(Snackbar { card: 0, text, can_undo: false, shown: self.now() });
                    self.settings_changed();
                }
                CloudMode::Move => {
                    let to = self.home_place();
                    self.cloud.dialog = Some(CloudDialog::Back { from: study, to, then: CloudMode::Off });
                }
            },
            (_, None) => {}
            (CloudMode::Move, Some(root)) => {
                let to = root.path.join(self.study_name());
                if is_inside(&study, &root.path) {
                    // Already in that cloud folder: nothing to move.
                    let text = lang.cloud_already_there(&self.pick_name(&root));
                    self.connect(CloudMode::Move, &root);
                    self.cloud.dialog = Some(CloudDialog::Message(text));
                } else if is_inside(&root.path, &study) {
                    self.cloud.dialog = Some(CloudDialog::Message(lang.cloud_same_folder().into()));
                } else {
                    let plan = relocate::plan(&study);
                    if plan.files == 0 {
                        // Nothing here yet (a second computer): just use the folder in the cloud.
                        let _ = std::fs::create_dir_all(&to);
                        if current != CloudMode::Move {
                            self.config.cloud.local_root = study.display().to_string();
                        }
                        self.config.destination_root = to.display().to_string();
                        self.connect(CloudMode::Move, &root);
                    } else {
                        let existing = relocate::plan(&to);
                        self.cloud.dialog = Some(CloudDialog::Move { root, from: study, to, plan, existing });
                    }
                }
            }
            (CloudMode::Mirror, Some(root)) => {
                if current == CloudMode::Move {
                    // First back to this computer, then the copy.
                    let to = self.home_place();
                    self.cloud.dialog = Some(CloudDialog::Back { from: study, to, then: CloudMode::Mirror });
                    return;
                }
                let to = root.path.join(self.study_name());
                if is_inside(&to, &study) || is_inside(&study, &to) {
                    self.cloud.dialog = Some(CloudDialog::Message(lang.cloud_same_folder().into()));
                } else {
                    let plan = relocate::plan(&study);
                    self.cloud.dialog = Some(CloudDialog::Mirror { root, to, plan });
                }
            }
        }
    }

    /// Where the study folder goes when it comes back from the cloud: its old place, or Documents.
    fn home_place(&self) -> PathBuf {
        let cloud = expand_user(self.config.cloud.folder.trim());
        let old = expand_user(self.config.cloud.local_root.trim());
        if !self.config.cloud.local_root.trim().is_empty() && !is_inside(&old, &cloud) {
            return old;
        }
        let base = dirs::document_dir().or_else(dirs::home_dir).unwrap_or_else(|| PathBuf::from("."));
        base.join(self.study_name())
    }

    /// Use `root` as the cloud folder in `mode` (without moving anything).
    fn connect(&mut self, mode: CloudMode, root: &CloudRoot) {
        self.config.cloud.mode = mode;
        self.config.cloud.folder = root.path.display().to_string();
        self.config.cloud.provider = Some(root.provider);
        self.cloud.choice = mode;
        self.cloud.shared_snapshot = None;
        self.cloud.shared_checked = f64::NEG_INFINITY;
        self.save_quietly();
    }

    fn start_move(&mut self, root: &CloudRoot, from: PathBuf, to: PathBuf) {
        let before = (self.config.destination_root.clone(), self.config.cloud.clone());
        if self.config.cloud.mode != CloudMode::Move {
            self.config.cloud.local_root = from.display().to_string();
        }
        self.config.destination_root = to.display().to_string();
        self.connect(CloudMode::Move, root);
        self.cloud.leftover = None;
        self.cloud.moving = Some(Moving { progress: Progress::default(), back: false, before });
        self.cloud.worker.send(Job::Relocate { from, to });
    }

    fn start_back(&mut self, from: PathBuf, to: PathBuf, then: CloudMode) {
        let before = (self.config.destination_root.clone(), self.config.cloud.clone());
        self.config.destination_root = to.display().to_string();
        match (then, self.cloud.pick.clone()) {
            (CloudMode::Mirror, Some(root)) => self.connect(CloudMode::Mirror, &root),
            _ => {
                self.config.cloud.mode = CloudMode::Off;
                self.cloud.choice = CloudMode::Off;
                self.save_quietly();
            }
        }
        self.cloud.leftover = None;
        self.cloud.moving = Some(Moving { progress: Progress::default(), back: true, before });
        self.cloud.worker.send(Job::Relocate { from, to });
    }

    /// Move what is still at the study folder's old place into the study folder.
    fn move_rest(&mut self, from: PathBuf) {
        let to = self.study_root();
        if same_folder(&from, &to) {
            self.cloud.leftover = None;
            return;
        }
        let before = (self.config.destination_root.clone(), self.config.cloud.clone());
        let back = self.config.cloud.mode != CloudMode::Move;
        self.cloud.leftover = None;
        self.cloud.moving = Some(Moving { progress: Progress::default(), back, before });
        self.cloud.worker.send(Job::Relocate { from, to });
    }

    // ---------- the card ----------

    pub(super) fn cloud_settings_ui(&mut self, ui: &mut egui::Ui) {
        let lang = self.lang;
        section_title(ui, lang.cloud());
        fx::card(ui, |ui| {
            setting_row(ui, icon::CLOUD, lang.cloud_title(), lang.cloud_hint(), |_| {});
            let moving = self.cloud.moving.is_some();
            ui.add_enabled_ui(!moving, |ui| {
                for (mode, title, hint, id) in [
                    (CloudMode::Off, lang.cloud_off(), lang.cloud_off_hint(), "cloud-off"),
                    (CloudMode::Move, lang.cloud_move(), lang.cloud_move_hint(), "cloud-move"),
                    (CloudMode::Mirror, lang.cloud_mirror(), lang.cloud_mirror_hint(), "cloud-mirror"),
                ] {
                    if radio_row(ui, self.cloud.choice == mode, title, hint, id) {
                        self.cloud.choice = mode;
                    }
                }
                if self.cloud.choice != CloudMode::Off {
                    divider(ui);
                    self.cloud_folder_choice(ui);
                }
            });
            self.cloud_action_ui(ui);
            divider(ui);
            let available = self.config.cloud.mode != CloudMode::Off;
            let hint = if available { lang.share_hint() } else { lang.share_needs_cloud() };
            setting_row(ui, icon::SHARE_NETWORK, lang.share_settings(), hint, |ui| {
                ui.add_enabled_ui(available, |ui| {
                    if fx::toggle(ui, &mut self.config.cloud.share_settings).changed() {
                        self.cloud.shared_snapshot = None;
                        self.cloud.shared_checked = f64::NEG_INFINITY;
                        self.settings_changed();
                    }
                });
            });
        });
        ui.add_space(16.0);
    }

    fn cloud_folder_choice(&mut self, ui: &mut egui::Ui) {
        let p = fx::pal(ui);
        let lang = self.lang;
        ui.label(RichText::new(lang.cloud_folder()).small().color(p.on_surface_variant));
        ui.add_space(4.0);
        let Some(roots) = self.cloud.roots.clone() else {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(RichText::new(lang.cloud_searching()).color(p.on_surface_variant));
            });
            return;
        };
        if roots.is_empty() {
            ui.add(egui::Label::new(RichText::new(lang.cloud_none_found()).color(p.on_surface_variant)).wrap());
            ui.add_space(4.0);
        }
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(8.0, 8.0);
            let mut shown = roots.clone();
            // A folder chosen by hand stays in the list.
            if let Some(pick) = &self.cloud.pick
                && !shown.iter().any(|r| same_folder(&r.path, &pick.path))
            {
                shown.push(pick.clone());
            }
            for root in shown {
                let on = self.cloud.pick.as_ref().is_some_and(|p| same_folder(&p.path, &root.path));
                let name =
                    lang.provider_label(root.provider, root.account.as_deref(), &root.path.display().to_string());
                let chip = fx::chip(ui, on, &format!("{}  {name}", glyph(root.provider)));
                if chip.on_hover_text(root.path.display().to_string()).clicked() {
                    self.cloud.pick = Some(root);
                }
            }
            if fx::chip(ui, false, &format!("{}  {}", icon::FOLDER_OPEN, lang.other_folder())).clicked()
                && let Some(folder) = App::pick_folder(&self.config.cloud.folder)
            {
                let path = PathBuf::from(folder);
                let provider = detect::provider_of(&path, &roots).map(|r| r.provider).unwrap_or(Provider::Other);
                self.cloud.pick = Some(CloudRoot { provider, path, account: None });
            }
        });
        let configured = expand_user(self.config.cloud.folder.trim());
        let settled = self.cloud.choice == self.config.cloud.mode
            && self.cloud.pick.as_ref().is_some_and(|p| same_folder(&p.path, &configured));
        if let Some(pick) = self.cloud.pick.as_ref().filter(|_| !settled) {
            let target = pick.path.join(self.study_name());
            ui.add_space(4.0);
            ui.label(
                RichText::new(format!("{}  {}", icon::ARROW_RIGHT, lang.cloud_target(&target.display().to_string())))
                    .small()
                    .color(p.on_surface_variant),
            );
        }
    }

    /// The button that applies the choice, the move's progress, or how the copy is doing.
    fn cloud_action_ui(&mut self, ui: &mut egui::Ui) {
        let p = fx::pal(ui);
        let lang = self.lang;
        ui.add_space(8.0);
        if let Some(moving) = &self.cloud.moving {
            let progress = &moving.progress;
            let fraction = (progress.total > 0).then(|| progress.done as f32 / progress.total as f32);
            fx::progress_bar(ui, Id::new("cloud-move-progress"), fraction);
            ui.add_space(6.0);
            let mut cancel = false;
            ui.horizontal(|ui| {
                let done = (progress.done + 1).min(progress.total.max(1));
                ui.label(RichText::new(lang.moving_progress(done, progress.total)).color(p.on_surface));
                ui.label(RichText::new(&progress.current).small().color(p.on_surface_variant));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    cancel = fx::button(ui, ButtonStyle::Text, icon::X, lang.cancel()).clicked();
                });
            });
            if cancel {
                self.cloud.worker.cancel();
            }
            return;
        }
        let configured = expand_user(self.config.cloud.folder.trim());
        let pick_differs = self.cloud.pick.as_ref().is_some_and(|p| !same_folder(&p.path, &configured));
        let pending =
            self.cloud.choice != self.config.cloud.mode || (self.cloud.choice != CloudMode::Off && pick_differs);
        if pending {
            let (glyph, text) = match self.cloud.choice {
                CloudMode::Off => (icon::CLOUD_SLASH, lang.cloud_apply_off()),
                CloudMode::Move => (icon::CLOUD_ARROW_UP, lang.cloud_apply_move()),
                CloudMode::Mirror => (icon::COPY, lang.cloud_apply_mirror()),
            };
            let ready = self.cloud.choice == CloudMode::Off || self.cloud.pick.is_some();
            ui.horizontal(|ui| {
                ui.add_space(40.0);
                if ui.add_enabled_ui(ready, |ui| fx::button(ui, ButtonStyle::Filled, glyph, text)).inner.clicked() {
                    self.apply_cloud_choice();
                }
            });
            return;
        }
        let status = |ui: &mut egui::Ui, glyph: &str, color, text: String| {
            ui.horizontal(|ui| {
                ui.add_space(4.0);
                ui.label(RichText::new(glyph).size(18.0).color(color));
                ui.add_space(4.0);
                ui.add(egui::Label::new(RichText::new(text).color(p.on_surface)).wrap());
            });
        };
        // Files a move left at the old place (open at the time, failed, stopped): offer them again.
        if let Some((old, plan)) = self.cloud.leftover.clone() {
            let text = format!("{} {}", lang.relocate_details(0, 0, plan.files as usize), old.display());
            status(ui, icon::WARNING_CIRCLE, p.warning, text);
            ui.horizontal(|ui| {
                ui.add_space(30.0);
                if fx::button(ui, ButtonStyle::Tonal, icon::ARROWS_CLOCKWISE, lang.move_rest()).clicked() {
                    self.move_rest(old);
                }
            });
        }
        let name = self.cloud_name();
        let provider = self.config.cloud.provider.unwrap_or(Provider::Other);
        match self.config.cloud.mode {
            CloudMode::Off => {}
            CloudMode::Move => {
                let root = self.study_root().display().to_string();
                status(ui, icon::CLOUD_CHECK, p.success, lang.cloud_status_moved(&name, &root));
                status(ui, icon::DEVICES, p.on_surface_variant, lang.cloud_other_devices(provider));
            }
            CloudMode::Mirror => {
                let copy = cloud_study_root(&self.config).map(|c| c.display().to_string()).unwrap_or_default();
                status(ui, icon::COPY, p.success, lang.cloud_status_mirror(&name, &copy));
                if self.cloud.syncing {
                    ui.add_space(4.0);
                    fx::progress_bar(ui, Id::new("cloud-sync-progress"), None);
                    status(ui, icon::ARROWS_CLOCKWISE, p.on_surface_variant, lang.syncing().into());
                } else if let Some((at, report)) = &self.cloud.last_pass {
                    if report.unreachable {
                        status(ui, icon::CLOUD_WARNING, p.warning, lang.cloud_unreachable(&name));
                    } else {
                        let ago = lang.ago((self.now() - at).max(0.0) as u64);
                        status(
                            ui,
                            icon::CLOUD_CHECK,
                            p.on_surface_variant,
                            lang.last_sync(&ago, report.copied, report.updated),
                        );
                    }
                    if report.changed_in_cloud > 0 {
                        status(ui, icon::INFO, p.on_surface_variant, lang.changed_in_cloud(report.changed_in_cloud));
                    }
                }
                status(ui, icon::DEVICES, p.on_surface_variant, lang.cloud_other_devices(provider));
                ui.horizontal(|ui| {
                    ui.add_space(30.0);
                    let idle = !self.cloud.syncing;
                    if ui
                        .add_enabled_ui(idle, |ui| {
                            fx::button(ui, ButtonStyle::Text, icon::ARROWS_CLOCKWISE, lang.sync_now())
                        })
                        .inner
                        .clicked()
                    {
                        self.cloud.worker.send(Job::MirrorNow);
                    }
                });
            }
        }
    }

    // ---------- dialogs ----------

    pub(super) fn cloud_dialog_ui(&mut self, ctx: &egui::Context) {
        let Some(dialog) = &self.cloud.dialog else { return };
        let lang = self.lang;
        let mut close = false;
        // 0: the main button, 1: the second one ("Leave it in OneDrive", "Keep mine").
        let mut chose: Option<u8> = None;
        let modal = egui::Modal::new(Id::new("cloud-dialog")).frame(dialog_frame(ctx)).show(ctx, |ui| {
            let p = fx::pal(ui);
            ui.set_width(460.0);
            let text = |ui: &mut egui::Ui, text: String| {
                ui.add(egui::Label::new(RichText::new(text).color(p.on_surface_variant)).wrap());
                ui.add_space(6.0);
            };
            let title = |ui: &mut egui::Ui, title: String| {
                ui.label(fx::title(title, 22.0).color(p.on_surface));
                ui.add_space(12.0);
            };
            let mut buttons = |ui: &mut egui::Ui, main: &str, second: Option<String>, cancel: bool| {
                actions(ui, |ui| {
                    if fx::button(ui, ButtonStyle::Filled, "", main).clicked() {
                        chose = Some(0);
                    }
                    if let Some(second) = &second
                        && fx::button(ui, ButtonStyle::Text, "", second).clicked()
                    {
                        chose = Some(1);
                    }
                    if cancel && fx::button(ui, ButtonStyle::Text, "", lang.cancel()).clicked() {
                        close = true;
                    }
                });
            };
            match dialog {
                CloudDialog::Move { root, from, to, plan, existing } => {
                    title(ui, lang.confirm_move_title(&self.pick_name(root)));
                    let size = lang.size(plan.bytes);
                    text(
                        ui,
                        lang.confirm_move_text(
                            plan.files,
                            &size,
                            &from.display().to_string(),
                            &to.display().to_string(),
                        ),
                    );
                    if existing.files > 0 {
                        text(ui, lang.confirm_existing(existing.files));
                    }
                    text(ui, lang.confirm_close_documents().into());
                    buttons(ui, lang.move_button(), None, true);
                }
                CloudDialog::Mirror { root, to, plan } => {
                    title(ui, lang.confirm_mirror_title(&self.pick_name(root)));
                    text(ui, lang.confirm_mirror_text(plan.files, &lang.size(plan.bytes), &to.display().to_string()));
                    buttons(ui, lang.mirror_button(), None, true);
                }
                CloudDialog::Back { from, to, then } => {
                    title(ui, lang.back_title().into());
                    text(ui, lang.back_text(&from.display().to_string(), &to.display().to_string()));
                    text(ui, lang.confirm_close_documents().into());
                    let leave = (*then == CloudMode::Off).then(|| lang.leave_button(&self.cloud_name()));
                    buttons(ui, lang.back_button(), leave, true);
                }
                CloudDialog::Shared(remote) => {
                    title(ui, lang.shared_conflict_title().into());
                    text(
                        ui,
                        lang.shared_conflict_text(&remote.updated_by, remote.courses.len(), self.config.courses.len()),
                    );
                    buttons(ui, lang.take_cloud(), Some(lang.keep_mine().into()), false);
                }
                CloudDialog::Message(message) => {
                    text(ui, message.clone());
                    buttons(ui, lang.ok(), None, false);
                }
            }
        });
        let is_shared = matches!(self.cloud.dialog, Some(CloudDialog::Shared(_)));
        if modal.should_close() && !is_shared {
            close = true;
        }
        if let Some(button) = chose {
            match self.cloud.dialog.take() {
                Some(CloudDialog::Move { root, from, to, .. }) => self.start_move(&root, from, to),
                Some(CloudDialog::Mirror { root, .. }) => {
                    self.connect(CloudMode::Mirror, &root);
                    self.cloud.last_pass = None;
                }
                Some(CloudDialog::Back { from, to, then }) if button == 0 => self.start_back(from, to, then),
                Some(CloudDialog::Back { .. }) => {
                    // Leave it in the cloud: File Sorter just stops managing it.
                    self.config.cloud.mode = CloudMode::Off;
                    self.cloud.choice = CloudMode::Off;
                    self.save_quietly();
                }
                Some(CloudDialog::Shared(remote)) if button == 0 => self.take_shared(remote),
                Some(CloudDialog::Shared(remote)) => {
                    // Ours win: written as replacing theirs.
                    let file = cloud_study_root(&self.config).map(|root| root.join(SHARED_FILE));
                    if let Some(file) = file {
                        self.config.cloud.shared_seen = remote.updated_at;
                        let now_ms = chrono::Local::now().timestamp_millis().max(remote.updated_at + 1);
                        self.write_shared(&file, now_ms);
                    }
                }
                Some(CloudDialog::Message(_)) | None => {}
            }
        } else if close {
            self.cloud.dialog = None;
            self.cloud.choice = self.config.cloud.mode;
        }
    }
}
