//! Subfolders inside a module: lecture material, exercises, solutions and exams, told
//! apart by words in the file name (or, if it says nothing, the start of the document).

use std::path::Path;

use crate::config::{LearnedFolder, Subfolder};
use crate::matcher::normalise;

/// The kinds of files that get their own subfolder, in the order of
/// `Features::subfolder_names`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Category {
    Lecture = 0,
    Exercise = 1,
    Solution = 2,
    Exam = 3,
}

/// Words (normalised: lowercase, no accents) and word beginnings for each kind.
const SOLUTION_PARTS: &[&str] = &["losung", "loesung", "solution", "musterlos"];
const SOLUTION_WORDS: &[&str] = &["lsg", "sol", "solutions", "answers", "answer"];
const EXAM_STARTS: &[&str] = &["klausur", "altklausur", "probeklausur", "prufung", "pruefung", "exam", "midterm"];
const EXAM_WORDS: &[&str] = &["quiz", "endterm"];
const EXERCISE_STARTS: &[&str] =
    &["ubung", "uebung", "aufgabe", "exercise", "assignment", "homework", "hausaufgabe", "tutori", "praktikum"];
const EXERCISE_WORDS: &[&str] = &["blatt", "serie", "sheet", "ue", "ub", "hw", "lab"];
const LECTURE_STARTS: &[&str] = &["vorlesung", "lecture", "folie", "slide", "skript", "script", "kapitel", "chapter"];
const LECTURE_WORDS: &[&str] = &["vl", "handout", "notes", "kap"];

/// How many words at the start of a document are looked at when the name says nothing.
const TITLE_WORDS: usize = 12;

fn of_word(word: &str) -> Option<Category> {
    let starts = |list: &[&str]| list.iter().any(|s| word.starts_with(s));
    if SOLUTION_PARTS.iter().any(|s| word.contains(s)) || SOLUTION_WORDS.contains(&word) {
        Some(Category::Solution)
    } else if starts(EXAM_STARTS) || EXAM_WORDS.contains(&word) {
        Some(Category::Exam)
    } else if starts(EXERCISE_STARTS) || EXERCISE_WORDS.contains(&word) {
        Some(Category::Exercise)
    } else if starts(LECTURE_STARTS) || LECTURE_WORDS.contains(&word) {
        Some(Category::Lecture)
    } else {
        None
    }
}

/// The strongest kind among `words`: a solution beats an exam beats an exercise beats
/// lecture material ("Lösung Übungsblatt 3" is a solution, "Probeklausur Lösungen" too).
fn of_words<'a>(words: impl Iterator<Item = &'a str>) -> Option<Category> {
    let rank = |c: Category| match c {
        Category::Solution => 3,
        Category::Exam => 2,
        Category::Exercise => 1,
        Category::Lecture => 0,
    };
    words.filter_map(of_word).max_by_key(|c| rank(*c))
}

/// The kind of a file, from its name (`stem`, without extension) or else the start of its text.
#[cfg(test)]
pub fn classify(stem: &str, text: Option<&str>) -> Option<Category> {
    let name = normalise(stem);
    if let Some(category) = of_words(name.split(' ')) {
        return Some(category);
    }
    let start = normalise(text?);
    of_words(start.split(' ').take(TITLE_WORDS))
}

/// Whether a (normalised) keyword occurs in normalised text: a whole phrase, or the start of
/// a word ("zusammenfassung" also finds "Zusammenfassungen").
fn mentions(text: &str, keyword: &str) -> bool {
    if keyword.chars().count() < 2 {
        return false;
    }
    if keyword.contains(' ') {
        return crate::matcher::count_hits(text, keyword) > 0;
    }
    text.split(' ').any(|w| w == keyword || keyword.chars().count() >= 3 && w.starts_with(keyword))
}

/// The folders directly in a module that can receive files: everything except hidden
/// folders, the versions folder, an archive and folders of unpacked ZIP files.
pub fn existing_subfolders(module: &Path, versions_folder: &str) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(module) else { return Vec::new() };
    let mut names: Vec<String> = entries
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| {
            !n.starts_with('.')
                && !n.eq_ignore_ascii_case(versions_folder)
                && !n.eq_ignore_ascii_case(crate::archive::ARCHIVE_FOLDER)
                && !module.join(format!("{n}.zip")).exists()
        })
        .collect();
    names.sort();
    names
}

