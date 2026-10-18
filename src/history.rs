//! The history of sorted files, kept across restarts so a sort can be undone later.
//!
//! One JSON object per line, only ever appended to (several File Sorter processes may
//! write at the same time: the window and right-click sorting). Undoing a sort adds an
//! "undone" line for it; moving the study folder (into the cloud, or back) adds a
//! "relocated" line, so earlier entries can still be undone from the new place.

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
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
    Sorted {
        id: u64,
        time: i64,
        result: SortResult,
    },
    Undone {
        id: u64,
    },
    /// The study folder moved from `from` to `to`; `renamed` files got another name there.
    Relocated {
        from: PathBuf,
        to: PathBuf,
        renamed: Vec<(PathBuf, PathBuf)>,
    },
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

/// Note that the study folder moved from `from` to `to`.
pub fn record_relocation(dir: &Path, from: &Path, to: &Path, renamed: &[(PathBuf, PathBuf)]) -> io::Result<()> {
    append(dir, &Line::Relocated { from: from.to_path_buf(), to: to.to_path_buf(), renamed: renamed.to_vec() })
}

/// Where a file sorted to `path` is after the study folder moved from `from` to `to`.
/// Files that were not moved (still open at the time) are still where they were.
pub fn relocated(path: &Path, from: &Path, to: &Path, renamed: &[(PathBuf, PathBuf)]) -> Option<PathBuf> {
    if let Some((_, new)) = renamed.iter().find(|(old, _)| old == path) {
        return Some(new.clone());
    }
    let moved = to.join(path.strip_prefix(from).ok()?);
    (!path.exists() && moved.exists()).then_some(moved)
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
            Ok(Line::Relocated { from, to, renamed }) => {
                for entry in &mut entries {
                    if let Some(path) = entry.result.sorted_path_mut()
                        && let Some(new) = relocated(path, &from, &to, &renamed)
                    {
                        *path = new;
                    }
                }
            }
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
    fn entries_follow_the_study_folder_when_it_moves() {
        let tmp = tempfile::tempdir().unwrap();
        let (old, new) = (tmp.path().join("old"), tmp.path().join("OneDrive").join("Studium"));
        let entry = |name: &str| {
            let mut result = moved(name);
            if let Outcome::Moved { destination, .. } = &mut result.outcome {
                *destination = old.join("Prog").join(name);
            }
            result
        };
        let state = tmp.path().join("state");
        for name in ["a.pdf", "b.pdf", "open.pdf"] {
            record(&state, &entry(name)).unwrap();
        }
        // a.pdf moved as is, b.pdf had to be renamed, open.pdf was still open and stayed.
        fs::create_dir_all(new.join("Prog")).unwrap();
        fs::create_dir_all(old.join("Prog")).unwrap();
        fs::write(new.join("Prog").join("a.pdf"), "a").unwrap();
        fs::write(new.join("Prog").join("b (1).pdf"), "b").unwrap();
        fs::write(old.join("Prog").join("open.pdf"), "o").unwrap();
        let renamed = vec![(old.join("Prog").join("b.pdf"), new.join("Prog").join("b (1).pdf"))];
        record_relocation(&state, &old, &new, &renamed).unwrap();
        record(&state, &moved("later.pdf")).unwrap();

        let entries = recent(&state, 10);
        let destination = |i: usize| match &entries[i].result.outcome {
            Outcome::Moved { destination, .. } => destination.clone(),
            other => panic!("{other:?}"),
        };
        assert_eq!(destination(0), new.join("Prog").join("a.pdf"));
        assert_eq!(destination(1), new.join("Prog").join("b (1).pdf"));
        assert_eq!(destination(2), old.join("Prog").join("open.pdf"));
        assert_eq!(destination(3), PathBuf::from("/uni/later.pdf"), "later entries are not touched");
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
