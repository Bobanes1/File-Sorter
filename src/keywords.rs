//! Suggest keywords for a module from the files already in its folder.
//!
//! Good keywords are terms that show up in many of the module's own files (module
//! code, lecturer, recurring topics) but hardly ever in other modules' files, since
//! a term shared by two modules only produces ties and leaves files unsorted.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::config::{Config, Course, FILLER_WORDS, STOPWORDS};
use crate::extract::extract_text;
use crate::matcher::{count_hits, normalise};
use crate::sorter::is_candidate;

const MAX_OWN_FILES: usize = 40;
const MAX_OTHER_FILES: usize = 15;
const MAX_SUGGESTIONS: usize = 8;
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

#[derive(Debug, Clone, PartialEq)]
pub struct Suggestion {
    /// New keywords, best first (normalised form, ready to add to the module).
    pub keywords: Vec<String>,
    pub files_scanned: usize,
    pub folder: PathBuf,
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

/// A token that can be a keyword on its own: a real word, not filler.
fn is_word(token: &str) -> bool {
    token.chars().count() >= 3
        && token.chars().all(|c| c.is_alphanumeric())
        && token.chars().any(char::is_alphabetic)
        && !STOPWORDS.contains(&token)
        && !FILLER_WORDS.contains(&token)
        && !GENERIC_WORDS.contains(&token)
}

/// A number that can complete a module code ("inf 101", "ma 1"), but not a year.
fn is_code_number(token: &str) -> bool {
    !token.is_empty() && token.len() <= 4 && token.chars().all(|c| c.is_ascii_digit()) && !is_year(token)
}

/// Candidate terms in one file: single words plus pairs of neighbouring words.
fn file_terms(path: &Path) -> HashSet<String> {
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let text = normalise(&format!("{stem}\n{}", extract_text(path)));
    let tokens: Vec<&str> =
        text.split(' ').map(|t| t.trim_matches(|c: char| !c.is_alphanumeric())).filter(|t| !t.is_empty()).collect();
    let mut terms = HashSet::new();
    for (i, token) in tokens.iter().enumerate() {
        if !is_word(token) {
            continue;
        }
        terms.insert(token.to_string());
        if let Some(next) = tokens.get(i + 1)
            && (is_word(next) || is_code_number(next))
        {
            terms.insert(format!("{token} {next}"));
        }
    }
    terms
}

/// In how many of `files` each term appears.
fn document_frequency(files: &[PathBuf]) -> HashMap<String, usize> {
    let mut df = HashMap::new();
    for file in files {
        for term in file_terms(file) {
            *df.entry(term).or_insert(0) += 1;
        }
    }
    df
}

fn course_terms(course: &Course) -> Vec<String> {
    std::iter::once(&course.name).chain(&course.keywords).map(|t| normalise(t)).filter(|t| !t.is_empty()).collect()
}

/// Suggest new keywords for `config.courses[index]` from the files in its folder.
pub fn suggest_keywords(config: &Config, index: usize) -> Suggestion {
    let course = &config.courses[index];
    let folder = config.course_folder(course);
    let own_files = module_files(&folder, config, MAX_OWN_FILES);
    let mut suggestion = Suggestion { keywords: Vec::new(), files_scanned: own_files.len(), folder: folder.clone() };
    if own_files.is_empty() {
        return suggestion;
    }
    let own_df = document_frequency(&own_files);

    // What the other modules' files and settings look like.
    let mut other_files = Vec::new();
    let mut other_terms = Vec::new();
    for (i, other) in config.courses.iter().enumerate() {
        if i == index {
            continue;
        }
        other_terms.extend(course_terms(other));
        let other_folder = config.course_folder(other);
        if other_folder != folder {
            other_files.extend(module_files(&other_folder, config, MAX_OTHER_FILES));
        }
    }
    let other_df = document_frequency(&other_files);
    let existing = course_terms(course);

    // Existing keywords count as already chosen, with how often they occur in this module's files.
    let existing_df: Vec<(String, usize)> =
        existing.iter().map(|t| (t.clone(), *own_df.get(t).unwrap_or(&0))).collect();

    let n_own = own_files.len() as f64;
    let n_other = other_files.len().max(1) as f64;
    let min_df = if own_files.len() == 1 { 1 } else { 2 };
    let mut ranked: Vec<(f64, usize, String)> = own_df
        .into_iter()
        .filter(|(_, df)| *df >= min_df)
        .filter_map(|(term, df)| {
            let other_share = *other_df.get(&term).unwrap_or(&0) as f64 / n_other;
            if other_share > MAX_OTHER_SHARE {
                return None;
            }
            // Part of another module's name or keywords: would cause ties.
            if other_terms.iter().any(|t| count_hits(t, &term) > 0 || count_hits(&term, t) > 0) {
                return None;
            }
            // Already covered: an existing keyword matches wherever this term does.
            if existing.iter().any(|t| count_hits(&term, t) > 0) {
                return None;
            }
            Some((df as f64 / n_own - other_share, df, term))
        })
        .collect();
    let words = |t: &str| t.split(' ').count();
    ranked.sort_by(|a, b| {
        b.0.total_cmp(&a.0).then(b.1.cmp(&a.1)).then(words(&b.2).cmp(&words(&a.2))).then(a.2.cmp(&b.2))
    });

    let mut chosen: Vec<(String, usize)> = Vec::new();
    for (_, df, term) in ranked {
        let redundant = chosen.iter().chain(&existing_df).any(|(picked, picked_df)| {
            // A phrase containing a chosen word adds nothing; a word inside a chosen phrase
            // only adds something if it appears in more files than the phrase.
            count_hits(&term, picked) > 0 || (count_hits(picked, &term) > 0 && df <= *picked_df)
        });
        if !redundant {
            chosen.push((term, df));
        }
        if chosen.len() == MAX_SUGGESTIONS {
            break;
        }
    }
    suggestion.keywords = chosen.into_iter().map(|(t, _)| t).collect();
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
    fn finds_module_code_lecturer_and_topic() {
        let t = tree(false);
        let s = suggest_keywords(&t.config, 0);
        assert_eq!(s.files_scanned, 3);
        for expected in ["inf 101", "rekursion", "huber"] {
            assert!(s.keywords.contains(&expected.to_string()), "{expected} missing from {:?}", s.keywords);
        }
        for unwanted in ["vorlesung", "integral", "inf", "uebungsblatt", "schleifen", "prof huber"] {
            assert!(!s.keywords.contains(&unwanted.to_string()), "{unwanted} in {:?}", s.keywords);
        }
    }

    #[test]
    fn terms_shared_with_other_modules_are_dropped() {
        let t = tree(true);
        let s = suggest_keywords(&t.config, 0);
        assert!(!s.keywords.contains(&"huber".to_string()), "{:?}", s.keywords);
        assert!(s.keywords.contains(&"rekursion".to_string()));
    }

    #[test]
    fn running_twice_adds_nothing_new() {
        let mut t = tree(false);
        let first = suggest_keywords(&t.config, 0);
        assert!(!first.keywords.is_empty());
        t.config.courses[0].keywords.extend(first.keywords);
        let second = suggest_keywords(&t.config, 0);
        assert!(second.keywords.is_empty(), "second run suggested {:?}", second.keywords);
    }

    #[test]
    fn existing_keywords_are_not_suggested_again() {
        let mut t = tree(false);
        t.config.courses[0].keywords = vec!["Rekursion".into(), "INF101".into()];
        let s = suggest_keywords(&t.config, 0);
        assert!(!s.keywords.iter().any(|k| k == "rekursion" || k == "inf 101"), "{:?}", s.keywords);
        assert!(s.keywords.contains(&"huber".to_string()));
    }

    #[test]
    fn other_module_names_block_terms() {
        let t = tree(false);
        // "integral" only appears in Analysis II's files; nothing containing "analysis" is suggested.
        let s = suggest_keywords(&t.config, 1);
        assert!(s.keywords.contains(&"integral".to_string()), "{:?}", s.keywords);
        assert!(!s.keywords.iter().any(|k| k.contains("analysis")), "{:?}", s.keywords);
    }

    #[test]
    fn suggestions_make_new_files_match() {
        let mut t = tree(false);
        let s = suggest_keywords(&t.config, 0);
        t.config.courses[0].keywords.extend(s.keywords);
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
        assert!(s.keywords.iter().any(|k| k.contains("grenzwerte") || k.contains("folgen")), "{:?}", s.keywords);
        assert!(!s.keywords.iter().any(|k| k == "2026"), "{:?}", s.keywords);
    }
}
