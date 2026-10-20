//! Search the text of every document in the study folder.
//!
//! A background thread keeps an index of the text of each PDF page, slide and document,
//! saved in `search-index.json` so it doesn't have to read everything again at the next
//! start: only new and changed files are read. Files that are only in the cloud
//! (OneDrive "online only") are found by name, but not read, so they aren't downloaded.

use std::collections::HashMap;
use std::fs;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::sync::{Arc, RwLock};
use std::thread;
use std::time::{Duration, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use unicode_normalization::UnicodeNormalization;

pub const INDEX_FILE: &str = "search-index.json";
/// Files with text worth reading; other files are found by their name only.
const TEXT_TYPES: &[&str] = &["pdf", "pptx", "docx", "txt", "md"];
const MAX_DEPTH: usize = 8;
/// The index is brought up to date this often while the window is open.
const REFRESH: Duration = Duration::from_secs(10 * 60);
/// How much text is shown around a hit.
const SNIPPET_BEFORE: usize = 60;
const SNIPPET_AFTER: usize = 140;

/// One file in the index.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Doc {
    pub path: PathBuf,
    size: u64,
    modified: u64,
    /// Text per page or slide (empty for files that are only found by name).
    pages: Vec<String>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Stored {
    docs: Vec<Doc>,
}

/// The index in memory, with the text folded for searching (lower-case, no accents; one
/// character for each character of the original, so positions carry over).
#[derive(Debug, Default)]
pub struct Index {
    docs: Vec<Doc>,
    folded_names: Vec<String>,
    folded_pages: Vec<Vec<String>>,
}

/// One search result.
#[derive(Debug, Clone, PartialEq)]
pub struct Hit {
    pub path: PathBuf,
    /// 1-based page or slide the words were found on; `None` when only the name matched.
    pub page: Option<usize>,
    /// Text around the first hit, and where the search words are in it (byte ranges).
    pub snippet: String,
    pub marks: Vec<Range<usize>>,
    score: usize,
}

/// One character for each character: lower-case, without accents, white space as ' '.
fn fold_char(c: char) -> char {
    if c.is_whitespace() {
        return ' ';
    }
    let lower = c.to_lowercase().next().unwrap_or(c);
    if lower.is_ascii() {
        return lower;
    }
    lower.nfd().next().unwrap_or(lower)
}

pub fn fold(text: &str) -> String {
    text.chars().map(fold_char).collect()
}

fn stamp(meta: &fs::Metadata) -> u64 {
    meta.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map_or(0, |d| d.as_nanos() as u64)
}

impl Index {
    fn from_docs(docs: Vec<Doc>) -> Index {
        let folded_names =
            docs.iter().map(|d| fold(&d.path.file_name().unwrap_or_default().to_string_lossy())).collect();
        let folded_pages = docs.iter().map(|d| d.pages.iter().map(|p| fold(p)).collect()).collect();
        Index { docs, folded_names, folded_pages }
    }

    pub fn load(file: &Path) -> Index {
        let stored: Stored =
            fs::read_to_string(file).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
        Index::from_docs(stored.docs)
    }

    pub fn len(&self) -> usize {
        self.docs.len()
    }

    /// Documents with text changed after `since` (nanoseconds since 1970) that `skip` doesn't
    /// rule out, at most `limit`: path, size, modification time (seconds) and the text of
    /// their first `pages` pages.
    pub fn texts(
        &self,
        since: u64,
        pages: usize,
        limit: usize,
        skip: impl Fn(&Path, u64) -> bool,
    ) -> Vec<(PathBuf, u64, i64, String)> {
        self.docs
            .iter()
            .filter(|d| !d.pages.is_empty() && d.modified >= since && !skip(&d.path, d.size))
            .take(limit)
            .map(|d| {
                (
                    d.path.clone(),
                    d.size,
                    (d.modified / 1_000_000_000) as i64,
                    d.pages[..d.pages.len().min(pages)].join("\n"),
                )
            })
            .collect()
    }

    /// Files containing every word of `query` (in the name or text), best first.
    pub fn search(&self, query: &str, limit: usize) -> Vec<Hit> {
        let words: Vec<String> = fold(query).split_whitespace().map(str::to_string).collect();
        if words.is_empty() {
            return Vec::new();
        }
        let mut hits = Vec::new();
        for (i, doc) in self.docs.iter().enumerate() {
            let name = &self.folded_names[i];
            let pages = &self.folded_pages[i];
            let found = |w: &String| name.contains(w.as_str()) || pages.iter().any(|p| p.contains(w.as_str()));
            if !words.iter().all(found) {
                continue;
            }
            let count = |text: &str| words.iter().map(|w| text.matches(w.as_str()).count()).sum::<usize>();
            let in_name = count(name);
            // The page with the most hits, preferring one that has all words.
            let best = pages
                .iter()
                .enumerate()
                .map(|(n, p)| (words.iter().filter(|w| p.contains(w.as_str())).count(), count(p), n))
                .filter(|(present, _, _)| *present > 0)
                .max_by_key(|(present, hits, n)| (*present, *hits, std::cmp::Reverse(*n)));
            let total: usize = pages.iter().map(|p| count(p)).sum();
            let (page, snippet, marks) = match best {
                Some((_, _, n)) => {
                    let (snippet, marks) = snippet(&doc.pages[n], &pages[n], &words);
                    (Some(n + 1), snippet, marks)
                }
                None => (None, String::new(), Vec::new()),
            };
            hits.push(Hit { path: doc.path.clone(), page, snippet, marks, score: in_name * 20 + total.min(200) });
        }
        hits.sort_by(|a, b| b.score.cmp(&a.score).then_with(|| a.path.cmp(&b.path)));
        hits.truncate(limit);
        hits
    }
}

/// Text around the first hit on a page, white space tidied, with the search words marked.
fn snippet(raw: &str, folded: &str, words: &[String]) -> (String, Vec<Range<usize>>) {
    let first = words.iter().filter_map(|w| folded.find(w.as_str())).min().unwrap_or(0);
    let at = folded[..first].chars().count();
    let start = at.saturating_sub(SNIPPET_BEFORE);
    let mut text = String::new();
    let mut folded_text = String::new();
    let mut space = false;
    for (r, f) in raw.chars().zip(folded.chars()).skip(start).take(SNIPPET_BEFORE + SNIPPET_AFTER) {
        if f == ' ' || r.is_control() {
            if !space && !text.is_empty() {
                text.push(' ');
                folded_text.push(' ');
            }
            space = true;
            continue;
        }
        space = false;
        text.push(r);
        folded_text.push(f);
    }
    if start > 0 {
        text.insert(0, '…');
        folded_text.insert(0, '…');
    }
    if raw.chars().count() > start + SNIPPET_BEFORE + SNIPPET_AFTER {
        text.push('…');
        folded_text.push('…');
    }
    // Mark each word: char positions in the folded text are the same in the original.
    let byte_at = |chars: usize| text.char_indices().nth(chars).map_or(text.len(), |(b, _)| b);
    let mut marks = Vec::new();
    for word in words {
        for (b, _) in folded_text.match_indices(word.as_str()) {
            let from = folded_text[..b].chars().count();
            let to = from + word.chars().count();
            marks.push(byte_at(from)..byte_at(to));
        }
    }
    marks.sort_by_key(|r| r.start);
    // Overlapping marks ("rek" and "rekursion") become one.
    let mut merged: Vec<Range<usize>> = Vec::new();
    for mark in marks {
        match merged.last_mut() {
            Some(last) if mark.start <= last.end => last.end = last.end.max(mark.end),
            _ => merged.push(mark),
        }
    }
    (text, merged)
}

fn is_text_type(path: &Path) -> bool {
    let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    TEXT_TYPES.contains(&ext.as_str())
}

/// Every file in `root` worth finding: documents and the other types File Sorter sorts.
fn files_in(root: &Path, extensions: &[String]) -> Vec<(PathBuf, fs::Metadata)> {
    fn walk(dir: &Path, depth: usize, extensions: &[String], out: &mut Vec<(PathBuf, fs::Metadata)>) {
        let Ok(entries) = fs::read_dir(dir) else { return };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') || crate::cloud::is_junk(&name) {
                continue;
            }
            let Ok(meta) = fs::symlink_metadata(&path) else { continue };
            if meta.is_dir() {
                if depth < MAX_DEPTH {
                    walk(&path, depth + 1, extensions, out);
                }
            } else if meta.is_file() {
                let ext = path.extension().map(|e| format!(".{}", e.to_string_lossy().to_lowercase()));
                if is_text_type(&path) || ext.is_some_and(|e| extensions.contains(&e)) {
                    out.push((path, meta));
                }
            }
        }
    }
    let mut out = Vec::new();
    walk(root, 0, extensions, &mut out);
    out
}

