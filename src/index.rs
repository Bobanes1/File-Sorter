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
/// Changes in the study folder are seen as they happen; besides, the index is brought up to
/// date this often (for changes the system didn't report, e.g. on network drives).
const REFRESH: Duration = Duration::from_secs(60 * 60);
/// After a change, wait this long for more before reading (a copy of many files is one pass).
const SETTLE: Duration = Duration::from_secs(3);
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
    /// Other pages with hits.
    pub more_pages: Vec<usize>,
    score: usize,
    modified: u64,
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

    /// The first `words` words of every document with text, by path.
    pub fn starts(&self, words: usize) -> std::collections::HashMap<PathBuf, String> {
        self.docs
            .iter()
            .filter_map(|d| {
                let first = d.pages.first()?;
                Some((d.path.clone(), first.split_whitespace().take(words).collect::<Vec<_>>().join(" ")))
            })
            .collect()
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
    #[cfg(test)]
    pub fn search(&self, query: &str, limit: usize) -> Vec<Hit> {
        self.search_with(&Query::parse(query), &Filter::default(), limit)
    }

    /// Files that have every term of the query (word beginnings, phrases; with one typo
    /// allowed for longer words when nothing is found otherwise), none of the excluded words,
    /// and pass the filter – ranked by BM25 with the name and the title page counting more.
    pub fn search_with(&self, query: &Query, filter: &Filter, limit: usize) -> Vec<Hit> {
        if query.terms.is_empty() && query.phrases.is_empty() {
            return Vec::new();
        }
        let candidates: Vec<usize> = (0..self.docs.len()).filter(|&i| filter.allows(&self.docs[i])).collect();
        let mut hits = self.rank(query, &candidates, false, limit);
        // Nothing (or very little) found: allow one typo in longer words.
        if hits.len() < 3 && query.terms.iter().any(|t| t.chars().count() >= TYPO_LENGTH) {
            let fuzzy = self.rank(query, &candidates, true, limit);
            for hit in fuzzy {
                if !hits.iter().any(|h| h.path == hit.path) {
                    hits.push(hit);
                }
            }
        }
        hits.truncate(limit);
        hits
    }

    fn rank(&self, query: &Query, candidates: &[usize], fuzzy: bool, limit: usize) -> Vec<Hit> {
        // The words each term stands for in a document (itself, or a near word when fuzzy).
        let mut found: Vec<(usize, Vec<Vec<String>>)> = Vec::new();
        'docs: for &i in candidates {
            let name = &self.folded_names[i];
            let pages = &self.folded_pages[i];
            for excluded in &query.excluded {
                if count_starts(name, excluded) > 0 || pages.iter().any(|p| count_starts(p, excluded) > 0) {
                    continue 'docs;
                }
            }
            let mut forms = Vec::new();
            for term in &query.terms {
                let mut words = vec![term.clone()];
                let present = |w: &String| count_starts(name, w) > 0 || pages.iter().any(|p| count_starts(p, w) > 0);
                if !present(term) {
                    if !fuzzy || term.chars().count() < TYPO_LENGTH {
                        continue 'docs;
                    }
                    words = near_words(name, pages, term);
                    if words.is_empty() {
                        continue 'docs;
                    }
                }
                forms.push(words);
            }
            for phrase in &query.phrases {
                if !squeezed(name).contains(phrase.as_str())
                    && !pages.iter().any(|p| squeezed(p).contains(phrase.as_str()))
                {
                    continue 'docs;
                }
                forms.push(vec![phrase.clone()]);
            }
            found.push((i, forms));
        }
        if found.is_empty() {
            return Vec::new();
        }
        // BM25 over all documents: how rare each term is, and how long each document is.
        let n = self.docs.len().max(1) as f64;
        let length = |i: usize| self.folded_pages[i].iter().map(|p| p.len()).sum::<usize>() as f64 / 6.0 + 1.0;
        let average = (0..self.docs.len()).map(length).sum::<f64>() / n;
        let all_terms: Vec<&String> = query.terms.iter().chain(&query.phrases).collect();
        // In how many documents of the whole index each term is.
        let with_term: Vec<f64> = all_terms
            .iter()
            .map(|t| {
                (0..self.docs.len())
                    .filter(|&i| {
                        count_starts(&self.folded_names[i], t) > 0
                            || self.folded_pages[i].iter().any(|p| count_starts(p, t) > 0)
                    })
                    .count() as f64
            })
            .collect();
        let mut hits = Vec::new();
        for (i, forms) in found {
            let doc = &self.docs[i];
            let name = &self.folded_names[i];
            let pages = &self.folded_pages[i];
            let mut score = 0.0;
            for (k, words) in forms.iter().enumerate() {
                let idf = ((n - with_term[k] + 0.5) / (with_term[k] + 0.5) + 1.0).ln();
                let count = |text: &str| words.iter().map(|w| count_starts(text, w)).sum::<usize>() as f64;
                let title: f64 = pages.first().map_or(0.0, |p| count(&p[..p.len().min(TITLE_CHARS)]));
                let tf = count(name) * NAME_WEIGHT + title + pages.iter().map(|p| count(p)).sum::<f64>();
                score += idf * tf * (BM25_K1 + 1.0) / (tf + BM25_K1 * (1.0 - BM25_B + BM25_B * length(i) / average));
            }
            if fuzzy {
                score *= 0.5;
            }
            let marks: Vec<String> = forms.iter().flatten().cloned().collect();
            // The pages with the most hits, the best first.
            let mut on_pages: Vec<(usize, usize)> = pages
                .iter()
                .enumerate()
                .map(|(p, text)| (marks.iter().filter(|w| text.contains(w.as_str())).count(), p))
                .filter(|(present, _)| *present > 0)
                .collect();
            on_pages.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
            let (page, snippet, marked) = match on_pages.first() {
                Some(&(_, p)) => {
                    let (snippet, marked) = snippet(&doc.pages[p], &pages[p], &marks);
                    (Some(p + 1), snippet, marked)
                }
                None => (None, String::new(), Vec::new()),
            };
            let more_pages = on_pages.iter().skip(1).take(5).map(|(_, p)| p + 1).collect();
            hits.push(Hit {
                path: doc.path.clone(),
                page,
                snippet,
                marks: marked,
                more_pages,
                score: (score * 1000.0) as usize,
                modified: doc.modified,
            });
        }
        // Best first; equally good: the newer file first.
        hits.sort_by(|a, b| b.score.cmp(&a.score).then(b.modified.cmp(&a.modified)).then_with(|| a.path.cmp(&b.path)));
        hits.truncate(limit);
        hits
    }
}

