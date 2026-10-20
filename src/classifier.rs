//! A small learning classifier (naive Bayes) that knows the modules by the files already in
//! them: the words of their names and of the start of their text. It is asked only when the
//! keywords give no module or a tie, and it only answers when it is clearly sure.
//!
//! The model is built in the background (from the module folders and the search index) and
//! kept in `module-model.json`; sorting reads it from there.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

use crate::config::Config;
use crate::subfolder_learn::terms;

pub const MODEL_FILE: &str = "module-model.json";
/// Files read per module.
const MAX_FILES: usize = 400;
/// A module needs this many files before it is known well enough.
const MIN_FILES: usize = 5;
/// The answer must be at least this likely,
const MIN_PROBABILITY: f64 = 0.8;
/// and this many times as likely as the second.
const MIN_LEAD: f64 = 2.0;
/// A file must have this many words the model knows.
const MIN_KNOWN_WORDS: usize = 2;
/// Words at the start of a document that are learned from.
pub const TEXT_WORDS: usize = 40;

/// What the files of one module say.
#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq)]
struct ModuleWords {
    /// The module's label ("Informatik › Programmieren").
    module: String,
    files: usize,
    /// In how many files each word is.
    counts: HashMap<String, usize>,
    total: usize,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq)]
pub struct Model {
    modules: Vec<ModuleWords>,
}

/// Document files in `dir` and below (a few levels), newest first.
fn module_files(dir: &Path, config: &Config) -> Vec<PathBuf> {
    fn walk(dir: &Path, depth: usize, config: &Config, out: &mut Vec<(SystemTime, PathBuf)>) {
        let Ok(entries) = fs::read_dir(dir) else { return };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if depth < 3 && !entry.file_name().to_string_lossy().starts_with('.') {
                    walk(&path, depth + 1, config, out);
                }
            } else if crate::sorter::is_candidate(&path, config) {
                let modified = entry.metadata().and_then(|m| m.modified()).unwrap_or(SystemTime::UNIX_EPOCH);
                out.push((modified, path));
            }
        }
    }
    let mut files = Vec::new();
    walk(dir, 0, config, &mut files);
    files.sort_by(|a, b| b.cmp(a));
    files.into_iter().take(MAX_FILES).map(|(_, p)| p).collect()
}

/// The words of a file: its name, and the start of its text if known.
fn file_words(path: &Path, start: Option<&String>) -> HashSet<String> {
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let mut words = terms(&stem);
    if let Some(text) = start {
        words.extend(terms(text));
    }
    words
}

impl Model {
    /// Learn the modules from the files in their folders. `starts` gives the first words of
    /// documents, where known.
    pub fn train(config: &Config, starts: &HashMap<PathBuf, String>) -> Model {
        let mut modules = Vec::new();
        for course in &config.courses {
            let mut words = ModuleWords { module: course.label(), ..Default::default() };
            for file in module_files(&config.course_folder(course), config) {
                words.files += 1;
                for w in file_words(&file, starts.get(&file)) {
                    *words.counts.entry(w).or_default() += 1;
                    words.total += 1;
                }
            }
            modules.push(words);
        }
        Model { modules }
    }

    /// The module (label) for a file with these words, and the words that pointed there, if
    /// the model is clearly sure; only among `allowed` labels when given.
    pub fn predict(&self, words: &HashSet<String>, allowed: Option<&[String]>) -> Option<(String, Vec<String>)> {
        let candidates: Vec<&ModuleWords> = self
            .modules
            .iter()
            .filter(|m| m.files >= MIN_FILES)
            .filter(|m| allowed.is_none_or(|a| a.contains(&m.module)))
            .collect();
        if candidates.len() < 2 {
            // With one known module there is nothing to compare against.
            return None;
        }
        let vocabulary: HashSet<&String> = self.modules.iter().flat_map(|m| m.counts.keys()).collect();
        let known: Vec<&String> = words.iter().filter(|w| vocabulary.contains(w)).collect();
        if known.len() < MIN_KNOWN_WORDS {
            return None;
        }
        let all_files: usize = candidates.iter().map(|m| m.files).sum();
        let v = vocabulary.len() as f64;
        let logs: Vec<f64> = candidates
            .iter()
            .map(|m| {
                let prior = (m.files as f64 / all_files as f64).ln();
                known.iter().fold(prior, |acc, w| {
                    let count = m.counts.get(*w).copied().unwrap_or(0) as f64;
                    acc + ((count + 1.0) / (m.total as f64 + v)).ln()
                })
            })
            .collect();
        let top = logs.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
        let sum: f64 = logs.iter().map(|l| (l - top).exp()).sum();
        let mut probabilities: Vec<(f64, usize)> =
            logs.iter().enumerate().map(|(i, l)| ((l - top).exp() / sum, i)).collect();
        probabilities.sort_by(|a, b| b.0.total_cmp(&a.0));
        let (best, i) = probabilities[0];
        let second = probabilities.get(1).map_or(0.0, |p| p.0);
        if best < MIN_PROBABILITY || best < second * MIN_LEAD {
            return None;
        }
        let module = candidates[i];
        // The words that point most to it, to show why.
        let mut why: Vec<&String> =
            known.into_iter().filter(|w| module.counts.get(*w).is_some_and(|c| *c >= 2)).collect();
        why.sort_by_key(|w| std::cmp::Reverse(module.counts[*w]));
        Some((module.module.clone(), why.into_iter().take(3).cloned().collect()))
    }

