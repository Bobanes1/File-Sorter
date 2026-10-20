//! Learn from the files already in the subfolders which files go where: a word that is in
//! the names of many files of «Vorlesung» and hardly anywhere else ("vl", "folien") sends
//! new files there too.
//!
//! Only names (and, if the search index has them, the first words of the text) are read.
//! A word must be typical of one subfolder in several modules, so a module's own name or
//! its lecturer isn't taken for a subfolder word.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use crate::categories::existing_subfolders;
use crate::config::{Config, LearnedFolder};
use crate::matcher::normalise;
use crate::sorter::is_candidate;

/// A word must be in this many files of the subfolder,
const MIN_FILES: usize = 3;
/// with at least this share of all subfolder files that have it,
const MIN_SHARE: f64 = 0.8;
/// and in at least this share of the subfolder's files.
const MIN_COVER: f64 = 0.15;
/// At most this many words per subfolder.
const MAX_WORDS: usize = 8;
/// Files read per subfolder of a module.
const MAX_FILES: usize = 400;
/// Words at the start of a document that count.
pub const TEXT_WORDS: usize = 20;

/// The words of a name or text that can tell a subfolder: no numbers, years, semesters or
/// everyday words.
pub fn terms(text: &str) -> HashSet<String> {
    const SEMESTERS: &[&str] = &["hs", "fs", "ws", "ss", "sose", "wise", "sem"];
    normalise(text)
        .split(' ')
        .map(|t| t.trim_matches(|c: char| !c.is_alphanumeric()))
        .filter(|t| {
            t.chars().count() >= 2
                && t.chars().all(char::is_alphabetic)
                && !SEMESTERS.contains(t)
                && !crate::config::STOPWORDS.contains(t)
                && !crate::config::FILLER_WORDS.contains(t)
        })
        .map(str::to_string)
        .collect()
}

/// Document files in `dir` and one level below.
pub fn files_in(dir: &Path, config: &Config) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(entries) = fs::read_dir(dir) else { return out };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if let Ok(inner) = fs::read_dir(&path) {
                out.extend(inner.flatten().map(|e| e.path()).filter(|p| p.is_file() && is_candidate(p, config)));
            }
        } else if is_candidate(&path, config) {
            out.push(path);
        }
        if out.len() >= MAX_FILES {
            break;
        }
    }
    out.truncate(MAX_FILES);
    out
}

/// The files of one subfolder (in all modules): the module each is in, and its terms.
type FolderFiles = Vec<(usize, HashSet<String>)>;

/// What was seen in the subfolders: per subfolder (by normalised name) the terms of each
/// file and the module it is in.
#[derive(Default)]
struct Seen {
    /// Normalised name → (name as written, its files).
    folders: HashMap<String, (String, FolderFiles)>,
}

/// Learn the words of every subfolder from the files in it. `starts` gives the first words
/// of a document's text, where known. The words the user took out stay out.
pub fn learn(config: &Config, starts: &HashMap<PathBuf, String>) -> Vec<LearnedFolder> {
    let mut seen = Seen::default();
    for (m, course) in config.courses.iter().enumerate() {
        let module = config.course_folder(course);
        for sub in existing_subfolders(&module, &config.features.versions_folder) {
            let key = normalise(&sub);
            let entry = seen.folders.entry(key).or_insert_with(|| (sub.clone(), Vec::new()));
            for file in files_in(&module.join(&sub), config) {
                let stem = file.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                let mut words = terms(&stem);
                if let Some(start) = starts.get(&file) {
                    words.extend(terms(start));
                }
                entry.1.push((m, words));
            }
        }
    }
    pick(config, &seen)
}