/// Bring `old` up to date with the files in `root`: unchanged files keep their text, new
/// and changed ones are read. `progress` is told after every file read.
pub fn refresh(root: &Path, extensions: &[String], old: &[Doc], mut progress: impl FnMut(&[Doc])) -> Vec<Doc> {
    let known: HashMap<&Path, &Doc> = old.iter().map(|d| (d.path.as_path(), d)).collect();
    let mut docs = Vec::new();
    for (path, meta) in files_in(root, extensions) {
        let (size, modified) = (meta.len(), stamp(&meta));
        if let Some(doc) = known.get(path.as_path()).filter(|d| d.size == size && d.modified == modified) {
            docs.push((*doc).clone());
            continue;
        }
        // Only in the cloud: found by name, read once it is on this computer.
        let pages = if is_text_type(&path) && !crate::cloud::is_placeholder(&path) {
            crate::extract::pages(&path)
        } else {
            Vec::new()
        };
        let modified = if pages.is_empty() && crate::cloud::is_placeholder(&path) { 0 } else { modified };
        docs.push(Doc { path, size, modified, pages });
        progress(&docs);
    }
    docs.sort_by(|a, b| a.path.cmp(&b.path));
    docs
}

/// What the indexing thread is asked to do.
enum Job {
    /// Index this folder (the study folder, after it moved or at the start).
    Root(PathBuf, Vec<String>),
    /// Something was sorted: look for new files now.
    Refresh,
}

