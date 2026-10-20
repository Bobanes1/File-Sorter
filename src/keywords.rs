//! Suggest keywords for a module from the files already in its folder.
//!
//! Good keywords are terms that show up in many of the module's own files (module
//! code, lecturer, recurring topics) but hardly ever in other modules' files, since
//! a term shared by two modules only produces ties and leaves files unsorted.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::common_words::is_common;
use crate::config::{Config, Course, FILLER_WORDS, STOPWORDS};
use crate::extract::extract_text;
use crate::matcher::{count_hits, normalise};
use crate::sorter::is_candidate;

const MAX_OWN_FILES: usize = 40;
const MAX_OTHER_FILES: usize = 15;
const MAX_SUGGESTIONS: usize = 6;
/// A term found in more than this share of other modules' files is not distinctive.
const MAX_OTHER_SHARE: f64 = 0.05;
const MAX_DEPTH: usize = 4;

/// Words found in every course's material (in normalised form: lowercase, no accents).
const GENERIC_WORDS: &[&str] = &[
    // German
    "vorlesung",
    "vorlesungen",
    "folie",
    "folien",
    "ubung",
    "ubungen",
    "ubungsblatt",
    "blatt",
    "kapitel",
    "seite",
    "seiten",
    "woche",
    "semester",
    "wintersemester",
    "sommersemester",
    "aufgabe",
    "aufgaben",
    "losung",
    "losungen",
    "musterlosung",
    "skript",
    "inhalt",
    "inhaltsverzeichnis",
    "ubersicht",
    "einleitung",
    "zusammenfassung",
    "beispiel",
    "beispiele",
    "definition",
    "satz",
    "beweis",
    "abbildung",
    "tabelle",
    "universitat",
    "hochschule",
    "institut",
    "fakultat",
    "professor",
    "prof",
    "termin",
    "datum",
    "abgabe",
    "ist",
    "sind",
    "wird",
    "werden",
    "wir",
    "sie",
    "ich",
    "nicht",
    "auch",
    "aus",
    "bei",
    "bis",
    "nach",
    "uber",
    "unter",
    "vor",
    "wie",
    "als",
    "wenn",
    "dann",
    "dass",
    "sich",
    "sein",
    "ihre",
    "ihr",
    "eines",
    "einer",
    "einem",
    "einen",
    "kann",
    "konnen",
    "muss",
    "mussen",
    "soll",
    "sollen",
    "hat",
    "haben",
    "hier",
    "dies",
    "diese",
    "dieser",
    "dieses",
    "alle",
    "mehr",
    "nur",
    "noch",
    "schon",
    "sehr",
    "was",
    "wer",
    "durch",
    "gegen",
    "ohne",
    "aber",
    "denn",
    "sondern",
    "also",
    "nun",
    "jede",
    "jeder",
    "jedes",
    "sowie",
    "bzw",
    // English
    "lecture",
    "lectures",
    "slide",
    "slides",
    "exercise",
    "exercises",
    "sheet",
    "chapter",
    "page",
    "pages",
    "week",
    "solution",
    "solutions",
    "homework",
    "assignment",
    "task",
    "tasks",
    "university",
    "faculty",
    "department",
    "institute",
    "date",
    "note",
    "notes",
    "contents",
    "overview",
    "summary",
    "example",
    "examples",
    "theorem",
    "lemma",
    "proof",
    "figure",
    "table",
    "this",
    "that",
    "these",
    "those",
    "are",
    "was",
    "were",
    "has",
    "have",
    "had",
    "will",
    "would",
    "can",
    "could",
    "should",
    "may",
    "might",
    "not",
    "but",
    "also",
    "into",
    "over",
    "under",
    "about",
    "which",
    "what",
    "when",
    "where",
    "who",
    "how",
    "all",
    "any",
    "each",
    "other",
    "some",
    "such",
    "only",
    "more",
    "most",
    "very",
    "there",
    "their",
    "they",
    "them",
    "then",
    "than",
    "its",
    "our",
    "your",
    "you",
    "his",
    "her",
    "she",
    "him",
    "been",
    "being",
    "does",
    "did",
    "use",
    "used",
    "using",
    "via",
    "per",
    "both",
    // File and web noise
    "pdf",
    "pptx",
    "docx",
    "http",
    "https",
    "www",
    "html",
    "email",
    "mail",
    "copy",
    "final",
    "draft",
    "version",
];