/// Longer words may have one typo (only when nothing is found without).
const TYPO_LENGTH: usize = 5;
/// A hit in the file name counts this many times a hit in the text.
const NAME_WEIGHT: f64 = 3.0;
/// The start of the first page (the title page) counts twice.
const TITLE_CHARS: usize = 400;
const BM25_K1: f64 = 1.2;
const BM25_B: f64 = 0.75;

/// How often a word begins with `term` in folded text ("rekurs" in "Rekursion").
fn count_starts(text: &str, term: &str) -> usize {
    if term.is_empty() {
        return 0;
    }
    text.match_indices(term)
        .filter(|(b, _)| *b == 0 || !text[..*b].chars().next_back().is_some_and(char::is_alphanumeric))
        .count()
}

/// Folded text with runs of spaces made one, for phrases across line breaks.
fn squeezed(text: &str) -> String {
    text.split(' ').filter(|w| !w.is_empty()).collect::<Vec<_>>().join(" ")
}

/// Words in a document one typo away from `term` (or whose beginning is).
fn near_words(name: &str, pages: &[String], term: &str) -> Vec<String> {
    let length = term.chars().count();
    let mut near: Vec<String> = Vec::new();
    for text in std::iter::once(name).chain(pages.iter().map(String::as_str)) {
        for word in text.split(|c: char| !c.is_alphanumeric()) {
            if word.chars().count() < length - 1 || near.iter().any(|n| n == word) {
                continue;
            }
            let start: String = word.chars().take(length).collect();
            let shorter: String = word.chars().take(length - 1).collect();
            let longer: String = word.chars().take(length + 1).collect();
            if [start, shorter, longer].iter().any(|w| crate::matcher::one_typo(w, term)) {
                near.push(word.to_string());
            }
        }
    }
    near
}

/// What was typed into the search field: words (matched at word beginnings), phrases in
/// quotes and words to leave out (`-wort`).
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Query {
    pub terms: Vec<String>,
    pub phrases: Vec<String>,
    pub excluded: Vec<String>,
}

impl Query {
    pub fn parse(text: &str) -> Query {
        let folded = fold(text).replace(['„', '“', '”', '«', '»'], "\"");
        let mut query = Query::default();
        let mut parts = folded.split('"');
        let mut quoted = false;
        for part in parts.by_ref() {
            if quoted {
                let phrase = squeezed(part.trim());
                if !phrase.is_empty() {
                    query.phrases.push(phrase);
                }
            } else {
                for word in part.split_whitespace() {
                    match word.strip_prefix('-') {
                        Some(rest) if !rest.is_empty() => query.excluded.push(rest.to_string()),
                        _ => {
                            let word = word.trim_matches(|c: char| !c.is_alphanumeric());
                            if !word.is_empty() {
                                query.terms.push(word.to_string());
                            }
                        }
                    }
                }
            }
            quoted = !quoted;
        }
        query
    }
}