fn pick(config: &Config, seen: &Seen) -> Vec<LearnedFolder> {
    // Words of the modules themselves (name, keywords) and of the subfolder names.
    let mut module_words: HashSet<String> = HashSet::new();
    for course in &config.courses {
        for text in std::iter::once(&course.name).chain(&course.keywords).chain(std::iter::once(&course.subject)) {
            module_words.extend(terms(text));
        }
    }
    let folder_words: HashSet<String> = seen.folders.values().flat_map(|(name, _)| terms(name)).collect();
    // In how many subfolder files each word is.
    let mut total: HashMap<&str, usize> = HashMap::new();
    for (_, files) in seen.folders.values() {
        for (_, words) in files {
            for w in words {
                *total.entry(w.as_str()).or_default() += 1;
            }
        }
    }
    let mut learned = Vec::new();
    let mut keys: Vec<&String> = seen.folders.keys().collect();
    keys.sort();
    for key in keys {
        let (written, files) = &seen.folders[key];
        // The name as in the list, if it is there.
        let name = config
            .features
            .subfolder_list
            .iter()
            .find(|f| normalise(&f.name) == *key)
            .map_or_else(|| written.clone(), |f| f.name.clone());
        let previous = config.features.subfolder_learned.iter().find(|l| normalise(&l.name) == *key);
        let removed = previous.map(|l| l.removed.clone()).unwrap_or_default();
        let modules_with_files: HashSet<usize> = files.iter().map(|(m, _)| *m).collect();
        let mut counts: HashMap<&str, (usize, HashSet<usize>)> = HashMap::new();
        for (m, words) in files {
            for w in words {
                let entry = counts.entry(w.as_str()).or_default();
                entry.0 += 1;
                entry.1.insert(*m);
            }
        }
        let mut words: Vec<(&str, usize)> = counts
            .into_iter()
            .filter(|(w, (count, modules))| {
                *count >= MIN_FILES
                    && *count as f64 / total[w] as f64 >= MIN_SHARE
                    && *count as f64 / files.len() as f64 >= MIN_COVER
                    // Typical of the subfolder, not of one module (its code, its lecturer).
                    && (modules.len() >= 2 || modules_with_files.len() == 1 && config.courses.len() == 1)
                    && !module_words.contains(*w)
                    && !folder_words.contains(*w)
                    && !removed.iter().any(|r| r == w)
            })
            .map(|(w, (count, _))| (w, count))
            .collect();
        words.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
        let words: Vec<String> = words.into_iter().take(MAX_WORDS).map(|(w, _)| w.to_string()).collect();
        if !words.is_empty() || !removed.is_empty() {
            learned.push(LearnedFolder { name, words, removed });
        }
    }
    // Folders not seen this time (gone, renamed) keep what the user took out.
    for old in &config.features.subfolder_learned {
        if !old.removed.is_empty() && !learned.iter().any(|l| normalise(&l.name) == normalise(&old.name)) {
            learned.push(LearnedFolder { name: old.name.clone(), words: Vec::new(), removed: old.removed.clone() });
        }
    }
    learned
}

/// The words that are new in `new` compared with `old`: (subfolder, word).
pub fn added_words(old: &[LearnedFolder], new: &[LearnedFolder]) -> Vec<(String, String)> {
    let mut added = Vec::new();
    for folder in new {
        let before = old.iter().find(|o| normalise(&o.name) == normalise(&folder.name));
        for word in &folder.words {
            if !before.is_some_and(|b| b.words.contains(word)) {
                added.push((folder.name.clone(), word.clone()));
            }
        }
    }
    added
}

/// Take a learned word out (and keep it out).
pub fn remove_word(learned: &mut [LearnedFolder], folder: &str, word: &str) {
    if let Some(f) = learned.iter_mut().find(|f| f.name == folder) {
        f.words.retain(|w| w != word);
        if !f.removed.iter().any(|r| r == word) {
            f.removed.push(word.to_string());
        }
    }
}

