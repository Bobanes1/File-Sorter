//! Right-click → "Sort with File Sorter": sorts the chosen files and shows a small
//! results window. The same window shows what the main window sorted by itself while it
//! wasn't in front (`--show-results`).
//!
//! Explorer starts one File Sorter per selected file. They all drop their file into a
//! shared inbox; the first one becomes the leader, sorts everything that arrives and
//! shows it in a single window, the others hand over their file and quit.

use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, BufReader, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread;
use std::time::{Duration, Instant};

use eframe::egui::{self, Align, Layout, Margin, Rect, RichText, ViewportCommand, vec2};
use egui_phosphor::regular as icon;

use crate::config::{Config, load_config};
use crate::fx;
use crate::fx::ButtonStyle;
use crate::gui::{Card, CardKind, FileState, Finished, RowAction, activity_row, app_icon};
use crate::i18n::Lang;
use crate::sorter::{SortResult, sort_chosen};
use crate::undo::undo;

/// How long a follower waits for a leader to pick up its file before taking over.
const FOLLOWER_PATIENCE: Duration = Duration::from_secs(20);
/// Without a window: stop once nothing new arrived for this long.
const QUIET_IDLE: Duration = Duration::from_millis(1500);
/// The results window closes this long after the last file (unless the mouse is over it).
const CLOSE_AFTER_SECS: f64 = 6.0;

/// A queue of files waiting to be sorted, shared by all File Sorter processes.
pub struct Inbox {
    queue: PathBuf,
    lock: PathBuf,
}

impl Inbox {
    pub fn new(dir: &Path) -> Inbox {
        Inbox { queue: dir.join("sort-queue.txt"), lock: dir.join("sort.lock") }
    }

    /// Results of automatic sorting waiting to be shown (one JSON line each).
    pub fn results(dir: &Path) -> Inbox {
        Inbox { queue: dir.join("results-queue.txt"), lock: dir.join("results.lock") }
    }

    fn submit_lines(&self, lines: &[String]) -> io::Result<()> {
        self.with_queue(|file| {
            file.seek(SeekFrom::End(0))?;
            for line in lines {
                writeln!(file, "{line}")?;
            }
            file.flush()
        })
    }

    fn take_lines(&self) -> io::Result<Vec<String>> {
        self.with_queue(|file| {
            file.seek(SeekFrom::Start(0))?;
            let lines: Vec<String> = BufReader::new(&*file)
                .lines()
                .collect::<io::Result<Vec<_>>>()?
                .into_iter()
                .filter(|l| !l.is_empty())
                .collect();
            file.set_len(0)?;
            Ok(lines)
        })
    }

    /// The lock, if no other process holds it.
    fn try_lead(&self) -> io::Result<Option<File>> {
        if let Some(dir) = self.lock.parent() {
            fs::create_dir_all(dir)?;
        }
        let lock = OpenOptions::new().write(true).create(true).truncate(false).open(&self.lock)?;
        match lock.try_lock() {
            Ok(()) => Ok(Some(lock)),
            Err(fs::TryLockError::WouldBlock) => Ok(None),
            Err(fs::TryLockError::Error(e)) => Err(e),
        }
    }

    fn with_queue<T>(&self, f: impl FnOnce(&mut File) -> io::Result<T>) -> io::Result<T> {
        if let Some(dir) = self.queue.parent() {
            fs::create_dir_all(dir)?;
        }
        let mut file = OpenOptions::new().read(true).write(true).create(true).truncate(false).open(&self.queue)?;
        file.lock()?;
        let result = f(&mut file);
        let _ = file.unlock();
        result
    }

    fn read(file: &mut File) -> io::Result<Vec<PathBuf>> {
        file.seek(SeekFrom::Start(0))?;
        BufReader::new(&*file)
            .lines()
            .map(|line| line.map(PathBuf::from))
            .filter(|p| !matches!(p, Ok(p) if p.as_os_str().is_empty()))
            .collect()
    }

    pub fn submit(&self, paths: &[PathBuf]) -> io::Result<()> {
        self.with_queue(|file| {
            file.seek(SeekFrom::End(0))?;
            for path in paths {
                writeln!(file, "{}", path.to_string_lossy())?;
            }
            file.flush()
        })
    }

    pub fn take_all(&self) -> io::Result<Vec<PathBuf>> {
        self.with_queue(|file| {
            let items = Self::read(file)?;
            file.set_len(0)?;
            Ok(items)
        })
    }

