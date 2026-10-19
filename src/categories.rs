//! Subfolders inside a module: lecture material, exercises, solutions and exams, told
//! apart by words in the file name (or, if it says nothing, the start of the document).

use crate::matcher::normalise;

/// The kinds of files that get their own subfolder, in the order of
/// `Features::subfolder_names`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
pub fn classify(stem: &str, text: Option<&str>) -> Option<Category> {
    let name = normalise(stem);
    if let Some(category) = of_words(name.split(' ')) {
        return Some(category);
    }
    let start = normalise(text?);
    of_words(start.split(' ').take(TITLE_WORDS))
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
    fn folder_names_are_made_safe() {
        assert_eq!(safe_folder_name(" Übungen "), "Übungen");
        assert_eq!(safe_folder_name("a/b\\c"), "a b c");
        assert_eq!(safe_folder_name(".."), "");
        assert_eq!(safe_folder_name("Prüfungen."), "Prüfungen");
    }
}