/// The search index, kept up to date by a background thread.
pub struct Indexer {
    pub index: Arc<RwLock<Index>>,
    busy: Arc<std::sync::atomic::AtomicBool>,
    tx: Sender<Job>,
}

impl Indexer {
    /// Load the saved index from `state_dir` and start the thread (it waits for `set_root`).
    pub fn start(state_dir: Option<PathBuf>, repaint: impl Fn() + Send + 'static) -> Indexer {
        let file = state_dir.map(|d| d.join(INDEX_FILE));
        let index = Arc::new(RwLock::new(file.as_deref().map(Index::load).unwrap_or_default()));
        let busy = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (tx, rx) = channel();
        let (shared, flag) = (Arc::clone(&index), Arc::clone(&busy));
        let _ =
            thread::Builder::new().name("file-sorter-index".into()).spawn(move || run(rx, shared, flag, file, repaint));
        Indexer { index, busy, tx }
    }

    pub fn set_root(&self, root: PathBuf, extensions: Vec<String>) {
        let _ = self.tx.send(Job::Root(root, extensions));
    }

    pub fn refresh(&self) {
        let _ = self.tx.send(Job::Refresh);
    }

    /// True while files are being read.
    pub fn busy(&self) -> bool {
        self.busy.load(std::sync::atomic::Ordering::Relaxed)
    }
}

