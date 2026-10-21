//! The simple rules that sort while the AI model Laya isn't there: which module's name and
//! keywords are in a file's name and text.

use unicode_normalization::UnicodeNormalization;
use unicode_normalization::char::is_combining_mark;

use crate::config::Course;

/// Why a file was matched to a module, with the keywords that matched.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum Reason {
    FileName(Vec<String>),
    Content(Vec<String>),
    /// The user chose the module.
    Chosen,
    /// Earlier versions only (still in the history): the download page, the module's other
    /// files, the timetable.
    Origin(String),
    Learned(Vec<String>),
    Timetable(String),
    /// The AI model (Laya) read the document and is this sure (percent).
    Ai(u8),
}

impl Reason {
    pub fn keywords(&self) -> &[String] {
        match self {
            Reason::FileName(k) | Reason::Content(k) | Reason::Learned(k) => k,
            Reason::Chosen | Reason::Origin(_) | Reason::Timetable(_) | Reason::Ai(_) => &[],
        }
    }

    /// English description, for the log file.
    pub fn describe(&self) -> String {
        match self {
            Reason::FileName(k) => format!("filename contains {}", k.join(", ")),
            Reason::Content(k) => format!("content mentions {}", k.join(", ")),
            Reason::Chosen => "chosen by hand".to_string(),
            Reason::Origin(host) => format!("downloaded from {host} like earlier files of this module"),
            Reason::Learned(k) => format!("like the module's other files ({})", k.join(", ")),
            Reason::Timetable(m) => format!("a tie; the lesson of {m} was running"),
            Reason::Ai(p) => format!("the AI model read it, {p}% sure"),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Match {
    /// Index into the course list that was passed in.
    pub course: usize,
    pub score: usize,
    pub reason: Reason,
}

/// Lowercase, strip accents and turn separators like `_ - .` into spaces.
///
/// Letters and digits are split apart ("INF101" → "inf 101"), so a module code
/// matches however it is written: "INF101", "INF-101" or "INF 101".
pub fn normalise(text: &str) -> String {
    let lowered: String = text.nfkd().filter(|c| !is_combining_mark(*c)).collect::<String>().to_lowercase();
    let mut spaced = String::with_capacity(lowered.len());
    let mut prev: Option<char> = None;
    for c in lowered.chars() {
        let c = if "_-.()[]{},;:/\\".contains(c) { ' ' } else { c };
        if let Some(p) = prev
            && ((p.is_alphabetic() && c.is_ascii_digit()) || (p.is_ascii_digit() && c.is_alphabetic()))
        {
            spaced.push(' ');
        }
        spaced.push(c);
        prev = Some(c);
    }
    spaced.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Count whole-word occurrences of an already normalised keyword in normalised text.
///
/// Word boundaries so "ai" doesn't match "maintain" and "inf 101" doesn't match
/// "inf 1011". A plural "s" is allowed ("algorithm" matches "algorithms"). The
/// keyword "prog" still matches "prog1", because normalising splits it into "prog 1".
pub(crate) fn count_hits(text: &str, keyword: &str) -> usize {
    if keyword.is_empty() {
        return 0;
    }
    let bytes = text.as_bytes();
    let is_word_char = |i: usize| bytes.get(i).is_some_and(u8::is_ascii_alphanumeric);
    let mut hits = 0;
    let mut start = 0;
    while let Some(pos) = text[start..].find(keyword) {
        let begin = start + pos;
        let mut end = begin + keyword.len();
        let boundary_before = begin == 0 || !bytes[begin - 1].is_ascii_alphanumeric();
        if bytes.get(end) == Some(&b's') && !is_word_char(end + 1) {
            end += 1;
        }
        if boundary_before && !is_word_char(end) {
            hits += 1;
            start = end;
        } else {
            // Step past the first character of this occurrence (respecting UTF-8 boundaries).
            start = begin + text[begin..].chars().next().map_or(1, char::len_utf8);
        }
    }
    hits
}

/// The module name plus its keywords, without duplicates (compared in normalised form).
fn terms(course: &Course) -> Vec<(String, String)> {
    let mut seen = Vec::<String>::new();
    let mut out = Vec::new();
    for term in std::iter::once(&course.name).chain(course.keywords.iter()) {
        let key = normalise(term);
        if !key.is_empty() && !seen.contains(&key) {
            seen.push(key.clone());
            out.push((term.clone(), key));
        }
    }
    out
}

/// Whether `a` and `b` differ by one typo: a letter more, less, different, or two swapped.
pub fn one_typo(a: &str, b: &str) -> bool {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    if a == b || a.len().abs_diff(b.len()) > 1 {
        return false;
    }
    let mut i = 0;
    while i < a.len().min(b.len()) && a[i] == b[i] {
        i += 1;
    }
    let rest = |x: &[char], y: &[char]| x == y;
    match a.len().cmp(&b.len()) {
        std::cmp::Ordering::Equal => {
            rest(&a[i + 1..], &b[i + 1..])
                || (i + 1 < a.len() && a[i] == b[i + 1] && a[i + 1] == b[i] && rest(&a[i + 2..], &b[i + 2..]))
        }
        std::cmp::Ordering::Greater => rest(&a[i + 1..], &b[i..]),
        std::cmp::Ordering::Less => rest(&a[i..], &b[i + 1..]),
    }
}

/// Points for a word in the file name: twice what it can get from the text (title and body
/// together), so the name weighs most.
const IN_NAME: usize = 100;
/// … on the title page (the first words of the text).
const ON_TITLE: usize = 20;
/// … each time further in, up to `BODY_HITS` times.
const IN_BODY: usize = 10;
const BODY_HITS: usize = 3;
/// How many words count as the title page.
const TITLE_WORDS: usize = 60;
/// How far the best module must be ahead of the next one to be clear: 1.5 times.
const LEAD: (usize, usize) = (3, 2);

/// What the name and text say about a file's module.
#[derive(Debug, Clone, PartialEq)]
pub enum Decision {
    /// One module is clearly ahead.
    Clear(Match),
    /// Several modules fit about equally well: their indices, best first.
    Unsure(Vec<usize>),
    /// No module's words are in it.
    Nothing,
}

/// The module of a file by its name (`name`, without the extension) and the start of its text.
///
/// Each module's name and keywords are looked for as whole words: in the name they count
/// most, on the title page less, further in least. A word several modules share counts that
/// much less. The best module wins when it is 1.5 times ahead of the next; else the user is
/// asked between the ones that are close.
pub fn decide(name: &str, text: &str, courses: &[Course]) -> Decision {
    let all: Vec<Vec<(String, String)>> = courses.iter().map(terms).collect();
    let shared = |key: &str| all.iter().filter(|t| t.iter().any(|(_, k)| k == key)).count().max(1);
    let name = normalise(name);
    let text = normalise(text);
    let title_end = text.match_indices(' ').nth(TITLE_WORDS).map_or(text.len(), |(p, _)| p);
    let title = &text[..title_end];
    // Per module: (points, words found, any of them in the name).
    let scores: Vec<(usize, Vec<String>, bool)> = all
        .iter()
        .map(|terms| {
            let mut points = 0;
            let mut found = Vec::new();
            let mut by_name = false;
            for (term, key) in terms {
                let mut here = 0;
                if count_hits(&name, key) > 0 {
                    here += IN_NAME;
                    by_name = true;
                }
                if count_hits(title, key) > 0 {
                    here += ON_TITLE;
                }
                here += count_hits(&text, key).min(BODY_HITS) * IN_BODY;
                if here > 0 {
                    points += here / shared(key);
                    found.push(term.clone());
                }
            }
            (points, found, by_name)
        })
        .collect();
    let mut order: Vec<usize> = (0..courses.len()).filter(|&i| scores[i].0 > 0).collect();
    order.sort_by(|&a, &b| scores[b].0.cmp(&scores[a].0).then(a.cmp(&b)));
    let Some(&best) = order.first() else { return Decision::Nothing };
    let points = scores[best].0;
    let close: Vec<usize> = order.iter().copied().filter(|&i| scores[i].0 * LEAD.0 > points * LEAD.1).collect();
    if close.len() > 1 {
        return Decision::Unsure(close.into_iter().take(3).collect());
    }
    let (_, found, by_name) = scores[best].clone();
    let reason = if by_name { Reason::FileName(found) } else { Reason::Content(found) };
    Decision::Clear(Match { course: best, score: points, reason })
}

/// The module of a file whose text `extract` reads, if one is clear.
#[cfg(test)]
pub fn match_file(
    path: &std::path::Path,
    courses: &[Course],
    extract: impl FnOnce(&std::path::Path) -> String,
) -> Option<Match> {
    let name = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    match decide(&name, &extract(path), courses) {
        Decision::Clear(m) => Some(m),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn courses() -> Vec<Course> {
        vec![
            Course::new("Programming Course", &["programming", "python", "prog1", "algorithm"]),
            Course::new("Linear Algebra", &["linear algebra", "matrix", "LA1"]),
            Course::new("Statistics", &["statistics", "probability", "stats"]),
        ]
    }

    fn no_content(_: &Path) -> String {
        String::new()
    }

    fn name_of(m: Option<Match>, courses: &[Course]) -> Option<String> {
        m.map(|m| courses[m.course].name.clone())
    }

    /// Hits of a module's words in a text (any weight).
    fn hits(text: &str, course: &Course) -> usize {
        let text = normalise(text);
        terms(course).iter().map(|(_, key)| count_hits(&text, key)).sum()
    }

    #[test]
    fn normalise_strips_separators_and_accents() {
        assert_eq!(normalise("Analysis_Übung-03.Final"), "analysis ubung 03 final");
        assert_eq!(normalise("INF101_Blatt5"), "inf 101 blatt 5");
        assert_eq!(normalise("WS2026/27"), "ws 2026 27");
        assert_eq!(normalise("  Einführung   in (C++) "), "einfuhrung in c++");
    }

    #[test]
    fn filename_keyword_matches() {
        let c = courses();
        let m = match_file(Path::new("Prog1_Lecture03.pdf"), &c, no_content);
        assert_eq!(name_of(m.clone(), &c).as_deref(), Some("Programming Course"));
        assert!(matches!(m.unwrap().reason, Reason::FileName(_)));
    }

    #[test]
    fn multiword_keyword_and_plural() {
        let c = courses();
        let m = match_file(Path::new("linear-algebra-sheet2.pdf"), &c, no_content);
        assert_eq!(name_of(m, &c).as_deref(), Some("Linear Algebra"));
        let m = match_file(Path::new("sorting algorithms.pdf"), &c, no_content);
        assert_eq!(name_of(m, &c).as_deref(), Some("Programming Course"));
    }

    #[test]
    fn keyword_needs_word_boundary() {
        let ai = Course::new("AI", &["ai"]);
        assert_eq!(hits("maintain the system", &ai), 0);
        assert_eq!(hits("intro to ai lecture", &ai), 1);
        assert_eq!(hits("ai2 and ais", &ai), 2);
        assert_eq!(hits("aisle", &ai), 0);
        let prog = Course::new("P", &["prog"]);
        assert_eq!(hits("prog1 progress", &prog), 1);
    }

    #[test]
    fn module_codes_match_however_written() {
        let c = Course::new("P", &["INF-101"]);
        for name in ["INF101_Blatt5", "inf 101 folien", "Uebung_INF-101", "INF_101"] {
            assert_eq!(hits(name, &c), 1, "{name}");
        }
        assert_eq!(hits("INF1011", &c), 0);
        let prog = Course::new("Q", &["prog1"]);
        assert_eq!(hits("Prog 1 Woche 3", &prog), 1);
    }

    #[test]
    fn roman_numerals_stay_apart() {
        let one = Course::new("Analysis I", &[]);
        assert_eq!(hits("Analysis_II_Blatt3", &one), 0);
        assert_eq!(hits("Analysis_I_Blatt3", &one), 1);
    }

    #[test]
    fn course_name_counts_as_keyword() {
        let c = vec![Course::new("Statistics", &[])];
        assert!(match_file(Path::new("Statistics Lecture 1.pdf"), &c, no_content).is_some());
    }

    #[test]
    fn nothing_found_and_a_tie() {
        let c = courses();
        let holiday = decide("holiday photos", "beach and sun", &c);
        assert_eq!(holiday, Decision::Nothing);
        // One word of two modules each in the name, and nothing in the text: ask.
        assert_eq!(decide("python matrix", "", &c), Decision::Unsure(vec![0, 1]));
    }

    #[test]
    fn content_used_when_filename_is_generic() {
        let c = courses();
        let text = "Lecture 5: Linear Algebra\nEigenvalues of a matrix";
        let m = match_file(Path::new("lecture5.pdf"), &c, |_| text.into()).unwrap();
        assert_eq!(c[m.course].name, "Linear Algebra");
        assert!(matches!(m.reason, Reason::Content(_)));
        assert!(m.reason.describe().starts_with("content mentions"));
    }

    #[test]
    fn content_breaks_filename_tie() {
        let c = courses();
        let m = match_file(Path::new("python matrix.pdf"), &c, |_| "Introduction to Programming with Python".into());
        assert_eq!(name_of(m, &c).as_deref(), Some("Programming Course"));
    }

    #[test]
    fn the_name_weighs_more_than_the_text() {
        let c = courses();
        // «Statistics» in the name; the text mentions matrices many times.
        let text = format!("matrix {}", "matrix ".repeat(40));
        let m = match_file(Path::new("Statistics_Sheet3.pdf"), &c, |_| text).unwrap();
        assert_eq!(c[m.course].name, "Statistics");
    }

    #[test]
    fn repeated_content_word_is_capped() {
        let c = vec![Course::new("A", &["alpha"]), Course::new("B", &["beta", "gamma", "delta"])];
        let text = format!("{}beta gamma delta beta gamma delta", "alpha ".repeat(50));
        let m = match_file(Path::new("notes.pdf"), &c, |_| text).unwrap();
        assert_eq!(c[m.course].name, "B");
    }

    #[test]
    fn shared_keywords_count_less_and_the_title_page_more() {
        let c = vec![
            Course::new("Programmieren", &["informatik", "java"]),
            Course::new("Datenbanken", &["informatik", "sql"]),
        ];
        // «informatik» is in both modules, «java» only in one: java decides.
        let text = format!("Informatik Informatik Informatik {} java java", "wort ".repeat(80));
        let m = match_file(Path::new("x.pdf"), &c, |_| text).unwrap();
        assert_eq!(m.course, 0);
        // The same word on the title page counts more than further in.
        let text = format!("SQL Einführung {} java", "wort ".repeat(80));
        assert_eq!(match_file(Path::new("x.pdf"), &c, |_| text).unwrap().course, 1);
    }

    #[test]
    fn a_small_lead_is_unsure() {
        let c = courses();
        // python (programming) three times, matrix (linear algebra) once: clearly ahead.
        assert!(matches!(decide("notes", "python python python matrix", &c), Decision::Clear(m) if m.course == 0));
        // Once each: ask between the two.
        assert_eq!(decide("notes", "python matrix", &c), Decision::Unsure(vec![0, 1]));
    }

    #[test]
    fn typos_are_told_apart() {
        assert!(one_typo("statistik", "statisitk") && one_typo("eigenwerte", "eigenwert") && !one_typo("abc", "abc"));
        assert!(!one_typo("statistik", "stochastik"));
    }
}