    fn contains_any(&self, paths: &[PathBuf]) -> bool {
        self.with_queue(Self::read).map(|items| paths.iter().any(|p| items.contains(p))).unwrap_or(false)
    }

    /// Become the leader (returns the held lock), or `None` once another leader took our files.
    pub fn lead(&self, mine: &[PathBuf], patience: Duration) -> io::Result<Option<File>> {
        let deadline = Instant::now() + patience;
        loop {
            let lock = OpenOptions::new().write(true).create(true).truncate(false).open(&self.lock)?;
            match lock.try_lock() {
                Ok(()) => return Ok(Some(lock)),
                Err(fs::TryLockError::WouldBlock) => {}
                Err(fs::TryLockError::Error(e)) => return Err(e),
            }
            if !self.contains_any(mine) || Instant::now() > deadline {
                return Ok(None);
            }
            thread::sleep(Duration::from_millis(100));
        }
    }
}

/// Sort `paths`, together with whatever other File Sorter processes hand in.
pub fn run(config_path: &Path, paths: Vec<PathBuf>, quiet: bool) -> ExitCode {
    let paths: Vec<PathBuf> = paths.into_iter().map(|p| std::path::absolute(&p).unwrap_or(p)).collect();
    let inbox = Inbox::new(config_path.parent().unwrap_or(Path::new(".")));
    if let Err(e) = inbox.submit(&paths) {
        log::error!("Could not queue the files to sort: {e}");
        return ExitCode::FAILURE;
    }
    let _lock = match inbox.lead(&paths, FOLLOWER_PATIENCE) {
        Ok(Some(lock)) => lock,
        Ok(None) => return ExitCode::SUCCESS, // another File Sorter shows them
        Err(e) => {
            log::error!("Could not coordinate with other File Sorter windows: {e}");
            return ExitCode::FAILURE;
        }
    };
    let config = match load_config(config_path) {
        Ok(config) => config,
        Err(e) => {
            log::error!("Could not read {}: {e}", config_path.display());
            return ExitCode::FAILURE;
        }
    };
    if quiet || !config.results_window {
        sort_until_idle(&inbox, &config);
        return ExitCode::SUCCESS;
    }

    let stop = Arc::new(AtomicBool::new(false));
    let (tx, rx) = channel::<Vec<SortResult>>();
    let worker = {
        let stop = Arc::clone(&stop);
        let config = config.clone();
        thread::spawn(move || {
            while !stop.load(Ordering::SeqCst) {
                match inbox.take_all() {
                    Ok(batch) if !batch.is_empty() => {
                        let results = sort_chosen(&batch, &config);
                        log_results(&results);
                        if tx.send(results).is_err() {
                            break;
                        }
                    }
                    _ => thread::sleep(Duration::from_millis(150)),
                }
            }
        })
    };
    show_window(config, rx, &stop);
    stop.store(true, Ordering::SeqCst);
    let _ = worker.join(); // let a move that is in progress finish
    ExitCode::SUCCESS
}

/// A result handed to the results window, with its history entry (to undo it).
#[derive(serde::Serialize, serde::Deserialize)]
struct Queued {
    id: Option<u64>,
    result: SortResult,
}

/// Don't start another results window within this long of the last (it is still starting).
const STARTING: Duration = Duration::from_secs(4);

/// Show `results` (sorted by the main window while it wasn't in front) in the results
/// window, starting one if none is open.
pub fn show_results(config_path: &Path, state_dir: &Path, results: &[SortResult]) {
    static STARTED: std::sync::Mutex<Option<Instant>> = std::sync::Mutex::new(None);
    let inbox = Inbox::results(state_dir);
    let lines: Vec<String> =
        results.iter().filter_map(|r| serde_json::to_string(&Queued { id: r.id, result: r.clone() }).ok()).collect();
    if let Err(e) = inbox.submit_lines(&lines) {
        log::info!("Could not hand the results to the results window: {e}");
        return;
    }
    let mut started = STARTED.lock().unwrap_or_else(|e| e.into_inner());
    if started.is_some_and(|t| t.elapsed() < STARTING) {
        return;
    }
    // One is open: it takes them from the queue.
    match inbox.try_lead() {
        Ok(Some(lock)) => drop(lock),
        _ => return,
    }
    let Ok(exe) = std::env::current_exe() else { return };
    match std::process::Command::new(exe).arg("--config").arg(config_path).arg("--show-results").spawn() {
        Ok(_) => *started = Some(Instant::now()),
        Err(e) => log::info!("Could not open the results window: {e}"),
    }
}

