//! The cloud features' background thread: moving the study folder and keeping its copy
//! up to date take a while, and the window (or the watcher) must not wait for them.
//! While a copy is set up, the thread also brings it up to date every minute by itself,
//! so annotations made on this computer reach the cloud even while the window is hidden.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::time::{Duration, Instant};

use super::mirror::{MIRROR_STATE, Mirror, MirrorReport};
use super::relocate::{self, Progress, Report};

/// How often the copy is brought up to date while it is set up.
pub const MIRROR_EVERY: Duration = Duration::from_secs(60);

/// Work for the background thread.
#[derive(Debug, Clone, PartialEq)]
pub enum Job {
    /// Move the study folder `from` into `to` (into the cloud, or back).
    Relocate { from: PathBuf, to: PathBuf },
    /// Keep a copy of the study folder `local` in `cloud` (`None`: stop). Starts a pass now.
    SetMirror(Option<(PathBuf, PathBuf)>),
    /// Bring the copy up to date now.
    MirrorNow,
    /// `file` was just sorted into the study folder.
    MirrorFile(PathBuf),
    /// Sorting `file` was undone: its copy goes too, unless it was changed in the cloud.
    Forget(PathBuf),
}

/// What the background thread reports.
#[derive(Debug, Clone)]
pub enum CloudEvent {
    Progress(Progress),
    Relocated { from: PathBuf, to: PathBuf, result: Result<Report, String> },
    /// A pass over the whole copy (`full`), or one file.
    Mirrored { report: MirrorReport, full: bool },
    /// A pass over the whole copy started.
    MirrorStarted,
}

pub type CloudSink = Arc<dyn Fn(CloudEvent) + Send + Sync>;

pub struct Worker {
    jobs: Sender<Job>,
    cancel: Arc<AtomicBool>,
    busy: Arc<AtomicBool>,
    /// Quitting: the thread starts nothing new.
    stop: Arc<AtomicBool>,
}

impl Worker {
    /// Start the thread. The copy's state is kept in `state_dir` (File Sorter's settings folder).
    pub fn start(state_dir: Option<PathBuf>, sink: CloudSink) -> Worker {
        let (jobs, rx) = channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let busy = Arc::new(AtomicBool::new(false));
        let stop = Arc::new(AtomicBool::new(false));
        let flags = Flags { cancel: Arc::clone(&cancel), busy: Arc::clone(&busy), stop: Arc::clone(&stop) };
        let spawned = std::thread::Builder::new()
            .name("file-sorter-cloud".into())
            .spawn(move || Thread::new(state_dir, sink).run(&rx, &flags));
        if let Err(e) = spawned {
            log::error!("Could not start the cloud thread: {e}");
        }
        Worker { jobs, cancel, busy, stop }
    }

    pub fn send(&self, job: Job) {
        let _ = self.jobs.send(job);
    }

    /// Stop the running job between two files (a move stops cleanly; the rest stays put).
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    /// Before quitting: stop the running job between two files and wait (at most
    /// `timeout`) until it has, so no file is left half copied.
    pub fn finish(&self, timeout: Duration) {
        self.stop.store(true, Ordering::Relaxed);
        self.cancel();
        let deadline = Instant::now() + timeout;
        while self.busy.load(Ordering::Relaxed) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}

struct Flags {
    cancel: Arc<AtomicBool>,
    busy: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
}

struct Thread {
    state_dir: Option<PathBuf>,
    sink: CloudSink,
    mirror: Mirror,
    /// The study folder and its copy, while a copy is set up.
    copy: Option<(PathBuf, PathBuf)>,
    next_pass: Instant,
}

impl Thread {
    fn new(state_dir: Option<PathBuf>, sink: CloudSink) -> Thread {
        let state_file =
            state_dir.as_ref().map(|d| d.join(MIRROR_STATE)).unwrap_or_else(|| PathBuf::from(MIRROR_STATE));
        Thread { mirror: Mirror::load(&state_file), state_dir, sink, copy: None, next_pass: Instant::now() }
    }

    fn run(mut self, rx: &Receiver<Job>, flags: &Flags) {
        let (cancel, busy) = (&*flags.cancel, &*flags.busy);
        loop {
            busy.store(false, Ordering::Relaxed);
            if flags.stop.load(Ordering::Relaxed) {
                return;
            }
            let job = if self.copy.is_some() {
                match rx.recv_timeout(self.next_pass.saturating_duration_since(Instant::now())) {
                    Ok(job) => Some(job),
                    Err(RecvTimeoutError::Timeout) => None,
                    Err(RecvTimeoutError::Disconnected) => return,
                }
            } else {
                match rx.recv() {
                    Ok(job) => Some(job),
                    Err(_) => return,
                }
            };
            busy.store(true, Ordering::Relaxed);
            // A cancel is for the job that was running when it was asked for (cleared before
            // looking at `stop`, so a quit asked for right now still cancels this job).
            cancel.store(false, Ordering::Relaxed);
            if flags.stop.load(Ordering::Relaxed) {
                busy.store(false, Ordering::Relaxed);
                return;
            }
            match job {
                Some(Job::Relocate { from, to }) => self.relocate(from, to, cancel),
                Some(Job::SetMirror(copy)) => {
                    self.copy = copy;
                    self.next_pass = Instant::now();
                }
                Some(Job::MirrorNow) => self.next_pass = Instant::now(),
                Some(Job::MirrorFile(file)) => {
                    if let Some((local, cloud)) = &self.copy {
                        let report = self.mirror.sync_file(local, cloud, &file);
                        (self.sink)(CloudEvent::Mirrored { report, full: false });
                    }
                }
                Some(Job::Forget(file)) => {
                    if let Some((local, cloud)) = &self.copy {
                        match self.mirror.forget(local, cloud, &file) {
                            Ok(true) => log::info!("Removed the cloud copy of {} again", file.display()),
                            Ok(false) => {}
                            Err(e) => log::info!("Could not remove the cloud copy of {}: {e}", file.display()),
                        }
                    }
                }
                None => {}
            }
            if let Some((local, cloud)) = &self.copy
                && Instant::now() >= self.next_pass
            {
                (self.sink)(CloudEvent::MirrorStarted);
                let report = self.mirror.sync_all(local, cloud, cancel);
                (self.sink)(CloudEvent::Mirrored { report, full: true });
                self.next_pass = Instant::now() + MIRROR_EVERY;
            }
        }
    }

    fn relocate(&mut self, from: PathBuf, to: PathBuf, cancel: &AtomicBool) {
        log::info!("Moving the study folder from {} to {}", from.display(), to.display());
        let sink = Arc::clone(&self.sink);
        let result = relocate::relocate(&from, &to, cancel, &mut |p| sink(CloudEvent::Progress(p.clone())));
        if let (Ok(report), Some(dir)) = (&result, self.state_dir.as_deref()) {
            record(dir, &from, &to, report);
        }
        (self.sink)(CloudEvent::Relocated { from, to, result: result.map_err(|e| e.to_string()) });
    }
}

fn record(dir: &Path, from: &Path, to: &Path, report: &Report) {
    if let Err(e) = crate::history::record_relocation(dir, from, to, &report.renamed) {
        log::info!("Could not note the move in the history: {e}");
    }
}