/// Which files a search looks at: in a module's folder, of a kind (lecture, exercise, …), of
/// a file type, changed since a time.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Filter {
    pub folder: Option<PathBuf>,
    pub kind: Option<crate::categories::Category>,
    /// Lower-case extensions without dot; empty means any.
    pub extensions: Vec<String>,
    /// Nanoseconds since 1970.
    pub since: Option<u64>,
}

impl Filter {
    pub fn allows(&self, doc: &Doc) -> bool {
        if self.folder.as_ref().is_some_and(|f| !doc.path.starts_with(f)) {
            return false;
        }
        if self.since.is_some_and(|t| doc.modified < t) {
            return false;
        }
        if !self.extensions.is_empty() {
            let ext = doc.path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
            if !self.extensions.contains(&ext) {
                return false;
            }
        }
        if let Some(kind) = self.kind {
            let stem = doc.path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            let folder = doc.path.parent().and_then(Path::file_name).map(|n| n.to_string_lossy().into_owned());
            let of_folder = folder.and_then(|f| crate::categories::kind_of(&f));
            if crate::categories::kind_of(&stem).or(of_folder) != Some(kind) {
                return false;
            }
        }
        true
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
    /// The system reported a change in the study folder.
    Changed,
    /// Stop watching and reading until `Resume` (saving resources in the background).
    Pause,
    Resume,
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
        let (shared, flag, own) = (Arc::clone(&index), Arc::clone(&busy), tx.clone());
        let _ = thread::Builder::new()
            .name("file-sorter-index".into())
            .spawn(move || run(rx, own, shared, flag, file, repaint));
        Indexer { index, busy, tx }
    }

    pub fn set_root(&self, root: PathBuf, extensions: Vec<String>) {
        let _ = self.tx.send(Job::Root(root, extensions));
    }

    pub fn refresh(&self) {
        let _ = self.tx.send(Job::Refresh);
    }

    /// Stop reading and watching (in the background, to save resources), or start again
    /// (and catch up on what changed meanwhile).
    pub fn pause(&self, paused: bool) {
        let _ = self.tx.send(if paused { Job::Pause } else { Job::Resume });
    }

    /// True while files are being read.
    pub fn busy(&self) -> bool {
        self.busy.load(std::sync::atomic::Ordering::Relaxed)
    }
}

/// Watch `folder` (and below) for changes; they arrive as `Job::Changed`.
fn watch(folder: &Path, tx: Sender<Job>) -> Option<notify::RecommendedWatcher> {
    use notify::Watcher;
    let mut watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
        if event.is_ok_and(|e| !e.kind.is_access()) {
            let _ = tx.send(Job::Changed);
        }
    })
    .ok()?;
    match watcher.watch(folder, notify::RecursiveMode::Recursive) {
        Ok(()) => Some(watcher),
        Err(e) => {
            log::info!("Can't watch the study folder for changes ({e}); it is read every hour");
            None
        }
    }
}