/// The subfolder the learned words point to for a (normalised) name or text: the one with the
/// most of its words in it, if there is one clear winner among `allowed`.
pub fn best_folder(text: &str, learned: &[LearnedFolder], allowed: impl Fn(&str) -> bool) -> Option<String> {
    let words = terms(text);
    let mut scores: Vec<(usize, &str)> = learned
        .iter()
        .filter(|f| allowed(&f.name))
        .map(|f| (f.words.iter().filter(|w| words.contains(*w)).count(), f.name.as_str()))
        .filter(|(score, _)| *score > 0)
        .collect();
    scores.sort_by_key(|s| std::cmp::Reverse(s.0));
    match scores.as_slice() {
        [(best, name), rest @ ..] if rest.first().is_none_or(|(second, _)| second < best) => Some(name.to_string()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sorter::tests::{make_pdf, test_config};

    fn put(dir: &Path, names: &[&str]) {
        fs::create_dir_all(dir).unwrap();
        for name in names {
            make_pdf(&dir.join(name), "x");
        }
    }

    #[test]
    fn learns_words_typical_of_a_subfolder() {
        let tmp = tempfile::tempdir().unwrap();
        let mut config = test_config(tmp.path());
        let prog = config.course_folder(&config.courses[0]);
        let stats = config.course_folder(&config.courses[1]);
        // «Folien» and «Handout» are typical of «Vorlesung» in both modules; «Huber» is the
        // lecturer of one module only; «Woche» is everywhere.
        put(
            &prog.join("Vorlesung"),
            &["Folien Woche 1.pdf", "Folien Woche 2.pdf", "Huber Handout 3.pdf", "Huber Folien 4.pdf"],
        );
        put(&stats.join("Vorlesung"), &["Folien Teil 1.pdf", "Handout 2.pdf", "Handout 3.pdf"]);
        put(&prog.join("Übungen"), &["Woche 1 Aufgaben.pdf", "Huber Aufgaben 2.pdf", "Woche 3 Aufgaben.pdf"]);
        put(&stats.join("Übungen"), &["Aufgaben 1.pdf", "Woche 2.pdf"]);
        let learned = learn(&config, &HashMap::new());
        let words = |name: &str| learned.iter().find(|l| l.name == name).map(|l| l.words.clone()).unwrap_or_default();
        assert_eq!(words("Vorlesung"), ["folien", "handout"]);
        assert_eq!(words("Übungen"), ["aufgaben"]);

        // A word taken out stays out when learning again.
        config.features.subfolder_learned = learned;
        remove_word(&mut config.features.subfolder_learned, "Vorlesung", "handout");
        let again = learn(&config, &HashMap::new());
        let vorlesung = again.iter().find(|l| l.name == "Vorlesung").unwrap();
        assert_eq!(vorlesung.words, ["folien"]);
        assert_eq!(vorlesung.removed, ["handout"]);
        assert_eq!(added_words(&config.features.subfolder_learned, &again), []);
    }

    #[test]
    fn needs_enough_files_and_skips_module_words() {
        let tmp = tempfile::tempdir().unwrap();
        let config = test_config(tmp.path());
        let prog = config.course_folder(&config.courses[0]);
        let stats = config.course_folder(&config.courses[1]);
        // Only two files with «skript»: not enough. The module's keyword «prog1» is no subfolder word.
        put(&prog.join("Vorlesung"), &["Skript 1.pdf", "prog1 Kapitel 2.pdf", "prog1 Kapitel 3.pdf"]);
        put(&stats.join("Vorlesung"), &["Skript 2.pdf", "Kapitel 4.pdf"]);
        let learned = learn(&config, &HashMap::new());
        let vorlesung = learned.iter().find(|l| l.name == "Vorlesung").map(|l| l.words.clone()).unwrap_or_default();
        assert_eq!(vorlesung, ["kapitel"]);
    }

    #[test]
    fn a_clear_winner_or_nothing() {
        let learned = vec![
            LearnedFolder { name: "Vorlesung".into(), words: vec!["folien".into(), "handout".into()], removed: vec![] },
            LearnedFolder { name: "Übungen".into(), words: vec!["aufgaben".into()], removed: vec![] },
        ];
        let all = |_: &str| true;
        assert_eq!(best_folder("Folien Handout Woche 5", &learned, all).as_deref(), Some("Vorlesung"));
        assert_eq!(best_folder("Aufgaben 3", &learned, all).as_deref(), Some("Übungen"));
        assert_eq!(best_folder("Folien zu den Aufgaben", &learned, all), None, "a tie");
        assert_eq!(best_folder("Organisation", &learned, all), None);
        assert_eq!(best_folder("Aufgaben 3", &learned, |n| n != "Übungen"), None, "not allowed here");
    }
}
