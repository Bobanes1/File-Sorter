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

/// How many words at the start of a document are its title when the name says nothing.
const TITLE_WORDS: usize = 12;
/// How far the title may reach after a header («Universität Zürich · Institut für … · Prof.
/// …»): words here still count, but less.
const HEAD_WORDS: usize = 40;
/// How much of the text is looked through for signs of its kind.
const BODY_WORDS: usize = 400;

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

/// The kind a name says (lecture, exercise, solution, exam), if any.
pub fn kind_of(name: &str) -> Option<Category> {
    of_words(normalise(name).split(' '))
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

/// The kind a document's text shows beyond its first words: a title after a header, numbered
/// tasks and «Zeigen Sie …» (an exercise sheet), «Lösung» (solutions), «Bearbeitungszeit»,
/// «Hilfsmittel» and points (an exam), «Definition 3.1», «Satz 3.2», «Beweis», «Kapitel 3» (lecture
/// material). Only a clear lead counts (at least 4 points and twice the next): a single «Klausur»
/// in a course description, or «die nächste Übung» on a slide, doesn't decide.
fn kind_of_text(normalised: &str) -> Option<Category> {
    use Category::*;
    let words: Vec<&str> = normalised.split_whitespace().take(BODY_WORDS).collect();
    let number = |i: usize| words.get(i).is_some_and(|w| w.chars().all(|c| c.is_ascii_digit()));
    let mut tasks = std::collections::BTreeSet::new();
    let (mut lecture, mut proofs, mut outline) = (0, 0, 0);
    let (mut asks, mut abgabe, mut exam, mut exam_points, mut solutions) = (0, 0, 0, 0, 0);
    for (i, word) in words.iter().enumerate() {
        let next = words.get(i + 1).copied().unwrap_or("");
        match *word {
            "aufgabe" | "exercise" | "problem" | "task" | "exercice" | "esercizio" if number(i + 1) => {
                tasks.insert(next);
            }
            "definition" | "satz" | "theorem" | "lemma" | "korollar" | "corollary" | "kapitel" | "chapter"
                if number(i + 1) =>
            {
                lecture += 2;
            }
            "beweis" | "proof" => proofs += 1,
            "zeigen" | "berechnen" | "bestimmen" | "beweisen" | "begrunden" | "skizzieren" if next == "sie" => {
                asks += 1
            }
            "show" | "prove" if next == "that" => asks += 1,
            "abgabe" | "due" => abgabe = 1,
            "bearbeitungszeit" | "prufungsdauer" => exam += 3,
            "hilfsmittel" | "matrikelnummer" | "matrikelnr" => exam += 2,
            "punkte" | "points" | "pkt" | "punkt" if i > 0 && number(i - 1) => exam_points += 1,
            "inhalt" | "outline" | "agenda" | "lernziele" | "zusammenfassung" => outline = 1,
            _ => {}
        }
        if i >= TITLE_WORDS && SOLUTION_PARTS.iter().any(|s| word.contains(s)) {
            solutions += 1;
        }
    }
    let mut points = [0u32; 4];
    // A title after a header: its strongest kind (a solution beats an exam, …, as in a name),
    // more if it is numbered like a title («Übungsblatt 4», «Serie 5»; not «eine Klausur am …»).
    if let Some(kind) = of_words(words.iter().take(HEAD_WORDS).skip(TITLE_WORDS).copied()) {
        let numbered = (TITLE_WORDS..HEAD_WORDS.min(words.len()))
            .any(|i| of_word(words[i]) == Some(kind) && (number(i + 1) || number(i.wrapping_sub(1))));
        points[kind as usize] += if numbered { 4 } else { 2 };
    }
    points[Exercise as usize] += if tasks.len() >= 2 { 3 } else { 0 } + asks.min(3) + abgabe;
    points[Lecture as usize] += lecture.min(6) + proofs.min(2) + outline;
    points[Exam as usize] += exam.min(6) + exam_points.min(3);
    points[Solution as usize] += (2 * solutions).min(4);
    let mut ranked: Vec<(Category, u32)> = [Lecture, Exercise, Solution, Exam].map(|k| (k, points[k as usize])).into();
    ranked.sort_by_key(|(_, p)| std::cmp::Reverse(*p));
    let ((kind, best), (_, second)) = (ranked[0], ranked[1]);
    (best >= 4 && best >= 2 * second).then_some(kind)
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
                && !same_name(n, versions_folder)
                && !same_name(n, crate::archive::ARCHIVE_FOLDER)
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
/// name says nothing, the same on the start of the text (`text`, read only then). (With Laya
/// on, it decides instead: `sorter::subfolder_for`.)
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
    let here =
        |folder: &str| list.iter().any(|f| same_name(&f.name, folder)) || existing.iter().any(|e| same_name(e, folder));
    let by_learned = |text: &str| crate::subfolder_learn::best_folder(text, learned, here);
    let by_kind = |category: Category| list.iter().find(|f| f.builtin == Some(category)).map(|f| f.name.clone());
    if let Some(folder) = by_words(&name).or_else(|| by_learned(stem)) {
        return Some(folder);
    }
    if let Some(category) = of_words(name.split(' ')) {
        return by_kind(category);
    }
    let listed = |dir: &String| list.iter().any(|f| same_name(&f.name, dir));
    if let Some(dir) = existing.iter().find(|dir| !listed(dir) && mentions(&name, &normalise(dir))) {
        return Some(dir.clone());
    }
    let text = normalise(&text()?);
    let start = text.split_whitespace().take(TITLE_WORDS).collect::<Vec<_>>().join(" ");
    let head = text.split_whitespace().take(HEAD_WORDS).collect::<Vec<_>>().join(" ");
    by_words(&head)
        .or_else(|| by_learned(&head))
        .or_else(|| of_words(start.split(' ')).and_then(by_kind))
        .or_else(|| kind_of_text(&text).and_then(by_kind))
}

/// The folders a file can go in, for the AI model: each with the words that say it (as a
/// module is asked with its keywords – the form it was trained on), the list's and the
/// module's own folders.
pub fn subfolder_options(list: &[Subfolder], existing: &[String]) -> Vec<(String, String)> {
    let mut options: Vec<(String, String)> = list
        .iter()
        .map(|f| {
            let builtin: &[&str] = match f.builtin {
                Some(Category::Lecture) => &["Vorlesung", "Folien", "Skript", "Kapitel", "lecture", "slides"],
                Some(Category::Exercise) => {
                    &["Übungsblatt", "Serie", "Aufgabe", "Hausaufgabe", "exercise", "homework", "assignment"]
                }
                Some(Category::Solution) => &["Lösung", "Musterlösung", "solution", "answers"],
                Some(Category::Exam) => &["Klausur", "Prüfung", "Altklausur", "exam"],
                None => &[],
            };
            let words: Vec<String> = builtin.iter().map(|w| w.to_string()).chain(f.keywords.iter().cloned()).collect();
            (f.name.clone(), words.join(", "))
        })
        .collect();
    for dir in existing {
        if !options.iter().any(|(name, _)| same_name(name, dir)) {
            options.push((dir.clone(), String::new()));
        }
    }
    options
}

/// Whether two folder names are the same apart from case (also «Ü» and «ü»; Windows and
/// macOS treat them so).
pub fn same_name(a: &str, b: &str) -> bool {
    a.to_lowercase() == b.to_lowercase()
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
    fn the_ai_model_chooses_among_the_list_and_the_modules_own_folders() {
        let list = crate::config::Subfolder::defaults();
        let options = subfolder_options(&list, &["Projekt".to_string(), "übungen".to_string()]);
        let names: Vec<&str> = options.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, ["Vorlesung", "Übungen", "Lösungen", "Prüfungen", "Projekt"]);
    }

    #[test]
    fn the_text_tells_the_kind_beyond_its_first_words() {
        use Category::*;
        let kind = |text: &str| kind_of_text(&normalise(text));
        let header = "Universität Zürich, Institut für Mathematik, Analysis I, Herbstsemester 2026, Prof. Dr. M. \
                      Muster, Assistenz: A. Beispiel. ";
        let sheet =
            format!("{header}Serie 5. Abgabe bis Freitag. 1. Berechnen Sie f'. 2. Zeigen Sie, dass f stetig ist.");
        assert_eq!(kind(&sheet), Some(Exercise));
        let tasks = "Aufgabe 1 (Folgen). Zeigen Sie, dass a_n konvergiert. Aufgabe 2. Bestimmen Sie den Grenzwert.";
        assert_eq!(kind(tasks), Some(Exercise));
        let solution = format!("{header}Musterlösung zu Blatt 4. Aufgabe 1: Lösung: Da die Folge beschränkt ist, …");
        assert_eq!(kind(&solution), Some(Solution));
        let exam =
            "Analysis I, 2. Februar 2026. Bearbeitungszeit: 120 Minuten. Hilfsmittel: keine. Aufgabe 1 (10 Punkte)";
        assert_eq!(kind(exam), Some(Exam));
        let script = "Folgen und Reihen. Definition 3.1: … Satz 3.2: Jede konvergente Folge ist beschränkt. Beweis: …";
        assert_eq!(kind(script), Some(Lecture));
        // One stray word doesn't decide.
        let slide =
            "Eigenwerte und Eigenvektoren. Definition 6.1 … Satz 6.2 … Beweis … Die nächste Übung ist am Montag.";
        assert_ne!(kind(slide), Some(Exercise));
        let info = "Analysis I, Herbstsemester 2026. Organisatorisches: Sprechstunde nach Vereinbarung, Raum HG G 5, \
                    Kontakt per E-Mail. Bewertung: eine schriftliche Klausur am Ende des Semesters.";
        assert_eq!(kind(info), None);
        // In the folders: a header before «Übungsblatt 4» no longer hides it.
        let list = crate::config::Subfolder::defaults();
        let text = || Some(format!("{header}Übungsblatt 4"));
        assert_eq!(pick_subfolder("scan_0042", &list, &[], &[], text).as_deref(), Some("Übungen"));
        assert_eq!(pick_subfolder("scan_0043", &list, &[], &[], || Some(info.to_string())), None);
    }

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