fn run(
    rx: Receiver<Job>,
    tx: Sender<Job>,
    index: Arc<RwLock<Index>>,
    busy: Arc<std::sync::atomic::AtomicBool>,
    file: Option<PathBuf>,
    repaint: impl Fn(),
) {
    let mut root: Option<(PathBuf, Vec<String>)> = None;
    let mut watcher: Option<notify::RecommendedWatcher> = None;
    let mut paused = false;
    // Something changed while paused: read when resumed.
    let mut missed = false;
    loop {
        let mut take = |job: Job, root: &mut Option<(PathBuf, Vec<String>)>, paused: &mut bool| match job {
            Job::Root(new, extensions) => {
                watcher = if *paused { None } else { watch(&new, tx.clone()) };
                *root = Some((new, extensions));
            }
            Job::Pause => {
                *paused = true;
                watcher = None;
            }
            Job::Resume => {
                *paused = false;
                if let Some((folder, _)) = root.as_ref() {
                    watcher = watch(folder, tx.clone());
                }
            }
            Job::Refresh | Job::Changed => {}
        };
        let first = match rx.recv_timeout(REFRESH) {
            Ok(job) => Some(job),
            Err(RecvTimeoutError::Timeout) => None,
            Err(RecvTimeoutError::Disconnected) => return,
        };
        let changed = matches!(first, Some(Job::Changed));
        let resumed = matches!(first, Some(Job::Resume));
        if let Some(job) = first {
            take(job, &mut root, &mut paused);
        }
        // Several requests in a row need only one pass; after a change, wait until it settles.
        let wait = if changed { SETTLE } else { Duration::ZERO };
        while let Ok(job) = rx.recv_timeout(wait) {
            take(job, &mut root, &mut paused);
        }
        if paused {
            missed = true;
            continue;
        }
        if resumed && !missed {
            continue;
        }
        missed = false;
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

    fn doc(path: &str, modified: u64, pages: &[&str]) -> Doc {
        Doc { path: PathBuf::from(path), size: 1, modified, pages: pages.iter().map(|p| p.to_string()).collect() }
    }

    #[test]
    fn ranks_by_relevance_with_phrases_beginnings_typos_and_exclusions() {
        let index = Index::from_docs(vec![
            doc(
                "/s/Prog/Vorlesung/VL5.pdf",
                1,
                &["Rekursion\nDie Rekursion ruft sich selbst auf. Rekursion und Iteration."],
            ),
            doc("/s/Prog/Übungen/Blatt3.pdf", 2, &["Aufgabe 1: schreiben Sie eine rekursive Funktion"]),
            doc("/s/Stats/Vorlesung/VL2.pdf", 3, &["Mittelwert und Varianz", "Die Varianz misst die Streuung"]),
            doc("/s/Stats/Übungen/Serie 2.pdf", 4, &["Berechnen Sie die Varianz der Stichprobe"]),
        ]);
        let paths =
            |hits: Vec<Hit>| hits.into_iter().map(|h| h.path.to_string_lossy().into_owned()).collect::<Vec<_>>();
        // The document all about it first; «rekurs» finds «Rekursion» and «rekursive».
        assert_eq!(paths(index.search("rekurs", 10)), ["/s/Prog/Vorlesung/VL5.pdf", "/s/Prog/Übungen/Blatt3.pdf"]);
        // A phrase must be together (also across a line break).
        assert_eq!(paths(index.search("\"misst die streuung\"", 10)), ["/s/Stats/Vorlesung/VL2.pdf"]);
        assert!(index.search("\"streuung misst\"", 10).is_empty());
        // Leave out: -wort.
        assert_eq!(paths(index.search("varianz -stichprobe", 10)), ["/s/Stats/Vorlesung/VL2.pdf"]);
        // One typo in a longer word, when nothing is found otherwise.
        assert_eq!(paths(index.search("Varianzz", 10)).len(), 2);
        assert_eq!(paths(index.search("Rekrusion", 10))[0], "/s/Prog/Vorlesung/VL5.pdf");
        assert!(index.search("xyzabc", 10).is_empty());
        let hit = &index.search("varianz", 10)[0];
        assert_eq!(hit.path, PathBuf::from("/s/Stats/Vorlesung/VL2.pdf"));
        assert_eq!((hit.page, hit.more_pages.clone()), (Some(1), vec![2]));
    }

    #[test]
    fn filters_by_module_kind_type_and_time() {
        let index = Index::from_docs(vec![
            doc("/s/Prog/Vorlesung/VL5.pdf", 10, &["Rekursion"]),
            doc("/s/Prog/Übungen/Blatt3.pdf", 20, &["Rekursion"]),
            doc("/s/Prog/notes.md", 30, &["Rekursion"]),
            doc("/s/Stats/Lösung Serie 1.pdf", 40, &["Rekursion"]),
        ]);
        let query = Query::parse("rekursion");
        let count = |filter: Filter| index.search_with(&query, &filter, 10).len();
        assert_eq!(count(Filter::default()), 4);
        assert_eq!(count(Filter { folder: Some("/s/Prog".into()), ..Default::default() }), 3);
        assert_eq!(count(Filter { kind: Some(crate::categories::Category::Exercise), ..Default::default() }), 1);
        assert_eq!(count(Filter { kind: Some(crate::categories::Category::Solution), ..Default::default() }), 1);
        assert_eq!(count(Filter { extensions: vec!["md".into()], ..Default::default() }), 1);
        assert_eq!(count(Filter { since: Some(25), ..Default::default() }), 2);
    }

    #[test]
    fn queries_are_split_into_words_phrases_and_exclusions() {
        let q = Query::parse("Rekursion „Teile und herrsche“ -Lösung \"bäume\"");
        assert_eq!(q.terms, ["rekursion"]);
        assert_eq!(q.phrases, ["teile und herrsche", "baume"]);
        assert_eq!(q.excluded, ["losung"]);
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
