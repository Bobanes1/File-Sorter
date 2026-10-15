//! Watch the downloads folder and sort new files as soon as they finish downloading.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

use notify::event::{ModifyKind, RenameMode};
use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};

use crate::config::Config;
use crate::sorter::{
    LeftReason, Outcome, SortResult, TEMP_SUFFIXES, file_name, is_candidate, sort_existing, sort_file,
};

pub const STABLE_FOR: Duration = Duration::from_secs(2);
const POLL_INTERVAL: Duration = Duration::from_millis(500);
const MAX_WAIT: Duration = Duration::from_secs(600);
pub const RESCAN_INTERVAL: Duration = Duration::from_secs(10);
const DETECT_DELAY: Duration = Duration::from_secs(1);

/// What the watcher reports back to the GUI (or the log in headless mode).
#[derive(Debug, Clone)]
pub enum WatchEvent {
    /// A new file showed up; waiting for the download to finish.
    Detected(String),
    /// A file announced with `Detected` disappeared before it could be sorted.
    Vanished(String),
    Result(SortResult),
}

impl WatchEvent {
    /// English description, for the log file.
    pub fn text(&self) -> String {
        match self {
            WatchEvent::Detected(name) => format!("Detected {name} – waiting for the download to finish…"),
            WatchEvent::Vanished(name) => format!("{name} disappeared before it could be sorted"),
            WatchEvent::Result(r) => r.describe(),
        }
    }
}

/// Why watching could not start.
#[derive(Debug, Clone, PartialEq)]
pub enum StartError {
    NoFolder(PathBuf),
    /// Another File Sorter (window or `--headless`) is already watching.
    AlreadyRunning,
    Other(String),
}

impl std::fmt::Display for StartError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StartError::NoFolder(p) => write!(f, "Watch folder does not exist: {}", p.display()),
            StartError::AlreadyRunning => {
                write!(f, "File Sorter is already running and watching (another window or --headless)")
            }
            StartError::Other(e) => write!(f, "{e}"),
        }
    }
}

pub type Sink = Arc<dyn Fn(WatchEvent) + Send + Sync>;

/// True while a browser is still writing `path` under a temporary name.
///
/// Firefox creates an empty placeholder `x.pdf` and downloads into `x.pdf.part`;
/// Chrome and Edge download into `x.pdf.crdownload`.
pub fn has_temp_sibling(path: &Path) -> bool {
    let name = file_name(path);
    let Some(dir) = path.parent() else { return false };
    TEMP_SUFFIXES.iter().any(|suffix| dir.join(format!("{name}{suffix}")).exists())
}

/// Wait until the download is finished and the file can be opened.
///
/// Returns false if the file vanished or never finished within `timeout`.
pub fn wait_until_complete(path: &Path, stable_for: Duration, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    let mut last_size: Option<u64> = None;
    let mut stable_since = Instant::now();
    while Instant::now() < deadline {
        let size = match fs::metadata(path) {
            Ok(meta) => meta.len(),
            // The browser may delete its placeholder just before renaming the temp file over it.
            Err(_) if has_temp_sibling(path) => 0,
            Err(_) => return false,
        };
        let now = Instant::now();
        if last_size != Some(size) || size == 0 || has_temp_sibling(path) {
            // Still growing, an empty placeholder, or the browser is still writing a temp file.
            last_size = Some(size);
            stable_since = now;
        } else if now.duration_since(stable_since) >= stable_for {
            // On Windows this fails while the browser still holds the file open.
            if fs::File::open(path).is_ok() {
                return true;
            }
            stable_since = now;
        }
        thread::sleep(POLL_INTERVAL);
    }
    false
}

type Signature = Option<(u64, SystemTime)>;

fn signature(path: &Path) -> Signature {
    let meta = fs::metadata(path).ok()?;
    Some((meta.len(), meta.modified().ok()?))
}

struct Inner {
    config: RwLock<Config>,
    folder: RwLock<Option<PathBuf>>,
    sink: Sink,
    stable_for: Duration,
    rescan_interval: Duration,
    running: AtomicBool,
    /// Bumped on every start, so a rescan thread from an earlier start stops.
    generation: AtomicU64,
    pending: Mutex<HashSet<String>>,
    /// Files the rescan should skip while unchanged: present at start, or already left in place.
    ignore: Mutex<HashMap<String, Signature>>,
    /// Moves happen one at a time so two files can't race for the same name.
    move_lock: Mutex<()>,
}