    pub fn save(&self, file: &Path) -> std::io::Result<()> {
        crate::cloud::save_json(self, file)
    }
}

/// The model saved in the state folder, read again when the file changed.
pub fn load(config: &Config) -> Option<Model> {
    static CACHE: Mutex<Option<(PathBuf, SystemTime, Model)>> = Mutex::new(None);
    let file = config.state_file(MODEL_FILE)?;
    let modified = fs::metadata(&file).and_then(|m| m.modified()).ok()?;
    let mut cache = CACHE.lock().ok()?;
    if let Some((path, time, model)) = cache.as_ref()
        && *path == file
        && *time == modified
    {
        return Some(model.clone());
    }
    let model: Model = serde_json::from_str(&fs::read_to_string(&file).ok()?).ok()?;
    *cache = Some((file, modified, model.clone()));
    Some(model)
}

/// The module for `path` (with the text read from it, if any) by the learned model.
pub fn guess(
    config: &Config,
    path: &Path,
    text: Option<&str>,
    allowed: Option<&[String]>,
) -> Option<(String, Vec<String>)> {
    let model = load(config)?;
    let start = text.map(|t| t.split_whitespace().take(TEXT_WORDS).collect::<Vec<_>>().join(" "));
    model.predict(&file_words(path, start.as_ref()), allowed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sorter::tests::{make_pdf, test_config};

    #[test]
    fn learns_modules_from_their_files_and_only_answers_when_sure() {
        let tmp = tempfile::tempdir().unwrap();
        let config = test_config(tmp.path());
        let prog = config.course_folder(&config.courses[0]);
        let stats = config.course_folder(&config.courses[2]);
        fs::create_dir_all(&prog).unwrap();
        fs::create_dir_all(&stats).unwrap();
        for name in ["Rekursion Baum", "Rekursion Liste", "Schleifen Rekursion", "Objekte Klassen", "Klassen Vererbung"]
        {
            make_pdf(&prog.join(format!("{name}.pdf")), "x");
        }
        for name in ["Varianz Mittelwert", "Regression Varianz", "Verteilung Normal", "Hypothese Test", "Varianz Test"]
        {
            make_pdf(&stats.join(format!("{name}.pdf")), "x");
        }
        let model = Model::train(&config, &HashMap::new());
        let words = |text: &str| terms(text);
        let (module, why) = model.predict(&words("Rekursion Klassen Übersicht"), None).unwrap();
        assert_eq!(module, config.courses[0].label());
        assert!(why.contains(&"rekursion".to_string()));
        assert_eq!(model.predict(&words("Varianz Regression"), None).unwrap().0, config.courses[2].label());
        // Too few known words, or both modules about as likely: no answer.
        assert_eq!(model.predict(&words("Rekursion Ferien"), None), None);
        assert_eq!(model.predict(&words("Rekursion Varianz"), None), None);
        // Only among the modules in question.
        let only_stats = [config.courses[2].label()];
        assert_eq!(model.predict(&words("Rekursion Klassen"), Some(&only_stats)), None);
    }

    #[test]
    fn modules_with_few_files_are_not_guessed() {
        let tmp = tempfile::tempdir().unwrap();
        let config = test_config(tmp.path());
        let prog = config.course_folder(&config.courses[0]);
        fs::create_dir_all(&prog).unwrap();
        make_pdf(&prog.join("Rekursion Baum.pdf"), "x");
        let model = Model::train(&config, &HashMap::new());
        assert_eq!(model.predict(&terms("Rekursion Baum"), None), None);
    }
}