/// `--show-results`: the results window for what the main window sorted by itself.
pub fn run_results(config_path: &Path) -> ExitCode {
    let config = match load_config(config_path) {
        Ok(config) => config,
        Err(e) => {
            log::error!("Could not read {}: {e}", config_path.display());
            return ExitCode::FAILURE;
        }
    };
    let dir = config.state_dir.clone().unwrap_or_else(|| config_path.parent().unwrap_or(Path::new(".")).into());
    let inbox = Inbox::results(&dir);
    let lock = match inbox.try_lead() {
        Ok(Some(lock)) => lock,
        Ok(None) => return ExitCode::SUCCESS, // one is open already
        Err(e) => {
            log::error!("Could not open the results window: {e}");
            return ExitCode::FAILURE;
        }
    };
    let stop = Arc::new(AtomicBool::new(false));
    let (tx, rx) = channel::<Vec<SortResult>>();
    let worker = {
        let stop = Arc::clone(&stop);
        let inbox = Inbox::results(&dir);
        thread::spawn(move || {
            while !stop.load(Ordering::SeqCst) {
                let batch: Vec<SortResult> = inbox
                    .take_lines()
                    .unwrap_or_default()
                    .iter()
                    .filter_map(|l| serde_json::from_str::<Queued>(l).ok())
                    .map(|q| SortResult { id: q.id, ..q.result })
                    .collect();
                if batch.is_empty() {
                    thread::sleep(Duration::from_millis(200));
                } else if tx.send(batch).is_err() {
                    break;
                }
            }
        })
    };
    show_window(config, rx, &stop);
    stop.store(true, Ordering::SeqCst);
    let _ = worker.join();
    drop(lock);
    // Results that came in just as the window closed: show them in a new one.
    if fs::metadata(&inbox.queue).is_ok_and(|m| m.len() > 0)
        && let Ok(exe) = std::env::current_exe()
    {
        let _ = std::process::Command::new(exe).arg("--config").arg(config_path).arg("--show-results").spawn();
    }
    ExitCode::SUCCESS
}

/// The results window, showing what arrives on `rx` until it closes.
fn show_window(config: Config, rx: Receiver<Vec<SortResult>>, stop: &Arc<AtomicBool>) {
    let lang = Lang::from_setting(config.language.as_deref());
    let (theme, motion) = (config.theme, config.animations);
    let mut viewport = egui::ViewportBuilder::default()
        .with_title("File Sorter")
        .with_icon(Arc::new(app_icon()))
        .with_inner_size(POPUP_SIZE)
        .with_min_inner_size([320.0, 200.0])
        .with_always_on_top();
    // Bottom right, above the taskbar like a notification: not under the mouse, which
    // would pause the countdown.
    if let Some([_, _, right, bottom]) = crate::winwin::work_area() {
        let scale = crate::winwin::screen_scale();
        viewport =
            viewport.with_position([right / scale - POPUP_SIZE[0] - 24.0, bottom / scale - POPUP_SIZE[1] - 56.0]);
    }
    let options = eframe::NativeOptions { viewport, ..Default::default() };
    let window_stop = Arc::clone(stop);
    let shown = eframe::run_native(
        "File Sorter",
        options,
        Box::new(move |cc| {
            fx::install(&cc.egui_ctx);
            fx::apply_theme(&cc.egui_ctx, theme);
            fx::set_motion(&cc.egui_ctx, motion);
            Ok(Box::new(ResultsWindow::new(lang, config, rx, window_stop)))
        }),
    );
    if let Err(e) = shown {
        log::error!("Could not open the results window: {e}");
    }
}

/// Logical size of the results window.
const POPUP_SIZE: [f32; 2] = [500.0, 340.0];

fn log_results(results: &[SortResult]) {
    for result in results {
        log::info!("{}", result.describe());
    }
}

/// Without a window: sort until nothing new has arrived for a moment.
fn sort_until_idle(inbox: &Inbox, config: &Config) {
    let mut idle_since = Instant::now();
    while idle_since.elapsed() < QUIET_IDLE {
        match inbox.take_all() {
            Ok(batch) if !batch.is_empty() => {
                log_results(&sort_chosen(&batch, config));
                idle_since = Instant::now();
            }
            _ => thread::sleep(Duration::from_millis(150)),
        }
    }
}

