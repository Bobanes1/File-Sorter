//! Decide which module a file belongs to by scoring keyword hits.

use std::path::Path;

use unicode_normalization::UnicodeNormalization;
use unicode_normalization::char::is_combining_mark;

use crate::config::Course;

const FILENAME_WEIGHT: usize = 10;
/// A keyword repeated 50 times in a PDF shouldn't outweigh several different keywords.
const MAX_CONTENT_HITS_PER_KEYWORD: usize = 5;

/// Why a file was matched to a module, with the keywords that matched.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum Reason {
    FileName(Vec<String>),
    Content(Vec<String>),
    /// The user chose the module.
    Chosen,
    /// Downloaded from the same place (server named) as earlier files of the module.
    Origin(String),
    /// The keywords didn't decide; the module's other files have these words.
    Learned(Vec<String>),
    /// Several modules fit equally well; this one's lesson was running when the file came.
    Timetable(String),
}

impl Reason {
    pub fn keywords(&self) -> &[String] {
        match self {
            Reason::FileName(k) | Reason::Content(k) | Reason::Learned(k) => k,
            Reason::Chosen | Reason::Origin(_) | Reason::Timetable(_) => &[],
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

/// Short forms of a module name that people write in file names: the starts of its words
/// ("Lineare Algebra" → "linalg", "la"), the first word cut short with its number
/// ("Programmieren 1" → "prog 1").
fn abbreviations(name: &str) -> Vec<String> {
    let words: Vec<String> = normalise(name)
        .split(' ')
        .filter(|w| !w.is_empty() && !crate::config::STOPWORDS.contains(w) && !crate::config::FILLER_WORDS.contains(w))
        .map(str::to_string)
        .collect();
    let letters: Vec<&String> = words.iter().filter(|w| w.chars().all(char::is_alphabetic)).collect();
    let number = words.iter().find(|w| w.chars().all(|c| c.is_ascii_digit() || "iv".contains(c)) && w.len() <= 3);
    let mut out = Vec::new();
    if letters.len() >= 2 {
        let starts: String = letters.iter().map(|w| w.chars().take(3).collect::<String>()).collect();
        out.push(starts);
        let initials: String = letters.iter().filter_map(|w| w.chars().next()).collect();
        if initials.len() >= 2 {
            out.push(initials);
        }
    }
    if let Some(first) = letters.first().filter(|w| w.chars().count() > 5) {
        let short: String = first.chars().take(4).collect();
        out.push(match number {
            Some(n) => format!("{short} {n}"),
            None => short,
        });
    }
    out.retain(|a| a.chars().count() >= 2 && !crate::common_words::is_common(a));
    out.dedup();
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

/// How much each module's words count: a word several modules share counts less, and short
/// forms and typos count half. Built once per decision.
pub struct Weights {
    /// Per module: (word as written, normalised, weight in tenths).
    terms: Vec<Vec<(String, String, usize)>>,
    /// Per module: short forms of its name only it has.
    short: Vec<Vec<String>>,
}

/// Full weight of one hit.
const FULL: usize = 10;
/// A typo or a short form counts half.
const HALF: usize = 5;
/// Content hits within this many words of the start (the title page) count double.
const TITLE_WORDS: usize = 60;

impl Weights {
    pub fn new(courses: &[Course]) -> Weights {
        let all: Vec<Vec<(String, String)>> = courses.iter().map(terms).collect();
        let shared = |key: &str| all.iter().filter(|t| t.iter().any(|(_, k)| k == key)).count().max(1);
        let terms = all
            .iter()
            .map(|t| t.iter().map(|(term, key)| (term.clone(), key.clone(), FULL / shared(key))).collect())
            .collect();
        let names: Vec<Vec<String>> = courses.iter().map(|c| abbreviations(&c.name)).collect();
        let keys: Vec<String> = all.iter().flatten().map(|(_, k)| k.clone()).collect();
        let short = names
            .iter()
            .map(|mine| {
                mine.iter()
                    .filter(|a| names.iter().filter(|n| n.contains(a)).count() == 1 && !keys.contains(a))
                    .cloned()
                    .collect()
            })
            .collect();
        Weights { terms, short }
    }

    /// Weighted hits of module `i` in a file name: whole keywords, short forms of the module
    /// name and keywords with one typo. Returns (score, matched words).
    pub fn score_name(&self, name: &str, i: usize) -> (usize, Vec<String>) {
        let norm = normalise(name);
        let tokens: Vec<&str> = norm.split(' ').collect();
        let mut total = 0;
        let mut matched = Vec::new();
        for (term, key, weight) in &self.terms[i] {
            let hits = count_hits(&norm, key);
            if hits > 0 {
                total += hits * weight;
                matched.push(term.clone());
            } else if !key.contains(' ')
                && key.chars().count() >= 6
                && tokens.iter().any(|t| t.chars().count() >= 5 && one_typo(t, key))
            {
                total += weight * HALF / FULL;
                matched.push(term.clone());
            }
        }
        for short in &self.short[i] {
            if count_hits(&norm, short) > 0 {
                total += HALF;
                matched.push(short.clone());
            }
        }
        (total, matched)
    }

    /// Weighted hits of module `i` in a text, each word at most `cap` times; hits on the title
    /// page count double.
    pub fn score_text(&self, text: &str, i: usize, cap: usize) -> (usize, Vec<String>) {
        let norm = normalise(text);
        let title_end = norm.match_indices(' ').nth(TITLE_WORDS).map_or(norm.len(), |(p, _)| p);
        let title = &norm[..title_end];
        let mut total = 0;
        let mut matched = Vec::new();
        for (term, key, weight) in &self.terms[i] {
            let hits = count_hits(&norm, key).min(cap);
            if hits > 0 {
                let on_title = count_hits(title, key).min(hits);
                total += (hits + on_title) * weight;
                matched.push(term.clone());
            }
        }
        (total, matched)
    }
}

/// Keyword hits for one module in `text`. Returns (hits, matched keywords).
#[cfg(test)]
pub fn score_text(text: &str, course: &Course, cap: Option<usize>) -> (usize, Vec<String>) {
    let norm = normalise(text);
    let mut total = 0;
    let mut matched = Vec::new();
    for (term, key) in terms(course) {
        let mut hits = count_hits(&norm, &key);
        if hits > 0 {
            if let Some(cap) = cap {
                hits = hits.min(cap);
            }
            total += hits;
            matched.push(term);
        }
    }
    (total, matched)
}

/// The modules with any score, best first: (index, score).
fn ranked(scores: &[(usize, Vec<String>)]) -> Vec<(usize, usize)> {
    let mut ranked: Vec<(usize, usize)> =
        scores.iter().enumerate().filter(|(_, (s, _))| *s > 0).map(|(i, (s, _))| (i, *s)).collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    ranked
}

/// The best module, if it is ahead of the second by at least `margin` percent.
fn clear_winner(ranked: &[(usize, usize)], margin: usize) -> Option<(usize, usize)> {
    match ranked {
        [] => None,
        [first] => Some(*first),
        [first, second, ..] if first.1 * 100 >= second.1 * (100 + margin) && first.1 > second.1 => Some(*first),
        _ => None,
    }
}

fn dedup(items: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for item in items {
        if !out.contains(item) {
            out.push(item.clone());
        }
    }
    out
}

/// What the name and content say about a file's module.
#[derive(Debug, Clone, PartialEq)]
pub enum Decision {
    /// One module is clearly ahead.
    Clear(Match),
    /// Several modules are about as likely: (index, score), best first.
    Unsure(Vec<(usize, usize)>),
    /// No module's words are in it.
    Nothing,
}

/// How far ahead (percent) the best module must be to count as clear when the user wants to
/// be asked otherwise; without asking, any lead counts.
pub const UNSURE_MARGIN: usize = 25;

/// Return the module the file belongs to, or None to leave it where it is.
///
/// The filename is checked first. File contents are only read (via `extract`)
/// when the filename alone doesn't give one clear winner.
#[cfg(test)]
pub fn match_file(
    path: &Path,
    courses: &[Course],
    read_contents: bool,
    extract: impl FnOnce(&Path) -> String,
) -> Option<Match> {
    match decide(path, courses, read_contents, 0, extract) {
        Decision::Clear(m) => Some(m),
        _ => None,
    }
}

/// The module of a file by its name, then (if the name gives no clear winner) its content.
/// `margin` is how far ahead (percent) the best module must be to be clear.
pub fn decide(
    path: &Path,
    courses: &[Course],
    read_contents: bool,
    margin: usize,
    extract: impl FnOnce(&Path) -> String,
) -> Decision {
    if courses.is_empty() {
        return Decision::Nothing;
    }
    let weights = Weights::new(courses);
    let name = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let name_scores: Vec<_> = (0..courses.len()).map(|i| weights.score_name(&name, i)).collect();
    let by_name = ranked(&name_scores);
    if let Some((idx, score)) = clear_winner(&by_name, margin) {
        let kw = &name_scores[idx].1;
        return Decision::Clear(Match {
            course: idx,
            score: score * FILENAME_WEIGHT,
            reason: Reason::FileName(kw.clone()),
        });
    }
    let text = if read_contents { extract(path) } else { String::new() };
    if text.trim().is_empty() {
        return if by_name.is_empty() { Decision::Nothing } else { Decision::Unsure(by_name) };
    }
    let combined: Vec<_> = name_scores
        .iter()
        .enumerate()
        .map(|(i, (name_hits, name_kw))| {
            let (content_hits, content_kw) = weights.score_text(&text, i, MAX_CONTENT_HITS_PER_KEYWORD);
            let mut kw = name_kw.clone();
            kw.extend(content_kw);
            (name_hits * FILENAME_WEIGHT + content_hits, kw)
        })
        .collect();
    let all = ranked(&combined);
    match clear_winner(&all, margin) {
        Some((idx, score)) => {
            Decision::Clear(Match { course: idx, score, reason: Reason::Content(dedup(&combined[idx].1)) })
        }
        None if all.is_empty() => Decision::Nothing,
        None => Decision::Unsure(all),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn courses() -> Vec<Course> {
        vec![
            Course::new("Programming Course", &["programming", "python", "prog1", "algorithm"]),
            Course::new("Linear Algebra", &["linear algebra", "matrix", "LA1"]),
            Course::new("Statistics", &["statistics", "probability", "stats"]),
        ]
    }

    fn no_content(_: &Path) -> String {
        panic!("content should not be read when the filename decides")
    }

    fn name_of(m: Option<Match>, courses: &[Course]) -> Option<String> {
        m.map(|m| courses[m.course].name.clone())
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
        let m = match_file(Path::new("Prog1_Lecture03.pdf"), &c, true, no_content);
        assert_eq!(name_of(m, &c).as_deref(), Some("Programming Course"));
    }

    #[test]
    fn multiword_keyword_and_plural() {
        let c = courses();
        let m = match_file(Path::new("linear-algebra-sheet2.pdf"), &c, false, no_content);
        assert_eq!(name_of(m, &c).as_deref(), Some("Linear Algebra"));
        let m = match_file(Path::new("sorting algorithms.pdf"), &c, false, no_content);
        assert_eq!(name_of(m, &c).as_deref(), Some("Programming Course"));
    }

    #[test]
    fn keyword_needs_word_boundary() {
        let ai = Course::new("AI", &["ai"]);
        assert_eq!(score_text("maintain the system", &ai, None).0, 0);
        assert_eq!(score_text("intro to ai lecture", &ai, None).0, 1);
        assert_eq!(score_text("ai2 and ais", &ai, None).0, 2);
        assert_eq!(score_text("aisle", &ai, None).0, 0);
        let prog = Course::new("P", &["prog"]);
        assert_eq!(score_text("prog1 progress", &prog, None).0, 1);
    }

    #[test]
    fn module_codes_match_however_written() {
        let c = Course::new("P", &["INF-101"]);
        for name in ["INF101_Blatt5", "inf 101 folien", "Uebung_INF-101", "INF_101"] {
            assert_eq!(score_text(name, &c, None).0, 1, "{name}");
        }
        assert_eq!(score_text("INF1011", &c, None).0, 0);
        let prog = Course::new("P", &["prog1"]);
        assert_eq!(score_text("Prog 1 Woche 3", &prog, None).0, 1);
    }

    #[test]
    fn roman_numerals_stay_apart() {
        let one = Course::new("Analysis I", &[]);
        assert_eq!(score_text("Analysis_II_Blatt3", &one, None).0, 0);
        assert_eq!(score_text("Analysis_I_Blatt3", &one, None).0, 1);
    }

    #[test]
    fn course_name_counts_as_keyword() {
        let c = vec![Course::new("Statistics", &[])];
        assert!(match_file(Path::new("Statistics Lecture 1.pdf"), &c, false, no_content).is_some());
    }

    #[test]
    fn no_match_and_tie_return_none() {
        let c = courses();
        assert!(match_file(Path::new("holiday photos.pdf"), &c, true, |_| "beach and sun".into()).is_none());
        // One keyword for two modules in the filename, and nothing in the content.
        assert!(match_file(Path::new("python matrix.pdf"), &c, true, |_| String::new()).is_none());
    }

    #[test]
    fn content_used_when_filename_is_generic() {
        let c = courses();
        let text = "Lecture 5: Linear Algebra\nEigenvalues of a matrix";
        let m = match_file(Path::new("lecture5.pdf"), &c, true, |_| text.into()).unwrap();
        assert_eq!(c[m.course].name, "Linear Algebra");
        assert!(matches!(m.reason, Reason::Content(_)));
        assert!(m.reason.describe().starts_with("content mentions"));
    }

    #[test]
    fn content_breaks_filename_tie() {
        let c = courses();
        let m =
            match_file(Path::new("python matrix.pdf"), &c, true, |_| "Introduction to Programming with Python".into());
        assert_eq!(name_of(m, &c).as_deref(), Some("Programming Course"));
    }

    #[test]
    fn repeated_content_word_is_capped() {
        let c = vec![Course::new("A", &["alpha"]), Course::new("B", &["beta", "gamma", "delta"])];
        let text = format!("{}beta gamma delta beta gamma delta", "alpha ".repeat(50));
        let m = match_file(Path::new("notes.pdf"), &c, true, |_| text).unwrap();
        assert_eq!(c[m.course].name, "B");
    }

    #[test]
    fn shared_keywords_count_less_and_the_title_page_more() {
        let c = vec![
            Course::new("Programmieren", &["informatik", "java"]),
            Course::new("Datenbanken", &["informatik", "sql"]),
        ];
        // «informatik» is in both modules, «java» only in one: java decides.
        let text = format!("Informatik Informatik Informatik {} java", "wort ".repeat(80));
        let m = match_file(Path::new("x.pdf"), &c, true, |_| text).unwrap();
        assert_eq!(m.course, 0);
        // The same word on the title page counts more than further in.
        let text = format!("SQL Einführung {} java", "wort ".repeat(80));
        assert_eq!(match_file(Path::new("x.pdf"), &c, true, |_| text).unwrap().course, 1);
    }

    #[test]
    fn typos_and_short_forms_of_the_module_name() {
        let c = vec![
            Course::new("Lineare Algebra", &["eigenwerte"]),
            Course::new("Programmieren 1", &["java"]),
            Course::new("Statistik", &["regression"]),
        ];
        let module = |name: &str| match_file(Path::new(name), &c, false, no_content).map(|m| c[m.course].name.clone());
        assert_eq!(module("LinAlg_Blatt3.pdf").as_deref(), Some("Lineare Algebra"));
        assert_eq!(module("LA Serie 2.pdf").as_deref(), Some("Lineare Algebra"));
        assert_eq!(module("Prog1_Woche4.pdf").as_deref(), Some("Programmieren 1"));
        assert_eq!(module("Statisitk_Uebung.pdf").as_deref(), Some("Statistik"), "one typo");
        assert_eq!(module("Regresion.pdf").as_deref(), Some("Statistik"));
        assert_eq!(module("Stat_Kap2.pdf").as_deref(), Some("Statistik"), "the start of the name");
        assert_eq!(module("St.pdf"), None, "too short for a typo or a short form");
        assert!(one_typo("statistik", "statisitk") && one_typo("eigenwerte", "eigenwert") && !one_typo("abc", "abc"));
        assert!(!one_typo("statistik", "stochastik"));
    }

    #[test]
    fn a_small_lead_is_unsure() {
        let c = courses();
        // python (programming) twice, matrix (linear algebra) once in the content: a lead, but small.
        let text = "python python matrix";
        match decide(Path::new("notes.pdf"), &c, true, UNSURE_MARGIN, |_| text.into()) {
            Decision::Clear(m) => assert_eq!(m.course, 0),
            other => panic!("expected a clear winner, got {other:?}"),
        }
        let text = "python matrix";
        assert!(matches!(
            decide(Path::new("notes.pdf"), &c, true, UNSURE_MARGIN, |_| text.into()),
            Decision::Unsure(_)
        ));
        assert_eq!(decide(Path::new("holiday.pdf"), &c, true, UNSURE_MARGIN, |_| "sun".into()), Decision::Nothing);
    }

    #[test]
    fn content_skipped_when_disabled() {
        assert!(match_file(Path::new("lecture5.pdf"), &courses(), false, no_content).is_none());
    }
}