fn run(
    rx: Receiver<Job>,
    index: Arc<RwLock<Index>>,
    busy: Arc<std::sync::atomic::AtomicBool>,
    file: Option<PathBuf>,
    repaint: impl Fn(),
) {
    let mut root: Option<(PathBuf, Vec<String>)> = None;
    loop {
        match rx.recv_timeout(REFRESH) {
            Ok(Job::Root(new, extensions)) => root = Some((new, extensions)),
            Ok(Job::Refresh) | Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
        // Several requests in a row need only one pass.
        while let Ok(job) = rx.try_recv() {
            if let Job::Root(new, extensions) = job {
                root = Some((new, extensions));
            }
        }
        let Some((folder, extensions)) = &root else { continue };
        if !folder.is_dir() {
            continue;
        }
        busy.store(true, std::sync::atomic::Ordering::Relaxed);
        let old = index.read().unwrap().docs.clone();
        let mut since_shown = 0;
        let docs = refresh(folder, extensions, &old, |partial| {
            // Show what is there so far now and then, so the first results don't wait for all.
            since_shown += 1;
            if since_shown >= 20 {
                since_shown = 0;
                let mut merged: Vec<Doc> =
                    old.iter().filter(|d| !partial.iter().any(|p| p.path == d.path)).cloned().collect();
                merged.extend(partial.iter().cloned());
                *index.write().unwrap() = Index::from_docs(merged);
                repaint();
            }
        });
        let changed = docs != old;
        *index.write().unwrap() = Index::from_docs(docs.clone());
        busy.store(false, std::sync::atomic::Ordering::Relaxed);
        repaint();
        if changed
            && let Some(file) = &file
            && let Err(e) = crate::cloud::save_json(&Stored { docs }, file)
        {
            log::info!("Could not save the search index: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sorter::tests::make_pdf;

    #[test]
    fn folding_keeps_positions() {
        assert_eq!(fold("Übung ÄÖÜ é\tX"), "ubung aou e x");
        let text = "Größe ﬁ ß";
        assert_eq!(fold(text).chars().count(), text.chars().count());
    }

    #[test]
    fn finds_words_on_pages_with_snippets() {
        let docs = vec![
            Doc {
                path: PathBuf::from("/s/Prog/VL3.pdf"),
                size: 1,
                modified: 1,
                pages: vec!["Einleitung".into(), "Die Rekursion: eine Funktion ruft sich selbst auf.".into()],
            },
            Doc { path: PathBuf::from("/s/Prog/Rekursion Übung.pdf"), size: 1, modified: 1, pages: vec![] },
            Doc { path: PathBuf::from("/s/Stats/x.pdf"), size: 1, modified: 1, pages: vec!["Mittelwert".into()] },
        ];
        let index = Index::from_docs(docs);
        let hits = index.search("REKURSION", 10);
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].path, PathBuf::from("/s/Prog/Rekursion Übung.pdf"), "a hit in the name counts most");
        assert_eq!(hits[1].page, Some(2));
        assert_eq!(&hits[1].snippet[hits[1].marks[0].clone()], "Rekursion");
        assert_eq!(index.search("rekursion funktion", 10).len(), 1, "all words must occur");
        assert_eq!(index.search("ubung", 10).len(), 1, "accents don't matter");
        assert!(index.search("  ", 10).is_empty());
        assert!(index.search("nirgends", 10).is_empty());
    }

    #[test]
    fn snippets_mark_words_after_accented_letters() {
        let raw = "Äpfel und Birnen\n\nsind Obst";
        let (text, marks) = snippet(raw, &fold(raw), &["obst".to_string(), "apfel".to_string()]);
        assert_eq!(text, "Äpfel und Birnen sind Obst");
        assert_eq!(marks.iter().map(|m| &text[m.clone()]).collect::<Vec<_>>(), ["Äpfel", "Obst"]);
    }

    #[test]
    fn refresh_reads_only_new_and_changed_files() {
        let tmp = tempfile::tempdir().unwrap();
        let module = tmp.path().join("Prog");
        fs::create_dir_all(&module).unwrap();
        make_pdf(&module.join("vl1.pdf"), "Rekursion und Iteration");
        fs::write(module.join("notes.md"), "Hausaufgabe bis Freitag").unwrap();
        fs::write(module.join("code.py"), "print()").unwrap();
        let extensions = vec![".pdf".to_string()];
        let mut read = 0;
        let docs = refresh(tmp.path(), &extensions, &[], |_| read += 1);
        assert_eq!(read, 2, "code.py isn't a type File Sorter sorts here");
        let index = Index::from_docs(docs.clone());
        assert_eq!(index.search("rekursion", 5)[0].page, Some(1));
        assert_eq!(index.search("freitag", 5).len(), 1);
        let mut read = 0;
        let again = refresh(tmp.path(), &extensions, &docs, |_| read += 1);
        assert_eq!((read, again.len()), (0, 2), "nothing changed");
        fs::remove_file(module.join("notes.md")).unwrap();
        assert_eq!(refresh(tmp.path(), &extensions, &again, |_| {}).len(), 1, "gone files leave the index");
    }
}