/// The small window listing what happened to the chosen files, with "Undo" for each.
struct ResultsWindow {
    lang: Lang,
    config: Config,
    rx: Receiver<Vec<SortResult>>,
    stop: Arc<AtomicBool>,
    cards: Vec<Card>,
    sorted: usize,
    /// When the countdown to closing (re)started.
    countdown_from: f64,
    keep_open: bool,
    /// Finished undos and assignments, with the entry's key.
    undo_tx: Sender<(u64, Finished)>,
    undo_rx: Receiver<(u64, Finished)>,
    /// Moved to the bottom right (where the system didn't say where that is up front).
    placed: bool,
}

impl ResultsWindow {
    fn new(lang: Lang, config: Config, rx: Receiver<Vec<SortResult>>, stop: Arc<AtomicBool>) -> Self {
        let (undo_tx, undo_rx) = channel();
        ResultsWindow {
            lang,
            config,
            rx,
            stop,
            cards: Vec::new(),
            sorted: 0,
            countdown_from: 0.0,
            keep_open: false,
            undo_tx,
            undo_rx,
            placed: crate::winwin::work_area().is_some(),
        }
    }

    fn start_undo(&mut self, key: u64, ctx: &egui::Context, now: f64) {
        let Some(card) = self.cards.iter_mut().find(|c| c.key == key) else { return };
        let Some(result) = card.undoable().cloned() else { return };
        card.set_state(FileState::Undoing(result.clone()), now);
        let (tx, config, ctx) = (self.undo_tx.clone(), self.config.clone(), ctx.clone());
        thread::spawn(move || {
            let _ = tx.send((key, Finished::Undo(undo(&result, &config))));
            ctx.request_repaint();
        });
    }

    /// Put an entry's file into module `course`. (Keywords are only learned in the main window,
    /// which owns the settings.)
    fn start_assign(&mut self, key: u64, course: usize, ctx: &egui::Context, now: f64) {
        let Some(card) = self.cards.iter_mut().find(|c| c.key == key) else { return };
        let Some(result) = card.assignable().cloned() else { return };
        card.set_state(FileState::Assigning(result.clone()), now);
        let (tx, config, ctx) = (self.undo_tx.clone(), self.config.clone(), ctx.clone());
        thread::spawn(move || {
            let _ = tx.send((key, Finished::Assigned(crate::undo::reassign(&result, course, &config))));
            ctx.request_repaint();
        });
    }
}

impl eframe::App for ResultsWindow {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let now = ctx.input(|i| i.time);
        let lang = self.lang;
        let p = fx::pal(ui);
        // Linux: the screen's size is known once the window is open (X11 places it there;
        // Wayland decides by itself).
        if !self.placed
            && let Some(screen) = ctx.input(|i| i.viewport().monitor_size)
        {
            self.placed = true;
            let at = egui::pos2(screen.x - POPUP_SIZE[0] - 24.0, screen.y - POPUP_SIZE[1] - 72.0);
            ctx.send_viewport_cmd(ViewportCommand::OuterPosition(at));
        }
        while let Ok(batch) = self.rx.try_recv() {
            for result in batch {
                if result.sorted() {
                    self.sorted += 1;
                }
                let kind = CardKind::File { name: result.file_name(), state: FileState::Done(result) };
                self.cards.push(Card::new(kind, now));
            }
            self.countdown_from = now;
        }
        while let Ok((key, finished)) = self.undo_rx.try_recv() {
            if let Some(card) = self.cards.iter_mut().find(|c| c.key == key)
                && let CardKind::File { state: FileState::Undoing(result) | FileState::Assigning(result), .. } =
                    &card.kind
            {
                let result = result.clone();
                let state = match finished {
                    Finished::Undo(Ok(back)) => {
                        self.sorted = self.sorted.saturating_sub(1);
                        FileState::Undone { back }
                    }
                    Finished::Assigned(Ok(new)) => {
                        if new.sorted() && !result.sorted() {
                            self.sorted += 1;
                        }
                        FileState::Done(new)
                    }
                    Finished::Undo(Err(error)) | Finished::Assigned(Err(error)) => {
                        FileState::UndoFailed { result, error }
                    }
                };
                card.set_state(state, now);
            }
            self.countdown_from = now;
        }
        // The countdown waits while the mouse is over the window or a file is being put back.
        let busy = self
            .cards
            .iter()
            .any(|c| matches!(c.kind, CardKind::File { state: FileState::Undoing(_) | FileState::Assigning(_), .. }));
        let hovered = ctx.input(|i| i.pointer.hover_pos().is_some());
        if hovered || busy {
            self.countdown_from = now;
        }
        let remaining = (CLOSE_AFTER_SECS - (now - self.countdown_from)).max(0.0);
        let done = !self.cards.is_empty();
        if done && !self.keep_open && remaining <= 0.0 {
            ctx.send_viewport_cmd(ViewportCommand::Close);
        }

