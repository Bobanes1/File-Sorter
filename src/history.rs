//! The history of sorted files, kept across restarts so a sort can be undone later.
//!
//! One JSON object per line, only ever appended to (several File Sorter processes may
//! write at the same time: the window and right-click sorting). Undoing a sort adds an
//! "undone" line for it.

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use chrono::{DateTime, Local, TimeZone};
use serde::{Deserialize, Serialize};

use crate::sorter::SortResult;

pub const HISTORY_FILE: &str = "history.jsonl";
/// Once the file is bigger than this, only the newest lines are kept.
const TRIM_ABOVE: u64 = 1_000_000;
const KEEP_AFTER_TRIM: usize = 300;

#[derive(Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
enum Line {
    Sorted { id: u64, time: i64, result: SortResult },
    Undone { id: u64 },
}

/// One sorted file from the history.
#[derive(Debug, Clone)]
pub struct Entry {
    pub time: DateTime<Local>,
    /// With `id` set, so it can be undone.
    pub result: SortResult,
    pub undone: bool,
}

/// A new id: microseconds since 1970, made unique within this process.
fn new_id() -> u64 {
    static LAST: AtomicU64 = AtomicU64::new(0);
    let now = Local::now().timestamp_micros().max(0) as u64;
    let mut last = LAST.load(Ordering::SeqCst);
    loop {
        let id = now.max(last + 1);
        match LAST.compare_exchange(last, id, Ordering::SeqCst, Ordering::SeqCst) {
            Ok(_) => return id,
            Err(current) => last = current,
        }
    }
}

fn append(dir: &Path, line: &Line) -> io::Result<()> {
    fs::create_dir_all(dir)?;
    let file = dir.join(HISTORY_FILE);
    let mut text = serde_json::to_string(line).map_err(io::Error::other)?;
    text.push('\n');
    let mut out = OpenOptions::new().create(true).append(true).open(&file)?;
    // One write per line, so lines from different processes don't get mixed up.
    out.write_all(text.as_bytes())?;
    drop(out);
    if fs::metadata(&file).map(|m| m.len() > TRIM_ABOVE).unwrap_or(false) {
        trim(&file)?;
    }
    Ok(())
}

/// Keep the newest lines: at most `KEEP_AFTER_TRIM`, and at most half of `TRIM_ABOVE` bytes.
fn trim(file: &Path) -> io::Result<()> {
    let text = fs::read_to_string(file)?;
    let mut keep = Vec::new();
    let mut size = 0;
    for line in text.lines().rev().take(KEEP_AFTER_TRIM) {
        size += line.len() + 1;
        if size as u64 > TRIM_ABOVE / 2 && !keep.is_empty() {
            break;
        }
        keep.push(line);
    }
    keep.reverse();
    let tmp = file.with_extension("tmp");
    fs::write(&tmp, keep.join("\n") + "\n")?;
    fs::rename(&tmp, file)
}

/// Add a sorted file to the history; returns its id.
pub fn record(dir: &Path, result: &SortResult) -> io::Result<u64> {
    let id = new_id();
    append(dir, &Line::Sorted { id, time: Local::now().timestamp(), result: result.clone() })?;
    Ok(id)
}

/// Mark a sorted file as undone.
pub fn record_undo(dir: &Path, id: u64) -> io::Result<()> {
    append(dir, &Line::Undone { id })
}

/// The newest `limit` sorted files, oldest first.
pub fn recent(dir: &Path, limit: usize) -> Vec<Entry> {
    let Ok(text) = fs::read_to_string(dir.join(HISTORY_FILE)) else { return Vec::new() };
    let mut entries: Vec<Entry> = Vec::new();
    let mut undone = Vec::new();
    for line in text.lines() {
        match serde_json::from_str::<Line>(line) {
            Ok(Line::Sorted { id, time, mut result }) => {
                result.id = Some(id);
                let time = Local.timestamp_opt(time, 0).single().unwrap_or_else(Local::now);
                entries.push(Entry { time, result, undone: false });
            }
            Ok(Line::Undone { id }) => undone.push(id),
            Err(_) => {} // a line from a newer or older version, or cut off
        }
    }
    for entry in &mut entries {
        entry.undone = entry.result.id.is_some_and(|id| undone.contains(&id));
    }
    let skip = entries.len().saturating_sub(limit);
    entries.split_off(skip)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::matcher::Reason;
    use crate::sorter::Outcome;
    use std::path::PathBuf;

    fn moved(name: &str) -> SortResult {
        SortResult::new(
            PathBuf::from(format!("/dl/{name}")),
            Outcome::Moved {
                course: "Informatik › Programmierung 1".into(),
                destination: PathBuf::from(format!("/uni/{name}")),
                reason: Reason::FileName(vec!["prog1".into()]),
            },
        )
    }

    #[test]
    fn records_reads_back_and_marks_undone() {
        let tmp = tempfile::tempdir().unwrap();
        let a = record(tmp.path(), &moved("a.pdf")).unwrap();
        let b = record(tmp.path(), &moved("b.pdf")).unwrap();
        assert_ne!(a, b);
        record_undo(tmp.path(), a).unwrap();
        fs::write(tmp.path().join("junk"), "").unwrap();
        // A damaged line is skipped.
        OpenOptions::new().append(true).open(tmp.path().join(HISTORY_FILE)).unwrap().write_all(b"{oops\n").unwrap();

        let entries = recent(tmp.path(), 10);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].result.file_name(), "a.pdf");
        assert_eq!(entries[0].result.id, Some(a));
        assert!(entries[0].undone && !entries[1].undone);
        assert_eq!(entries[1].result.outcome, moved("b.pdf").outcome);
        assert_eq!(recent(tmp.path(), 1).len(), 1);
        assert_eq!(recent(tmp.path(), 1)[0].result.file_name(), "b.pdf");
    }

    #[test]
    fn big_history_is_trimmed() {
        let tmp = tempfile::tempdir().unwrap();
        let long = "x".repeat(4000);
        for i in 0..300 {
            record(tmp.path(), &moved(&format!("{long}{i}.pdf"))).unwrap();
        }
        assert!(fs::metadata(tmp.path().join(HISTORY_FILE)).unwrap().len() <= TRIM_ABOVE);
        let entries = recent(tmp.path(), 1000);
        assert!(entries.len() <= KEEP_AFTER_TRIM && !entries.is_empty());
        assert_eq!(entries.last().unwrap().result.file_name(), format!("{long}299.pdf"));
    }
}
