//! Right-click → "Sort with File Sorter": sorts the chosen files and shows a small
//! results window.
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
use std::sync::mpsc::{Receiver, channel};
use std::thread;
use std::time::{Duration, Instant};

use eframe::egui::{self, Color32, Margin, ProgressBar, RichText, ViewportCommand, vec2};
use egui_phosphor::regular as icon;

use crate::config::{Config, load_config};
use crate::fx;
use crate::gui::{Card, CardKind, FileState, app_icon, card_ui};
use crate::i18n::Lang;
use crate::sorter::{SortResult, sort_chosen};

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

    let lang = Lang::from_setting(config.language.as_deref());
    let (theme, motion) = (config.theme, config.animations);
    let stop = Arc::new(AtomicBool::new(false));
    let (tx, rx) = channel::<Vec<SortResult>>();
    let worker = {
        let stop = Arc::clone(&stop);
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
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("File Sorter")
            .with_icon(Arc::new(app_icon()))
            .with_inner_size([480.0, 400.0])
            .with_min_inner_size([360.0, 240.0])
            .with_always_on_top(),
        ..Default::default()
    };
    let window_stop = Arc::clone(&stop);
    let shown = eframe::run_native(
        "File Sorter",
        options,
        Box::new(move |cc| {
            fx::install(&cc.egui_ctx);
            fx::apply_theme(&cc.egui_ctx, theme);
            fx::set_motion(&cc.egui_ctx, motion);
            Ok(Box::new(ResultsWindow::new(lang, rx, window_stop)))
        }),
    );
    stop.store(true, Ordering::SeqCst);
    let _ = worker.join(); // let a move that is in progress finish
    if let Err(e) = shown {
        log::error!("Could not open the results window: {e}");
    }
    ExitCode::SUCCESS
}

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

/// The small window listing what happened to the chosen files.
struct ResultsWindow {
    lang: Lang,
    rx: Receiver<Vec<SortResult>>,
    stop: Arc<AtomicBool>,
    cards: Vec<Card>,
    sorted: usize,
    last_activity: f64,
    keep_open: bool,
}

impl ResultsWindow {
    fn new(lang: Lang, rx: Receiver<Vec<SortResult>>, stop: Arc<AtomicBool>) -> Self {
        ResultsWindow { lang, rx, stop, cards: Vec::new(), sorted: 0, last_activity: 0.0, keep_open: false }
    }
}

impl eframe::App for ResultsWindow {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let now = ctx.input(|i| i.time);
        let lang = self.lang;
        while let Ok(batch) = self.rx.try_recv() {
            for result in batch {
                if result.sorted() {
                    self.sorted += 1;
                }
                let kind = CardKind::File { name: result.file_name(), state: FileState::Done(result.outcome) };
                self.cards.push(Card::new(kind, now));
            }
            self.last_activity = now;
        }
        // The countdown pauses while the mouse is over the window.
        if ctx.input(|i| i.pointer.hover_pos().is_some()) {
            self.last_activity = now;
        }
        let remaining = (CLOSE_AFTER_SECS - (now - self.last_activity)).max(0.0);
        let done = !self.cards.is_empty();
        if done && !self.keep_open && remaining <= 0.0 {
            ctx.send_viewport_cmd(ViewportCommand::Close);
        }

        let accent = fx::accent(ui);
        egui::Panel::top("quick-header")
            .frame(
                egui::Frame::new()
                    .fill(fx::lerp_color(ui.visuals().panel_fill, accent, 0.06))
                    .inner_margin(Margin::symmetric(12, 10)),
            )
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(icon::GRADUATION_CAP).size(24.0).color(accent));
                    ui.vertical(|ui| {
                        ui.label(RichText::new("File Sorter").size(18.0).strong());
                        if done {
                            let color = if self.sorted == self.cards.len() { fx::GREEN } else { fx::AMBER };
                            ui.label(RichText::new(lang.quick_done(self.sorted, self.cards.len())).color(color));
                        } else {
                            ui.horizontal(|ui| {
                                ui.add(egui::Spinner::new().color(fx::BLUE));
                                ui.label(RichText::new(lang.quick_sorting()).color(fx::BLUE));
                            });
                        }
                    });
                });
            });
        egui::Panel::bottom("quick-footer").show(ui, |ui| {
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui.button(format!("{} {}", icon::X, lang.close())).clicked() {
                    ctx.send_viewport_cmd(ViewportCommand::Close);
                }
                if !self.keep_open && done {
                    if ui.button(format!("{} {}", icon::PUSH_PIN, lang.keep_open())).clicked() {
                        self.keep_open = true;
                    }
                    let fraction = (remaining / CLOSE_AFTER_SECS) as f32;
                    ui.add(
                        ProgressBar::new(fraction)
                            .desired_width(ui.available_width())
                            .fill(accent.gamma_multiply(0.6))
                            .text(RichText::new(lang.closes_in(remaining.ceil() as u32)).color(Color32::WHITE)),
                    );
                }
            });
            ui.add_space(4.0);
        });
        egui::CentralPanel::default_margins().show(ui, |ui| {
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                ui.spacing_mut().item_spacing = vec2(8.0, 4.0);
                for card in self.cards.iter().rev() {
                    if card_ui(ui, lang, card, now).clicked()
                        && let Some(path) = card.destination()
                    {
                        crate::desktop::reveal(path);
                    }
                }
            });
        });
        ctx.request_repaint_after(Duration::from_millis(50));
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