/// What kind of term a suggestion is, shown next to it in the preview.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A module code such as "INF 101".
    Code,
    /// A lecturer's name (found after "Prof." or "Dr.").
    Lecturer,
    /// Found in file names or on title pages.
    Title,
    /// A recurring topic.
    Topic,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Suggested {
    /// Normalised form, ready to add to the module.
    pub term: String,
    /// In how many of the module's files it appears.
    pub files: usize,
    pub kind: Kind,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Suggestion {
    /// New keywords, best first.
    pub keywords: Vec<Suggested>,
    pub files_scanned: usize,
    pub folder: PathBuf,
}

impl Suggestion {
    #[cfg(test)]
    fn terms(&self) -> Vec<String> {
        self.keywords.iter().map(|k| k.term.clone()).collect()
    }
}

/// Document files in `folder` (and its subfolders), newest first.
fn module_files(folder: &Path, config: &Config, limit: usize) -> Vec<PathBuf> {
    fn walk(dir: &Path, depth: usize, config: &Config, out: &mut Vec<(SystemTime, PathBuf)>) {
        let Ok(entries) = fs::read_dir(dir) else { return };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if depth < MAX_DEPTH {
                    walk(&path, depth + 1, config, out);
                }
            } else if is_candidate(&path, config) {
                let modified = entry.metadata().and_then(|m| m.modified()).unwrap_or(SystemTime::UNIX_EPOCH);
                out.push((modified, path));
            }
        }
    }
    let mut files = Vec::new();
    walk(folder, 0, config, &mut files);
    files.sort_by(|a, b| b.cmp(a));
    files.into_iter().take(limit).map(|(_, p)| p).collect()
}

fn is_year(token: &str) -> bool {
    token.len() == 4
        && token.chars().all(|c| c.is_ascii_digit())
        && (token.starts_with("19") || token.starts_with("20"))
}

fn is_filler(token: &str) -> bool {
    STOPWORDS.contains(&token) || FILLER_WORDS.contains(&token) || GENERIC_WORDS.contains(&token) || is_common(token)
}

/// A token that can be a keyword on its own: a real, not too short word that isn't everyday language.
fn is_word(token: &str) -> bool {
    token.chars().count() >= 4
        && token.chars().all(char::is_alphanumeric)
        && token.chars().any(char::is_alphabetic)
        && !is_filler(token)
}

/// A number that can complete a module code ("inf 101", "ma 1"), but not a year.
fn is_code_number(token: &str) -> bool {
    !token.is_empty() && token.len() <= 4 && token.chars().all(|c| c.is_ascii_digit()) && !is_year(token)
}

/// The letters of a module code ("inf", "ma", "stat", "bwl"), not a word like "vl" or "kap".
fn is_code_letters(token: &str) -> bool {
    const NOT_CODES: &[&str] = &[
        "vl", "kap", "nr", "no", "teil", "folie", "seite", "woche", "week", "blatt", "sheet", "ss", "ws", "hs", "fs",
        "v",
    ];
    (2..=5).contains(&token.chars().count())
        && token.chars().all(char::is_alphabetic)
        && !NOT_CODES.contains(&token)
        && !STOPWORDS.contains(&token)
        && !FILLER_WORDS.contains(&token)
        && !GENERIC_WORDS.contains(&token)
        && !is_common(token)
}

/// Titles before a lecturer's name ("Prof. Dr. Huber").
const TITLES: &[&str] =
    &["prof", "professor", "professorin", "dr", "dozent", "dozentin", "lecturer", "pd", "med", "rer", "nat", "phil"];
/// How many words at the start of a document count as its title page.
const TITLE_WORDS: usize = 60;

/// What one file says about its module.
#[derive(Default)]
struct FileTerms {
    /// Every candidate term.
    all: HashSet<String>,
    /// Terms in the file name or on the title page.
    prominent: HashSet<String>,
    codes: HashSet<String>,
    lecturers: HashSet<String>,
    /// Words seen with a capital first letter, and seen in lowercase (German nouns are
    /// capitalised, so a German word only ever written in lowercase is no topic).
    capitalised: HashSet<String>,
    lowercase: HashSet<String>,
    /// How German or English the text looks.
    german: usize,
    english: usize,
}