/// Watches a folder and sorts each new file once its download has finished.
///
/// Every new file gets a short-lived thread that waits for the download to finish,
/// so one slow download never blocks the others. Besides filesystem events, the
/// folder is rescanned every few seconds in case an event got lost.
pub struct FolderWatcher {
    inner: Arc<Inner>,
    watcher: Option<RecommendedWatcher>,
    /// Lock file that keeps a second File Sorter from watching at the same time.
    lock_path: Option<PathBuf>,
    lock: Option<fs::File>,
}

impl FolderWatcher {
    pub fn new(config: Config, sink: Sink) -> Self {
        Self::with_timing(config, sink, STABLE_FOR, RESCAN_INTERVAL)
    }

    pub fn with_timing(config: Config, sink: Sink, stable_for: Duration, rescan_interval: Duration) -> Self {
        FolderWatcher {
            inner: Arc::new(Inner {
                config: RwLock::new(config),
                folder: RwLock::new(None),
                sink,
                stable_for,
                rescan_interval,
                running: AtomicBool::new(false),
                generation: AtomicU64::new(0),
                pending: Mutex::new(HashSet::new()),
                ignore: Mutex::new(HashMap::new()),
                move_lock: Mutex::new(()),
            }),
            watcher: None,
            lock_path: None,
            lock: None,
        }
    }

    /// Only one watcher per lock file may run at a time (across processes).
    pub fn with_lock(mut self, lock_path: PathBuf) -> Self {
        self.lock_path = Some(lock_path);
        self
    }

    pub fn running(&self) -> bool {
        self.inner.running.load(Ordering::SeqCst)
    }

    /// Use new settings (e.g. edited keywords) for files sorted from now on.
    pub fn update_config(&self, config: Config) {
        *self.inner.config.write().unwrap() = config;
    }

    pub fn start(&mut self) -> Result<(), StartError> {
        if self.running() {
            return Ok(());
        }
        let config = self.inner.config.read().unwrap().clone();
        let folder = config.watch_path();
        if !folder.is_dir() {
            return Err(StartError::NoFolder(folder));
        }
        let folder = folder.canonicalize().map_err(|e| StartError::Other(e.to_string()))?;
        if let Some(lock_path) = &self.lock_path {
            if let Some(dir) = lock_path.parent() {
                let _ = fs::create_dir_all(dir);
            }
            let file = fs::OpenOptions::new()
                .create(true)
                .truncate(false)
                .write(true)
                .open(lock_path)
                .map_err(|e| StartError::Other(e.to_string()))?;
            match file.try_lock() {
                Ok(()) => self.lock = Some(file),
                Err(fs::TryLockError::WouldBlock) => return Err(StartError::AlreadyRunning),
                Err(fs::TryLockError::Error(e)) => return Err(StartError::Other(e.to_string())),
            }
        }
        *self.inner.folder.write().unwrap() = Some(folder.clone());
        if !config.sort_existing_on_start {
            let snapshot = candidates(&folder, &config).into_iter().map(|p| (file_name(&p), signature(&p))).collect();
            *self.inner.ignore.lock().unwrap() = snapshot;
        }

        let inner = Arc::clone(&self.inner);
        let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            if let Ok(event) = res {
                for path in new_file_paths(&event) {
                    enqueue(&inner, path);
                }
            }
        })
        .map_err(|e| StartError::Other(e.to_string()))?;
        watcher.watch(&folder, RecursiveMode::NonRecursive).map_err(|e| StartError::Other(e.to_string()))?;
        self.watcher = Some(watcher);
        self.inner.running.store(true, Ordering::SeqCst);

        let inner = Arc::clone(&self.inner);
        let generation = self.inner.generation.fetch_add(1, Ordering::SeqCst) + 1;
        thread::Builder::new()
            .name("file-sorter-rescan".into())
            .spawn(move || rescan_loop(inner, generation))
            .map_err(|e| StartError::Other(e.to_string()))?;
        if config.sort_existing_on_start {
            self.sort_existing_async();
        }
        Ok(())
    }

    pub fn stop(&mut self) {
        self.inner.running.store(false, Ordering::SeqCst);
        self.watcher = None; // dropping the notify watcher stops the events
        self.lock = None; // closing the file releases the lock
    }

    /// Sort the files already in the watch folder on a background thread.
    pub fn sort_existing_async(&self) {
        let inner = Arc::clone(&self.inner);
        thread::spawn(move || {
            let results = {
                let _guard = inner.move_lock.lock().unwrap();
                let config = inner.config.read().unwrap().clone();
                sort_existing(&config)
            };
            for result in results {
                report(&inner, result);
            }
        });
    }

    /// Queue new files that no filesystem event told us about (the rescan thread does this periodically).
    #[cfg(test)]
    fn rescan(&self) {
        rescan(&self.inner);
    }

    /// Test helper: keep running but stop receiving filesystem events.
    #[cfg(test)]
    fn drop_events(&mut self) {
        self.watcher = None;
    }

    #[cfg(test)]
    fn is_ignored(&self, name: &str) -> bool {
        self.inner.ignore.lock().unwrap().contains_key(name)
    }
}