/// The subfolder a file goes into: one whose own words are in the file name, else the one
/// whose learned words are (learned from the files already in the subfolders), else the kind
/// the name says (lecture, exercise, solution, exam), else a folder already in the module
/// whose name is in the file name ("Projekt" for "Projekt_Anleitung.pdf"); and only if the
/// name says nothing, the same on the start of the text (`text`, read only then).
pub fn pick_subfolder(
    stem: &str,
    list: &[Subfolder],
    learned: &[LearnedFolder],
    existing: &[String],
    text: impl FnOnce() -> Option<String>,
) -> Option<String> {
    let name = normalise(stem);
    let by_words = |text: &str| {
        list.iter().find(|f| f.keywords.iter().any(|k| mentions(text, &normalise(k)))).map(|f| f.name.clone())
    };
    // Learned words count for a folder of the list or one this module has.
    let here = |folder: &str| {
        list.iter().any(|f| f.name.eq_ignore_ascii_case(folder))
            || existing.iter().any(|e| e.eq_ignore_ascii_case(folder))
    };
    let by_learned = |text: &str| crate::subfolder_learn::best_folder(text, learned, here);
    let by_kind = |category: Category| list.iter().find(|f| f.builtin == Some(category)).map(|f| f.name.clone());
    if let Some(folder) = by_words(&name).or_else(|| by_learned(stem)) {
        return Some(folder);
    }
    if let Some(category) = of_words(name.split(' ')) {
        return by_kind(category);
    }
    let listed = |dir: &String| list.iter().any(|f| f.name.eq_ignore_ascii_case(dir));
    if let Some(dir) = existing.iter().find(|dir| !listed(dir) && mentions(&name, &normalise(dir))) {
        return Some(dir.clone());
    }
    let text = text()?;
    let start = normalise(&text).split(' ').take(TITLE_WORDS).collect::<Vec<_>>().join(" ");
    by_words(&start).or_else(|| by_learned(&start)).or_else(|| of_words(start.split(' ')).and_then(by_kind))
}