impl FileTerms {
    fn of(path: &Path) -> FileTerms {
        let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let mut terms = FileTerms::default();
        // A file that is only in the cloud (OneDrive "online only") would be downloaded just to
        // be read here; its name has to do.
        let text = if crate::cloud::is_placeholder(path) { String::new() } else { extract_text(path) };
        terms.add(&normalise(&stem), usize::MAX);
        terms.add(&normalise(&text), TITLE_WORDS);
        terms.note_case(&text);
        terms
    }

    fn note_case(&mut self, text: &str) {
        const GERMAN: &[&str] = &["und", "der", "die", "das", "ist", "nicht", "mit", "wir", "werden"];
        const ENGLISH: &[&str] = &["the", "and", "is", "of", "to", "we", "are", "with", "this"];
        for raw in text.split(|c: char| !c.is_alphanumeric()).filter(|w| w.chars().count() >= 4 || GERMAN.contains(w)) {
            let word = normalise(raw);
            if GERMAN.contains(&word.as_str()) {
                self.german += 1;
            } else if ENGLISH.contains(&word.as_str()) {
                self.english += 1;
            }
            match raw.chars().next() {
                Some(c) if c.is_uppercase() => self.capitalised.insert(word),
                Some(c) if c.is_lowercase() => self.lowercase.insert(word),
                _ => false,
            };
        }
    }

    /// Add the terms of one text; the first `prominent_words` words count as prominent.
    fn add(&mut self, text: &str, prominent_words: usize) {
        let tokens: Vec<&str> =
            text.split(' ').map(|t| t.trim_matches(|c: char| !c.is_alphanumeric())).filter(|t| !t.is_empty()).collect();
        let insert = |terms: &mut FileTerms, term: String, i: usize| {
            if i < prominent_words {
                terms.prominent.insert(term.clone());
            }
            terms.all.insert(term);
        };
        for (i, token) in tokens.iter().enumerate() {
            let next = tokens.get(i + 1).copied();
            if is_code_letters(token)
                && let Some(number) = next.filter(|n| is_code_number(n))
            {
                let code = format!("{token} {number}");
                self.codes.insert(code.clone());
                insert(self, code, i);
            }
            if TITLES.contains(token)
                && let Some(name) = tokens[i + 1..].iter().find(|t| !TITLES.contains(t))
                && name.chars().count() >= 3
                && name.chars().all(char::is_alphabetic)
                && !is_filler(name)
            {
                self.lecturers.insert(name.to_string());
                insert(self, name.to_string(), i);
            }
            if !is_word(token) {
                continue;
            }
            insert(self, token.to_string(), i);
            if let Some(next) = next.filter(|n| is_word(n)) {
                insert(self, format!("{token} {next}"), i);
            }
        }
    }
}

fn course_terms(course: &Course) -> Vec<String> {
    std::iter::once(&course.name).chain(&course.keywords).map(|t| normalise(t)).filter(|t| !t.is_empty()).collect()
}

/// In how many files each term appears.
fn count<'a>(sets: impl Iterator<Item = &'a HashSet<String>>) -> HashMap<String, usize> {
    let mut counts = HashMap::new();
    for set in sets {
        for term in set {
            *counts.entry(term.clone()).or_insert(0) += 1;
        }
    }
    counts
}

/// At most this many keywords are learned from one file.
const MAX_LEARNED: usize = 2;

/// Keywords to learn when the user puts `file` into `config.courses[index]` by hand: up to
/// two terms of its name (a module code first, then the longest words) that aren't
/// everyday words, aren't a keyword of any module yet, and don't appear in the names of
/// other modules' files (they would only cause ties).
pub fn learn_from_name(config: &Config, index: usize, file: &Path) -> Vec<String> {
    let stem = file.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let mut terms = FileTerms::default();
    terms.add(&normalise(&stem), usize::MAX);
    let known: HashSet<String> = config.courses.iter().flat_map(course_terms).collect();
    let mut candidates: Vec<String> = terms
        .all
        .into_iter()
        .filter(|t| (terms.codes.contains(t) || !t.contains(' ')) && !known.contains(t))
        .filter(|t| !known.iter().any(|k| count_hits(t, k) > 0))
        .collect();
    // Module codes first, then longer (more specific) words.
    candidates.sort_by_key(|t| (!terms.codes.contains(t), std::cmp::Reverse(t.chars().count()), t.clone()));
    let other_names: Vec<String> = config
        .courses
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != index)
        .flat_map(|(_, c)| module_files(&config.course_folder(c), config, MAX_OWN_FILES))
        .filter_map(|p| p.file_stem().map(|s| normalise(&s.to_string_lossy())))
        .collect();
    let mut learned: Vec<String> = Vec::new();
    for term in candidates {
        if learned.len() == MAX_LEARNED {
            break;
        }
        if other_names.iter().any(|n| count_hits(n, &term) > 0) || learned.iter().any(|l| l.contains(&term)) {
            continue;
        }
        learned.push(term);
    }
    learned.into_iter().map(|t| readable(&t, &stem, terms.codes.contains(&t))).collect()
}