impl Drop for FolderWatcher {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Paths that may be a newly arrived file: created, or renamed to its final name.
fn new_file_paths(event: &notify::Event) -> Vec<PathBuf> {
    match event.kind {
        EventKind::Create(_) => event.paths.clone(),
        // Browsers rename "x.pdf.part" / "x.pdf.crdownload" to "x.pdf" when done.
        EventKind::Modify(ModifyKind::Name(RenameMode::To)) => event.paths.clone(),
        EventKind::Modify(ModifyKind::Name(RenameMode::Both)) => event.paths.get(1).cloned().into_iter().collect(),
        EventKind::Modify(ModifyKind::Name(RenameMode::Any)) => {
            event.paths.iter().filter(|p| p.exists()).cloned().collect()
        }
        _ => Vec::new(),
    }
}

fn candidates(folder: &Path, config: &Config) -> Vec<PathBuf> {
    fs::read_dir(folder)
        .map(|entries| entries.flatten().map(|e| e.path()).filter(|p| p.is_file() && is_candidate(p, config)).collect())
        .unwrap_or_default()
}

fn rescan(inner: &Arc<Inner>) {
    let Some(folder) = inner.folder.read().unwrap().clone() else { return };
    let config = inner.config.read().unwrap().clone();
    for path in candidates(&folder, &config) {
        let unchanged = inner.ignore.lock().unwrap().get(&file_name(&path)).is_some_and(|sig| *sig == signature(&path));
        if !unchanged {
            enqueue(inner, path);
        }
    }
}

fn rescan_loop(inner: Arc<Inner>, generation: u64) {
    let step = Duration::from_millis(100);
    let mut waited = Duration::ZERO;
    while inner.running.load(Ordering::SeqCst) && inner.generation.load(Ordering::SeqCst) == generation {
        thread::sleep(step);
        waited += step;
        if waited >= inner.rescan_interval {
            waited = Duration::ZERO;
            rescan(&inner);
        }
    }
}

fn enqueue(inner: &Arc<Inner>, path: PathBuf) {
    // Only files sitting directly in the watch folder; ignore temp files early.
    let Some(folder) = inner.folder.read().unwrap().clone() else { return };
    let in_folder = path.parent().and_then(|p| p.canonicalize().ok()).is_some_and(|p| p == folder);
    if !in_folder || !is_candidate(&path, &inner.config.read().unwrap()) {
        return;
    }
    let name = file_name(&path);
    if !inner.pending.lock().unwrap().insert(name.clone()) {
        return;
    }
    let inner = Arc::clone(inner);
    let path = folder.join(&name);
    thread::spawn(move || {
        handle(&inner, &path);
        inner.pending.lock().unwrap().remove(&name);
    });
}

fn handle(inner: &Arc<Inner>, path: &Path) {
    let name = file_name(path);
    // Firefox briefly creates randomly named files that are renamed right away;
    // only announce files that are still around a moment later.
    thread::sleep(DETECT_DELAY.min(inner.stable_for));
    // Put back by "undo": leave it alone without announcing it.
    if crate::sorter::is_kept(path, &inner.config.read().unwrap()) {
        inner.ignore.lock().unwrap().insert(name, signature(path));
        return;
    }
    let announced = path.exists() || has_temp_sibling(path);
    if announced {
        emit(inner, WatchEvent::Detected(name.clone()));
    }
    if !wait_until_complete(path, inner.stable_for, MAX_WAIT) || !inner.running.load(Ordering::SeqCst) {
        if announced && !path.exists() {
            emit(inner, WatchEvent::Vanished(name));
        }
        return;
    }
    let result = {
        let _guard = inner.move_lock.lock().unwrap();
        let config = inner.config.read().unwrap().clone();
        sort_file(path, &config)
    };
    {
        let mut ignore = inner.ignore.lock().unwrap();
        match &result.outcome {
            Outcome::Moved { .. } | Outcome::DuplicateRemoved { .. } => {
                ignore.remove(&name);
            }
            // Copy mode: the original stays; don't copy it again while it is unchanged.
            Outcome::Copied { .. } => {
                ignore.insert(name, signature(path));
            }
            // Still open in another program: leave it out of the ignore list, so the
            // next rescan tries again.
            Outcome::Left(LeftReason::InUse { .. }) => {
                ignore.remove(&name);
            }
            Outcome::Left(_) => {
                ignore.insert(name, signature(path));
            }
        }
    }
    report(inner, result);
}

fn report(inner: &Inner, result: SortResult) {
    emit(inner, WatchEvent::Result(result));
}

fn emit(inner: &Inner, event: WatchEvent) {
    log::info!("{}", event.text());
    (inner.sink)(event);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sorter::tests::{fixture, make_pdf, test_config};
    use std::io::Write;

    struct Harness {
        events: Arc<Mutex<Vec<WatchEvent>>>,
        watcher: FolderWatcher,
        cfg: Config,
        _tmp: tempfile::TempDir,
    }

    impl Harness {
        fn new(configure: impl FnOnce(&mut Config)) -> Self {
            let tmp = tempfile::tempdir().unwrap();
            let mut cfg = test_config(tmp.path());
            configure(&mut cfg);
            let events = Arc::new(Mutex::new(Vec::new()));
            let sink_events = Arc::clone(&events);
            let sink: Sink = Arc::new(move |e| sink_events.lock().unwrap().push(e));
            let watcher =
                FolderWatcher::with_timing(cfg.clone(), sink, Duration::from_millis(300), Duration::from_millis(500));
            Harness { events, watcher, cfg, _tmp: tmp }
        }

        fn downloads(&self) -> PathBuf {
            self.cfg.watch_path()
        }

        fn uni(&self) -> PathBuf {
            PathBuf::from(&self.cfg.destination_root)
        }

        fn messages(&self) -> Vec<String> {
            self.events.lock().unwrap().iter().map(WatchEvent::text).collect()
        }

        fn reports_for(&self, name: &str) -> usize {
            self.events
                .lock()
                .unwrap()
                .iter()
                .filter(|e| matches!(e, WatchEvent::Result(r) if r.file_name() == name))
                .count()
        }
    }

    fn wait_for(mut predicate: impl FnMut() -> bool) -> bool {
        let end = Instant::now() + Duration::from_secs(15);
        while Instant::now() < end {
            if predicate() {
                return true;
            }
            thread::sleep(Duration::from_millis(100));
        }
        false
    }

    /// Mimic Firefox: empty placeholder + growing .part file, then rename over the placeholder.
    fn firefox_download(target: &Path, payload: &[u8], seconds: f32) {
        fs::write(target, b"").unwrap();
        let part = target.with_file_name(format!("{}.part", file_name(target)));
        let mut fh = fs::File::create(&part).unwrap();
        let steps = 10;
        for chunk in payload.chunks(payload.len() / steps + 1) {
            fh.write_all(chunk).unwrap();
            fh.flush().unwrap();
            thread::sleep(Duration::from_secs_f32(seconds / steps as f32));
        }
        drop(fh);
        fs::rename(&part, target).unwrap();
    }

    fn padded_pdf(dir: &Path, text: &str) -> Vec<u8> {
        let src = dir.join("src.pdf");
        make_pdf(&src, text);
        let mut data = fs::read(&src).unwrap();
        fs::remove_file(src).unwrap();
        data.extend_from_slice(b"\n%");
        data.extend(std::iter::repeat_n(b'x', 50_000));
        data
    }

    #[test]
    fn temp_sibling_detection() {
        let tmp = tempfile::tempdir().unwrap();
        let f = tmp.path().join("a.pdf");
        fs::write(&f, "x").unwrap();
        assert!(!has_temp_sibling(&f));
        fs::write(tmp.path().join("a.pdf.part"), "").unwrap();
        assert!(has_temp_sibling(&f));
    }

    #[test]
    fn wait_ignores_empty_placeholder() {
        let tmp = tempfile::tempdir().unwrap();
        let f = tmp.path().join("a.pdf");
        fs::write(&f, "").unwrap();
        assert!(!wait_until_complete(&f, Duration::from_millis(200), Duration::from_millis(1500)));
    }

    #[test]
    fn firefox_download_sorted_once_and_complete() {
        let mut h = Harness::new(|_| {});
        let payload = padded_pdf(h._tmp.path(), "Week 3");
        h.watcher.start().unwrap();
        firefox_download(&h.downloads().join("prog1_vl3.pdf"), &payload, 2.0);

        let folder = h.uni().join("Programming Course");
        assert!(wait_for(|| folder.join("prog1_vl3.pdf").exists()));
        thread::sleep(Duration::from_millis(1500)); // give a wrong duplicate the chance to appear
        let names: Vec<_> = fs::read_dir(&folder).unwrap().flatten().map(|e| file_name(&e.path())).collect();
        assert_eq!(names, vec!["prog1_vl3.pdf"]);
        assert_eq!(fs::read(folder.join("prog1_vl3.pdf")).unwrap(), payload);
        assert_eq!(fs::read_dir(h.downloads()).unwrap().count(), 0);
        assert!(h.messages().iter().any(|m| m.starts_with("Detected prog1_vl3.pdf")));
    }

    #[test]
    fn firefox_download_matched_by_content() {
        let mut h = Harness::new(|_| {});
        h.watcher.start().unwrap();
        let payload = fs::read(fixture("programming.pptx")).unwrap();
        firefox_download(&h.downloads().join("lecture7.pptx"), &payload, 1.0);
        assert!(wait_for(|| h.uni().join("Programming Course").join("lecture7.pptx").exists()));
    }

    #[test]
    fn short_lived_file_not_announced() {
        let mut h = Harness::new(|_| {});
        h.watcher.start().unwrap();
        let tmp = h.downloads().join("Qd_ZJYyR.pdf");
        fs::write(&tmp, "").unwrap();
        thread::sleep(Duration::from_millis(50));
        fs::remove_file(&tmp).unwrap();
        thread::sleep(Duration::from_millis(1000));
        assert!(!h.messages().iter().any(|m| m.contains("Qd_ZJYyR")));
    }

    #[test]
    fn chrome_style_rename_and_unmatched_file() {
        let mut h = Harness::new(|_| {});
        h.watcher.start().unwrap();
        let w = h.downloads();
        make_pdf(&w.join("python_intro.pdf"), "x");
        let partial = w.join("lecture7.pptx.crdownload");
        fs::copy(fixture("programming.pptx"), &partial).unwrap();
        thread::sleep(Duration::from_millis(800));
        assert!(partial.exists(), "partial download must not be touched");
        fs::rename(&partial, w.join("lecture7.pptx")).unwrap();
        make_pdf(&w.join("shopping.pdf"), "milk");

        let prog = h.uni().join("Programming Course");
        assert!(wait_for(|| prog.join("python_intro.pdf").exists()));
        assert!(wait_for(|| prog.join("lecture7.pptx").exists()));
        assert!(wait_for(|| h.reports_for("shopping.pdf") == 1));
        assert!(w.join("shopping.pdf").exists());
    }

    #[test]
    fn rescan_finds_files_without_events() {
        let mut h = Harness::new(|_| {});
        let w = h.downloads();
        make_pdf(&w.join("old_python.pdf"), "x");
        h.watcher.start().unwrap();
        h.watcher.drop_events(); // simulate lost events: only the rescan is left
        make_pdf(&w.join("new_python.pdf"), "x");
        make_pdf(&w.join("unrelated.pdf"), "x");
        h.watcher.rescan();

        assert!(wait_for(|| h.uni().join("Programming Course").join("new_python.pdf").exists()));
        assert!(w.join("old_python.pdf").exists(), "files present at start are left alone");
        assert!(wait_for(|| h.reports_for("unrelated.pdf") == 1));
        assert!(h.watcher.is_ignored("unrelated.pdf"));
        h.watcher.rescan();
        thread::sleep(Duration::from_millis(1500)); // the periodic rescan runs too
        assert_eq!(h.reports_for("unrelated.pdf"), 1, "unmatched file re-reported");
    }

    #[test]
    fn sort_existing_on_start() {
        let mut h = Harness::new(|c| c.sort_existing_on_start = true);
        make_pdf(&h.downloads().join("stats_old.pdf"), "x");
        h.watcher.start().unwrap();
        assert!(wait_for(|| h.uni().join("Statistics").join("stats_old.pdf").exists()));
    }

    #[test]
    fn config_updates_apply_while_running() {
        let mut h = Harness::new(|_| {});
        h.watcher.start().unwrap();
        let mut cfg = h.cfg.clone();
        cfg.courses[2].keywords.push("hausaufgabe".into());
        h.watcher.update_config(cfg);
        make_pdf(&h.downloads().join("hausaufgabe3.pdf"), "x");
        assert!(wait_for(|| h.uni().join("Statistics").join("hausaufgabe3.pdf").exists()));
    }

    #[test]
    fn only_one_watcher_per_lock() {
        let mut first = Harness::new(|_| {});
        let lock = first._tmp.path().join("cfg").join("watch.lock");
        first.watcher = FolderWatcher::new(first.cfg.clone(), Arc::new(|_| {})).with_lock(lock.clone());
        first.watcher.start().unwrap();

        let mut second = FolderWatcher::new(first.cfg.clone(), Arc::new(|_| {})).with_lock(lock.clone());
        assert_eq!(second.start().unwrap_err(), StartError::AlreadyRunning);

        first.watcher.stop();
        second.start().expect("the lock is free again after stopping");
    }

    #[test]
    fn duplicate_download_is_removed_while_watching() {
        let mut h = Harness::new(|_| {});
        let folder = h.uni().join("Programming Course");
        fs::create_dir_all(&folder).unwrap();
        make_pdf(&folder.join("python_vl1.pdf"), "same");
        h.watcher.start().unwrap();
        make_pdf(&h.downloads().join("python_vl1.pdf"), "same");
        assert!(wait_for(|| fs::read_dir(h.downloads()).unwrap().count() == 0), "{:?}", h.messages());
        let names: Vec<_> = fs::read_dir(&folder).unwrap().flatten().map(|e| file_name(&e.path())).collect();
        assert_eq!(names, vec!["python_vl1.pdf"]);
    }

    #[test]
    fn copy_mode_copies_once_and_keeps_the_original() {
        let mut h = Harness::new(|c| c.action = crate::config::FileAction::Copy);
        h.watcher.start().unwrap();
        let original = h.downloads().join("prog1_vl4.pdf");
        make_pdf(&original, "Week 4");
        let copy = h.uni().join("Programming Course").join("prog1_vl4.pdf");
        assert!(wait_for(|| copy.exists()));
        // Several rescans later: still exactly one copy, one report, and the original is untouched.
        thread::sleep(Duration::from_millis(2000));
        assert!(original.exists());
        assert_eq!(fs::read(&original).unwrap(), fs::read(&copy).unwrap());
        assert_eq!(fs::read_dir(copy.parent().unwrap()).unwrap().count(), 1);
        assert_eq!(h.reports_for("prog1_vl4.pdf"), 1, "{:?}", h.messages());
        assert!(h.messages().iter().any(|m| m.starts_with("Copied prog1_vl4.pdf → Programming Course")));
    }

    #[test]
    fn vanished_download_is_reported() {
        let mut h = Harness::new(|_| {});
        h.watcher.start().unwrap();
        // A Firefox download in progress (placeholder + .part) that the user cancels.
        let f = h.downloads().join("python_gone.pdf");
        let part = h.downloads().join("python_gone.pdf.part");
        fs::write(&f, "").unwrap();
        fs::write(&part, "partial").unwrap();
        assert!(wait_for(|| h.messages().iter().any(|m| m.starts_with("Detected python_gone.pdf"))));
        fs::remove_file(&part).unwrap();
        fs::remove_file(&f).unwrap();
        assert!(wait_for(|| h.messages().iter().any(|m| m.contains("python_gone.pdf disappeared"))));
    }

    #[test]
    fn missing_watch_folder_is_an_error() {
        let mut h = Harness::new(|c| c.watch_folder = "/definitely/not/here".into());
        assert!(h.watcher.start().is_err());
        assert!(!h.watcher.running());
    }
}