/// A folder name the user typed, made safe: no path separators or characters Windows
/// forbids, no dots or spaces at the ends. Empty means "no subfolder".
pub fn safe_folder_name(name: &str) -> String {
    let cleaned: String =
        name.chars().map(|c| if "\\/:*?\"<>|".contains(c) || c.is_control() { ' ' } else { c }).collect();
    cleaned.trim().trim_matches('.').trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use Category::*;

    #[test]
    fn names_tell_the_kind() {
        let cases = [
            ("VL03_Rekursion", Some(Lecture)),
            ("Vorlesung 5 Folien", Some(Lecture)),
            ("Kapitel_2_Skript", Some(Lecture)),
            ("lecture-notes-week4", Some(Lecture)),
            ("Übungsblatt3", Some(Exercise)),
            ("Uebung_04", Some(Exercise)),
            ("Serie 7", Some(Exercise)),
            ("exercise_sheet_2", Some(Exercise)),
            ("Assignment1", Some(Exercise)),
            ("Lösung Übungsblatt 3", Some(Solution)),
            ("Musterlösung_Serie7", Some(Solution)),
            ("Loesungen_Blatt2", Some(Solution)),
            ("solutions-hw3", Some(Solution)),
            ("Altklausur_HS2024", Some(Exam)),
            ("Probeklausur Lösungen", Some(Solution)),
            ("Prüfung FS25", Some(Exam)),
            ("midterm2023", Some(Exam)),
            ("INF101 Organisatorisches", None),
            ("Literaturliste", None),
        ];
        for (name, expected) in cases {
            assert_eq!(classify(name, None), expected, "{name}");
        }
    }

    #[test]
    fn the_title_decides_when_the_name_does_not() {
        assert_eq!(classify("scan_0042", Some("Übungsblatt 3\nAbgabe bis 24.10.")), Some(Exercise));
        assert_eq!(classify("scan_0042", Some("Einführung in die Programmierung, Vorlesung 1")), Some(Lecture));
        let far = format!("{} Klausur", "wort ".repeat(30));
        assert_eq!(classify("scan_0042", Some(&far)), None, "only the start counts");
        assert_eq!(classify("Blatt 2", Some("Musterlösung")), Some(Exercise), "the name wins");
    }

    #[test]
    fn subfolders_by_own_words_kind_and_existing_folders() {
        let mut list = Subfolder::defaults();
        list.push(Subfolder {
            name: "Zusammenfassungen".into(),
            keywords: vec!["Zusammenfassung".into(), "cheat sheet".into()],
            builtin: None,
        });
        let existing = vec!["Projekt".into(), "Vorlesung".into(), "Bilder".into()];
        let pick = |stem: &str| pick_subfolder(stem, &list, &[], &existing, || None);
        assert_eq!(pick("Zusammenfassung VL3").as_deref(), Some("Zusammenfassungen"), "own words first");
        assert_eq!(pick("stats cheat sheet").as_deref(), Some("Zusammenfassungen"));
        assert_eq!(pick("VL03_Rekursion").as_deref(), Some("Vorlesung"));
        assert_eq!(pick("Musterlösung Serie 7").as_deref(), Some("Lösungen"));
        assert_eq!(pick("Projekt_Anleitung").as_deref(), Some("Projekt"), "a folder the user made");
        assert_eq!(pick("Organisatorisches"), None);
        let from_text = pick_subfolder("scan_0042", &list, &[], &existing, || Some("Übungsblatt 3".into()));
        assert_eq!(from_text.as_deref(), Some("Übungen"));
        // A kind whose folder was removed from the list: no subfolder.
        let without: Vec<_> = list.iter().filter(|f| f.builtin != Some(Lecture)).cloned().collect();
        assert_eq!(pick_subfolder("VL03", &without, &[], &[], || None), None);
    }

    #[test]
    fn learned_words_come_after_own_words_and_before_the_kind() {
        let mut list = Subfolder::defaults();
        list[0].keywords = vec!["Zusammenfassung".into()];
        let learned = vec![
            LearnedFolder { name: "Vorlesung".into(), words: vec!["handout".into()], removed: vec![] },
            // A learned word for «Übungen» that the name «Lösung» would send elsewhere.
            LearnedFolder { name: "Übungen".into(), words: vec!["korrektur".into()], removed: vec![] },
            LearnedFolder { name: "Projekt".into(), words: vec!["meilenstein".into()], removed: vec![] },
        ];
        let pick = |stem: &str, existing: &[String]| pick_subfolder(stem, &list, &learned, existing, || None);
        assert_eq!(pick("Handout Woche 3", &[]).as_deref(), Some("Vorlesung"));
        assert_eq!(pick("Lösung Korrektur", &[]).as_deref(), Some("Übungen"), "learned before the kind");
        assert_eq!(pick("Zusammenfassung Korrektur", &[]).as_deref(), Some("Vorlesung"), "own words first");
        assert_eq!(pick("Meilenstein 2", &[]), None, "no such folder here");
        assert_eq!(pick("Meilenstein 2", &["Projekt".into()]).as_deref(), Some("Projekt"));
        let from_text = pick_subfolder("scan_0042", &list, &learned, &[], || Some("Handout zur Woche 4".into()));
        assert_eq!(from_text.as_deref(), Some("Vorlesung"));
    }

    #[test]
    fn only_real_subfolders_count_as_existing() {
        let tmp = tempfile::tempdir().unwrap();
        for dir in ["Projekt", ".git", "Ältere Versionen", "Archiv", "Blatt3"] {
            std::fs::create_dir(tmp.path().join(dir)).unwrap();
        }
        std::fs::write(tmp.path().join("Blatt3.zip"), "zip").unwrap();
        std::fs::write(tmp.path().join("notes.pdf"), "x").unwrap();
        assert_eq!(existing_subfolders(tmp.path(), "Ältere Versionen"), ["Projekt"]);
    }

    #[test]
    fn folder_names_are_made_safe() {
        assert_eq!(safe_folder_name(" Übungen "), "Übungen");
        assert_eq!(safe_folder_name("a/b\\c"), "a b c");
        assert_eq!(safe_folder_name(".."), "");
        assert_eq!(safe_folder_name("Prüfungen."), "Prüfungen");
    }
}