/// A learned term as the user would write it: a code in capitals ("INF 101"), a word as it
/// is spelled in the file name (with its accents), in lowercase.
fn readable(term: &str, stem: &str, code: bool) -> String {
    if code {
        return term.to_uppercase();
    }
    stem.split(|c: char| !c.is_alphanumeric())
        .find(|raw| normalise(raw) == term)
        .map(str::to_lowercase)
        .unwrap_or_else(|| term.to_string())
}

/// Suggest new keywords for `config.courses[index]` from the files in its folder.
///
/// A good keyword is specific to this module: it appears in many of the module's files
/// and hardly ever in other modules' files, and it isn't an everyday word. Module codes,
/// the lecturer's name and terms from file names and title pages count extra.
pub fn suggest_keywords(config: &Config, index: usize) -> Suggestion {
    let course = &config.courses[index];
    let folder = config.course_folder(course);
    let own_files = module_files(&folder, config, MAX_OWN_FILES);
    let mut suggestion = Suggestion { keywords: Vec::new(), files_scanned: own_files.len(), folder: folder.clone() };
    if own_files.is_empty() {
        return suggestion;
    }
    let own: Vec<FileTerms> = own_files.iter().map(|f| FileTerms::of(f)).collect();
    let own_df = count(own.iter().map(|f| &f.all));
    let prominent_df = count(own.iter().map(|f| &f.prominent));
    let codes = count(own.iter().map(|f| &f.codes));
    let lecturers = count(own.iter().map(|f| &f.lecturers));
    let german = own.iter().map(|f| f.german).sum::<usize>() > own.iter().map(|f| f.english).sum::<usize>();
    let capitalised = count(own.iter().map(|f| &f.capitalised));
    let lowercase = count(own.iter().map(|f| &f.lowercase));

    // What the other modules' files and settings look like.
    let mut other_terms = Vec::new();
    let mut other_df: HashMap<String, usize> = HashMap::new();
    let mut other_modules: HashMap<String, usize> = HashMap::new();
    let mut n_other = 0;
    for (i, other) in config.courses.iter().enumerate() {
        if i == index {
            continue;
        }
        other_terms.extend(course_terms(other));
        let other_folder = config.course_folder(other);
        if other_folder == folder {
            continue;
        }
        let files: Vec<FileTerms> =
            module_files(&other_folder, config, MAX_OTHER_FILES).iter().map(|f| FileTerms::of(f)).collect();
        n_other += files.len();
        let df = count(files.iter().map(|f| &f.all));
        for (term, n) in df {
            *other_modules.entry(term.clone()).or_insert(0) += 1;
            *other_df.entry(term).or_insert(0) += n;
        }
    }
    let existing = course_terms(course);
    let existing_df: Vec<(String, usize)> =
        existing.iter().map(|t| (t.clone(), *own_df.get(t).unwrap_or(&0))).collect();

    let n_own = own_files.len() as f64;
    let min_df = if own_files.len() == 1 { 1 } else { 2 };
    let mut ranked: Vec<(f64, Suggested)> = own_df
        .iter()
        .filter(|(_, df)| **df >= min_df)
        .filter_map(|(term, &df)| {
            let in_other = *other_df.get(term).unwrap_or(&0) as f64;
            // Used by two other modules, or in more than a few of their files: not distinctive.
            if *other_modules.get(term).unwrap_or(&0) >= 2 || in_other / (n_other.max(1) as f64) > MAX_OTHER_SHARE {
                return None;
            }
            // Part of another module's name or keywords: would cause ties.
            if other_terms.iter().any(|t| count_hits(t, term) > 0 || count_hits(term, t) > 0) {
                return None;
            }
            // Already covered: an existing keyword matches wherever this term does.
            if existing.iter().any(|t| count_hits(term, t) > 0) {
                return None;
            }
            let share = df as f64 / n_own;
            let rarity = ((n_other as f64 + 1.0) / (in_other + 1.0)).ln() + 1.0;
            let prominence = *prominent_df.get(term).unwrap_or(&0) as f64 / df as f64;
            let kind = if codes.contains_key(term) {
                Kind::Code
            } else if lecturers.contains_key(term) {
                Kind::Lecturer
            } else if prominence >= 0.5 {
                Kind::Title
            } else {
                Kind::Topic
            };
            let boost = match kind {
                Kind::Code => 3.0,
                Kind::Lecturer => 2.5,
                _ => 1.0 + prominence,
            };
            // A topic must recur: in at least a third of the module's files.
            if kind == Kind::Topic && own_files.len() >= 3 && share < 1.0 / 3.0 {
                return None;
            }
            // In German texts, a single word never written with a capital is a verb or an
            // adjective ("behandeln", "wichtig"), not a topic.
            let lower_only = !capitalised.contains_key(term) && lowercase.contains_key(term);
            if german && matches!(kind, Kind::Topic | Kind::Title) && !term.contains(' ') && lower_only {
                return None;
            }
            let phrase = if term.contains(' ') { 1.2 } else { 1.0 };
            Some((share * rarity * boost * phrase, Suggested { term: term.clone(), files: df, kind }))
        })
        .collect();
    ranked.sort_by(|a, b| b.0.total_cmp(&a.0).then(b.1.files.cmp(&a.1.files)).then(a.1.term.cmp(&b.1.term)));

    let mut chosen: Vec<Suggested> = Vec::new();
    for (_, candidate) in ranked {
        let picked = chosen.iter().map(|c| (c.term.clone(), c.files)).chain(existing_df.iter().cloned());
        let redundant = picked.into_iter().any(|(picked, picked_df)| {
            // A phrase containing a chosen word adds nothing; a word inside a chosen phrase
            // only adds something if it appears in more files than the phrase.
            count_hits(&candidate.term, &picked) > 0
                || (count_hits(&picked, &candidate.term) > 0 && candidate.files <= picked_df)
        });
        if !redundant {
            chosen.push(candidate);
        }
        if chosen.len() == MAX_SUGGESTIONS {
            break;
        }
    }
    suggestion.keywords = chosen;
    suggestion
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::matcher::match_file;
    use crate::sorter::tests::make_pdf;

    struct Tree {
        _tmp: tempfile::TempDir,
        config: Config,
    }

    /// Studium/Informatik/Programmierung 1 and Studium/Mathematik/Analysis II with some PDFs.
    fn tree(analysis_mentions_huber: bool) -> Tree {
        let tmp = tempfile::tempdir().unwrap();
        let studium = tmp.path().join("Studium");
        let prog = studium.join("Informatik/Programmierung 1");
        let ana = studium.join("Mathematik/Analysis II");
        fs::create_dir_all(prog.join("Uebungen")).unwrap();
        fs::create_dir_all(&ana).unwrap();
        make_pdf(&prog.join("VL01_Einfuehrung.pdf"), "INF-101 Programmierung 1 Prof. Huber Rekursion Vorlesung");
        make_pdf(&prog.join("VL02.pdf"), "INF-101 Vorlesung 2 Huber Rekursion und Schleifen");
        make_pdf(&prog.join("Uebungen/Blatt3.pdf"), "Uebungsblatt INF-101 Rekursion Huber");
        make_pdf(&ana.join("ana1.pdf"), "Vorlesung Integral Analysis II");
        let guest = if analysis_mentions_huber { "Integral Huber Gastvortrag" } else { "Integral Gastvortrag" };
        make_pdf(&ana.join("ana2.pdf"), guest);
        let config = Config {
            destination_root: studium.to_string_lossy().into(),
            courses: vec![
                Course::new("Programmierung 1", &[]).with_subject("Informatik"),
                Course::new("Analysis II", &[]).with_subject("Mathematik"),
            ],
            ..Config::default()
        };
        Tree { _tmp: tmp, config }
    }

    #[test]
    fn learns_codes_and_rare_words_from_the_name() {
        let tmp = tempfile::tempdir().unwrap();
        let config = crate::sorter::tests::test_config(tmp.path());
        let stats = config.course_folder(&config.courses[2]);
        fs::create_dir_all(&stats).unwrap();
        fs::write(stats.join("Bayes Übersicht.pdf"), "x").unwrap();
        let learn = |name: &str| learn_from_name(&config, 0, &config.watch_path().join(name));
        assert_eq!(learn("INF101_Übungsblatt_3_Rekursion.pdf"), vec!["INF 101", "rekursion"]);
        assert_eq!(learn("Blatt 3.pdf"), Vec::<String>::new(), "only everyday words");
        assert_eq!(learn("Bayes Rekursion.pdf"), vec!["rekursion"], "bayes is in another module's files");
        assert_eq!(learn("python Schleifen Iteratoren.pdf"), vec!["iteratoren", "schleifen"], "python is known");
        assert_eq!(learn("Größenordnung.pdf"), vec!["größenordnung"]);
    }

    #[test]
    fn finds_module_code_lecturer_and_topic() {
        let t = tree(false);
        let s = suggest_keywords(&t.config, 0);
        assert_eq!(s.files_scanned, 3);
        for expected in ["inf 101", "rekursion", "huber"] {
            assert!(s.terms().contains(&expected.to_string()), "{expected} missing from {:?}", s.keywords);
        }
        for unwanted in ["vorlesung", "integral", "inf", "uebungsblatt", "schleifen", "prof huber"] {
            assert!(!s.terms().contains(&unwanted.to_string()), "{unwanted} in {:?}", s.keywords);
        }
    }

    #[test]
    fn terms_shared_with_other_modules_are_dropped() {
        let t = tree(true);
        let s = suggest_keywords(&t.config, 0);
        assert!(!s.terms().contains(&"huber".to_string()), "{:?}", s.keywords);
        assert!(s.terms().contains(&"rekursion".to_string()));
    }

    #[test]
    fn running_twice_adds_nothing_new() {
        let mut t = tree(false);
        let first = suggest_keywords(&t.config, 0);
        assert!(!first.keywords.is_empty());
        t.config.courses[0].keywords.extend(first.terms());
        let second = suggest_keywords(&t.config, 0);
        assert!(second.keywords.is_empty(), "second run suggested {:?}", second.keywords);
    }

    #[test]
    fn existing_keywords_are_not_suggested_again() {
        let mut t = tree(false);
        t.config.courses[0].keywords = vec!["Rekursion".into(), "INF101".into()];
        let s = suggest_keywords(&t.config, 0);
        assert!(!s.terms().iter().any(|k| k == "rekursion" || k == "inf 101"), "{:?}", s.keywords);
        assert!(s.terms().contains(&"huber".to_string()));
    }

    #[test]
    fn other_module_names_block_terms() {
        let t = tree(false);
        // "integral" only appears in Analysis II's files; nothing containing "analysis" is suggested.
        let s = suggest_keywords(&t.config, 1);
        assert!(s.terms().contains(&"integral".to_string()), "{:?}", s.keywords);
        assert!(!s.terms().iter().any(|k| k.contains("analysis")), "{:?}", s.keywords);
    }

    #[test]
    fn suggestions_make_new_files_match() {
        let mut t = tree(false);
        let s = suggest_keywords(&t.config, 0);
        t.config.courses[0].keywords.extend(s.terms());
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("INF101_Blatt5.pdf");
        make_pdf(&file, "x");
        let m = match_file(&file, &t.config.courses, false, |_| String::new()).unwrap();
        assert_eq!(m.course, 0);
    }

    #[test]
    fn missing_or_empty_folder_gives_nothing() {
        let mut t = tree(false);
        t.config.courses.push(Course::new("Datenbanken", &[]).with_subject("Informatik"));
        let s = suggest_keywords(&t.config, 2);
        assert_eq!(s.files_scanned, 0);
        assert!(s.keywords.is_empty());
    }

    /// Lecture texts in German, padded with the everyday words every course uses.
    fn lecture(topic: &str) -> String {
        format!(
            "{topic}. Heute behandeln wir im Rahmen der Vorlesung die folgenden Themen. Ein Beispiel zeigt das \
             Ergebnis, die Daten und die Methode. Wichtig ist, dass wir sehen, welche Frage hier gestellt wird. \
             Zusammenfassung und Übersicht: die Aufgabe ist einfach, das Problem bekannt."
        )
    }

    fn study_tree(other_modules_have_files: bool) -> Tree {
        let tmp = tempfile::tempdir().unwrap();
        let studium = tmp.path().join("Studium");
        let stat = studium.join("Mecom/Statistik I");
        let prog = studium.join("Informatik/Programmierung 1");
        let media = studium.join("Mecom/Medienwirkung");
        for dir in [&stat, &prog, &media] {
            fs::create_dir_all(dir).unwrap();
        }
        make_pdf(&stat.join("STAT101_VL1.pdf"), &lecture("STAT 101 Dr. Keller Varianz Standardabweichung"));
        make_pdf(&stat.join("STAT101_VL2.pdf"), &lecture("STAT 101 Dr. Keller Varianz und Regression"));
        make_pdf(&stat.join("STAT101_VL3.pdf"), &lecture("STAT 101 Keller lineare Regression Varianz"));
        make_pdf(&stat.join("Uebung4.pdf"), &lecture("Statistik Varianz Regression Aufgabenblatt"));
        if other_modules_have_files {
            make_pdf(&prog.join("vl1.pdf"), &lecture("INF 101 Prof. Huber Rekursion Schleifen"));
            make_pdf(&prog.join("vl2.pdf"), &lecture("INF 101 Huber Rekursion Klassen Objekte"));
            make_pdf(&media.join("mw1.pdf"), &lecture("Agenda Setting Prof. Meier Medienwirkung"));
            make_pdf(&media.join("mw2.pdf"), &lecture("Agenda Setting Framing Meier"));
        }
        let config = Config {
            destination_root: studium.to_string_lossy().into(),
            courses: vec![
                Course::new("Statistik I", &[]).with_subject("Mecom"),
                Course::new("Programmierung 1", &[]).with_subject("Informatik"),
                Course::new("Medienwirkung", &[]).with_subject("Mecom"),
            ],
            ..Config::default()
        };
        Tree { _tmp: tmp, config }
    }

    #[test]
    fn suggests_specific_terms_not_everyday_words() {
        for other_modules_have_files in [true, false] {
            let t = study_tree(other_modules_have_files);
            let s = suggest_keywords(&t.config, 0);
            let terms = s.terms();
            for expected in ["stat 101", "keller", "varianz"] {
                assert!(terms.contains(&expected.to_string()), "{expected} missing from {terms:?}");
            }
            for unwanted in [
                "heute",
                "behandeln",
                "beispiel",
                "ergebnis",
                "daten",
                "methode",
                "wichtig",
                "sehen",
                "frage",
                "rahmen",
            ] {
                assert!(!terms.iter().any(|t| t.split(' ').any(|w| w == unwanted)), "{unwanted} in {terms:?}");
            }
            let code = s.keywords.iter().find(|k| k.term == "stat 101").unwrap();
            assert_eq!((code.kind, code.files), (Kind::Code, 3));
            assert_eq!(s.keywords.iter().find(|k| k.term == "keller").unwrap().kind, Kind::Lecturer);
            assert!(s.keywords.len() <= MAX_SUGGESTIONS);
        }
    }

    #[test]
    fn other_modules_get_their_own_terms() {
        let t = study_tree(true);
        let prog = suggest_keywords(&t.config, 1).terms();
        assert!(prog.contains(&"inf 101".to_string()) && prog.contains(&"rekursion".to_string()), "{prog:?}");
        let media = suggest_keywords(&t.config, 2).terms();
        assert!(media.iter().any(|t| t.contains("agenda")), "{media:?}");
        assert!(media.contains(&"meier".to_string()), "{media:?}");
    }

    #[test]
    fn real_world_pdf_terms() {
        let tmp = tempfile::tempdir().unwrap();
        let folder = tmp.path().join("Mathematik/Einführung in die Analysis I");
        fs::create_dir_all(&folder).unwrap();
        fs::copy(crate::sorter::tests::fixture("analysis_chromium.pdf"), folder.join("vl4.pdf")).unwrap();
        let config = Config {
            destination_root: tmp.path().to_string_lossy().into(),
            courses: vec![Course::new("Einführung in die Analysis I", &[]).with_subject("Mathematik")],
            ..Config::default()
        };
        let s = suggest_keywords(&config, 0);
        assert_eq!(s.files_scanned, 1);
        assert!(s.terms().iter().any(|k| k.contains("grenzwerte") || k.contains("folge")), "{:?}", s.keywords);
        assert!(!s.terms().iter().any(|k| k == "2026"), "{:?}", s.keywords);
    }
}
