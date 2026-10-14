//! Log to the console when there is one, and always to a file next to the settings.
//!
//! The Windows exe has no console, so the file is the only trace of what happened.

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

const MAX_BYTES: u64 = 1_000_000;

struct Logger {
    path: PathBuf,
    file: Mutex<Option<File>>,
}

impl Logger {
    fn open(path: &Path) -> Option<File> {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir).ok()?;
        }
        OpenOptions::new().create(true).append(true).open(path).ok()
    }

    /// Keep one previous log ("file-sorter.log.1") once the current one grows too big.
    fn rotate_if_needed(&self, file: &mut Option<File>) {
        let too_big = file.as_ref().and_then(|f| f.metadata().ok()).is_some_and(|m| m.len() > MAX_BYTES);
        if too_big {
            *file = None;
            let _ = fs::rename(&self.path, self.path.with_extension("log.1"));
            *file = Self::open(&self.path);
        }
    }
}

impl log::Log for Logger {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        // Only our own messages; PDF parsing libraries are very chatty.
        metadata.target().starts_with("file_sorter") && metadata.level() <= log::Level::Info
    }

    fn log(&self, record: &log::Record) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let now = chrono::Local::now();
        let _ = writeln!(std::io::stderr(), "{} {}", now.format("%H:%M:%S"), record.args());
        let mut file = self.file.lock().unwrap();
        self.rotate_if_needed(&mut file);
        if let Some(f) = file.as_mut() {
            let _ = writeln!(f, "{} {}", now.format("%Y-%m-%d %H:%M:%S"), record.args());
        }
    }

    fn flush(&self) {
        if let Some(f) = self.file.lock().unwrap().as_mut() {
            let _ = f.flush();
        }
    }
}

pub fn init(path: &Path) {
    let logger = Logger { path: path.to_path_buf(), file: Mutex::new(Logger::open(path)) };
    if log::set_boxed_logger(Box::new(logger)).is_ok() {
        log::set_max_level(log::LevelFilter::Info);
    }
}