        egui::Panel::top("quick-header")
            .frame(egui::Frame::new().fill(p.surface).inner_margin(Margin { left: 16, right: 16, top: 12, bottom: 4 }))
            .show_separator_line(false)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    let (logo, _) = ui.allocate_exact_size(vec2(32.0, 32.0), egui::Sense::hover());
                    fx::avatar(
                        ui.painter(),
                        logo.center(),
                        16.0,
                        icon::GRADUATION_CAP,
                        p.primary_container,
                        p.on_primary_container,
                    );
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        ui.label(fx::title("File Sorter", 15.0).color(p.on_surface));
                        if done {
                            let color = if self.sorted == self.cards.len() { p.success } else { p.warning };
                            ui.label(
                                RichText::new(lang.quick_done(self.sorted, self.cards.len())).small().color(color),
                            );
                        } else {
                            ui.horizontal(|ui| {
                                ui.add(egui::Spinner::new().size(12.0).color(p.primary));
                                ui.label(RichText::new(lang.quick_sorting()).small().color(p.primary));
                            });
                        }
                    });
                });
            });
        egui::Panel::bottom("quick-footer")
            .frame(egui::Frame::new().fill(p.surface).inner_margin(Margin { left: 12, right: 12, top: 4, bottom: 8 }))
            .show_separator_line(false)
            .show(ui, |ui| {
                if done && !self.keep_open {
                    // A thin countdown bar, Material "linear progress" style.
                    let (bar, _) = ui.allocate_exact_size(vec2(ui.available_width(), 4.0), egui::Sense::hover());
                    ui.painter().rect_filled(bar, 2, p.container_highest);
                    let fraction = (remaining / CLOSE_AFTER_SECS) as f32;
                    let filled = Rect::from_min_size(bar.min, vec2(bar.width() * fraction, bar.height()));
                    ui.painter().rect_filled(filled, 2, p.primary);
                }
                ui.horizontal(|ui| {
                    if done && !self.keep_open {
                        let text =
                            if hovered { lang.paused().to_string() } else { lang.closes_in(remaining.ceil() as u32) };
                        ui.label(RichText::new(text).small().color(p.on_surface_variant));
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if fx::button(ui, ButtonStyle::Text, "", lang.close()).clicked() {
                            ctx.send_viewport_cmd(ViewportCommand::Close);
                        }
                        if done
                            && !self.keep_open
                            && fx::button(ui, ButtonStyle::Text, icon::PUSH_PIN, lang.keep_open()).clicked()
                        {
                            self.keep_open = true;
                        }
                    });
                });
            });
        let mut actions = Vec::new();
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(p.surface).inner_margin(Margin::symmetric(8, 0)))
            .show(ui, |ui| {
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    let modules: Vec<String> = self.config.courses.iter().map(|c| c.label()).collect();
                    let running = crate::timetable::running_modules(&self.config, chrono::Local::now().naive_local())
                        .into_iter()
                        .next();
                    for card in self.cards.iter().rev() {
                        match activity_row(ui, lang, card, now, &modules, running.as_deref()) {
                            RowAction::None => {}
                            action => actions.push((card.key, action)),
                        }
                    }
                });
            });
        for (key, action) in actions {
            match action {
                RowAction::OpenFile(path) => crate::desktop::open(&path),
                RowAction::ShowFolder(path) => crate::desktop::reveal(&path),
                RowAction::Undo => self.start_undo(key, &ctx, now),
                RowAction::Assign(course) => self.start_assign(key, course, &ctx, now),
                RowAction::None => {}
            }
        }
        ctx.request_repaint_after(Duration::from_millis(100));
    }

    fn on_exit(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::save_config;
    use crate::sorter::tests::{make_pdf, test_config};

    #[test]
    fn results_are_handed_over_with_their_history_entry() {
        let tmp = tempfile::tempdir().unwrap();
        let inbox = Inbox::results(tmp.path());
        let mut result = SortResult::new(
            PathBuf::from("/dl/a.pdf"),
            crate::sorter::Outcome::Moved {
                course: "Statistik".into(),
                destination: PathBuf::from("/uni/Statistik/a.pdf"),
                reason: crate::matcher::Reason::Chosen,
            },
        );
        result.id = Some(42);
        let line = serde_json::to_string(&Queued { id: result.id, result: result.clone() }).unwrap();
        inbox.submit_lines(&[line]).unwrap();
        let lines = inbox.take_lines().unwrap();
        let back: Queued = serde_json::from_str(&lines[0]).unwrap();
        assert_eq!(SortResult { id: back.id, ..back.result }, result);
        assert!(inbox.take_lines().unwrap().is_empty());
        // Only one results window: the second can't take the lock.
        let first = inbox.try_lead().unwrap();
        assert!(first.is_some() && inbox.try_lead().unwrap().is_none());
    }

    #[test]
    fn inbox_queues_and_drains() {
        let tmp = tempfile::tempdir().unwrap();
        let inbox = Inbox::new(tmp.path());
        let (a, b) = (tmp.path().join("a.pdf"), tmp.path().join("b c.pdf"));
        inbox.submit(std::slice::from_ref(&a)).unwrap();
        inbox.submit(std::slice::from_ref(&b)).unwrap();
        assert!(inbox.contains_any(std::slice::from_ref(&b)));
        assert_eq!(inbox.take_all().unwrap(), vec![a, b.clone()]);
        assert!(inbox.take_all().unwrap().is_empty());
        assert!(!inbox.contains_any(&[b]));
    }

    #[test]
    fn only_one_leader_and_followers_hand_over() {
        let tmp = tempfile::tempdir().unwrap();
        let inbox = Inbox::new(tmp.path());
        let mine = vec![tmp.path().join("x.pdf")];
        inbox.submit(&mine).unwrap();
        let leader = inbox.lead(&mine, Duration::from_secs(1)).unwrap().expect("first one leads");

        // A second process: its file is still queued, so it waits; once the leader takes
        // the files, it steps back.
        let theirs = vec![tmp.path().join("y.pdf")];
        inbox.submit(&theirs).unwrap();
        let follower = {
            let dir = tmp.path().to_path_buf();
            let theirs = theirs.clone();
            thread::spawn(move || Inbox::new(&dir).lead(&theirs, Duration::from_secs(10)).unwrap().is_some())
        };
        thread::sleep(Duration::from_millis(300));
        assert_eq!(inbox.take_all().unwrap().len(), 2, "the leader collects both files");
        assert!(!follower.join().unwrap(), "follower must not lead once its file was taken");
        drop(leader);

        // With the leader gone, the next process leads.
        assert!(inbox.lead(&[], Duration::from_millis(100)).unwrap().is_some());
    }

    /// Several right-clicked files at once (one process each): every file is sorted
    /// exactly once.
    #[test]
    fn many_processes_sort_every_file_once() {
        let tmp = tempfile::tempdir().unwrap();
        let config = test_config(tmp.path());
        let config_path = tmp.path().join("cfg").join("config.json");
        save_config(&config, &config_path).unwrap();
        let desktop = tmp.path().join("Desktop");
        fs::create_dir_all(&desktop).unwrap();
        let files: Vec<PathBuf> = (0..6)
            .map(|i| {
                let f = desktop.join(format!("python_{i}.pdf"));
                make_pdf(&f, "x");
                f
            })
            .collect();
        let handles: Vec<_> = files
            .iter()
            .map(|f| {
                let (cp, f) = (config_path.clone(), f.clone());
                thread::spawn(move || run(&cp, vec![f], true))
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
        let folder = tmp.path().join("University").join("Programming Course");
        let mut sorted: Vec<_> = fs::read_dir(&folder).unwrap().flatten().map(|e| e.file_name()).collect();
        sorted.sort();
        assert_eq!(sorted.len(), 6, "{sorted:?}");
        assert!(files.iter().all(|f| !f.exists()));
    }
}
