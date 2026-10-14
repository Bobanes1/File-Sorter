//! Decide which module a file belongs to by scoring keyword hits.

use std::path::Path;

use unicode_normalization::UnicodeNormalization;
use unicode_normalization::char::is_combining_mark;

use crate::config::Course;

const FILENAME_WEIGHT: usize = 10;
/// A keyword repeated 50 times in a PDF shouldn't outweigh several different keywords.
const MAX_CONTENT_HITS_PER_KEYWORD: usize = 5;

#[derive(Debug, Clone, PartialEq)]
pub struct Match {
    /// Index into the course list that was passed in.
    pub course: usize,
    pub score: usize,
    pub reason: String,
}

/// Lowercase, strip accents and turn separators like `_ - .` into spaces.
pub fn normalise(text: &str) -> String {
    let lowered: String = text.nfkd().filter(|c| !is_combining_mark(*c)).collect::<String>().to_lowercase();
    let spaced: String = lowered.chars().map(|c| if "_-.()[]{},;:/\\".contains(c) { ' ' } else { c }).collect();
    spaced.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Count whole-word occurrences of an already normalised keyword in normalised text.
///
/// Word boundaries so "ai" doesn't match "maintain". A plural "s" is allowed
/// ("algorithm" matches "algorithms") and digits may follow directly, so the
/// keyword "prog" still matches "prog1".
fn count_hits(text: &str, keyword: &str) -> usize {
    if keyword.is_empty() {
        return 0;
    }
    let bytes = text.as_bytes();
    let is_letter = |i: usize| bytes.get(i).is_some_and(u8::is_ascii_lowercase);
    let mut hits = 0;
    let mut start = 0;
    while let Some(pos) = text[start..].find(keyword) {
        let begin = start + pos;
        let mut end = begin + keyword.len();
        let boundary_before = begin == 0 || !bytes[begin - 1].is_ascii_alphanumeric();
        if bytes.get(end) == Some(&b's') && !is_letter(end + 1) {
            end += 1;
        }
        if boundary_before && !is_letter(end) {
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

/// Keyword hits for one module in `text`. Returns (hits, matched keywords).
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

/// Index and score of the single best module, or None if nothing matched or there is a tie.
fn best(scores: &[(usize, Vec<String>)]) -> Option<(usize, usize)> {
    let mut ranked: Vec<(usize, usize)> =
        scores.iter().enumerate().filter(|(_, (s, _))| *s > 0).map(|(i, (s, _))| (*s, i)).collect();
    ranked.sort_by(|a, b| b.cmp(a));
    match ranked.as_slice() {
        [] => None,
        [first, second, ..] if first.0 == second.0 => None,
        [first, ..] => Some((first.1, first.0)),
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

/// Return the module the file belongs to, or None to leave it where it is.
///
/// The filename is checked first. File contents are only read (via `extract`)
/// when the filename alone doesn't give one clear winner.
pub fn match_file(
    path: &Path,
    courses: &[Course],
    read_contents: bool,
    extract: impl FnOnce(&Path) -> String,
) -> Option<Match> {
    if courses.is_empty() {
        return None;
    }
    let name = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let name_scores: Vec<_> = courses.iter().map(|c| score_text(&name, c, None)).collect();
    if let Some((idx, _)) = best(&name_scores) {
        let (hits, kw) = &name_scores[idx];
        return Some(Match {
            course: idx,
            score: hits * FILENAME_WEIGHT,
            reason: format!("filename contains {}", kw.join(", ")),
        });
    }
    if !read_contents {
        return None;
    }
    let text = extract(path);
    if text.trim().is_empty() {
        return None;
    }
    let combined: Vec<_> = courses
        .iter()
        .zip(&name_scores)
        .map(|(course, (name_hits, name_kw))| {
            let (content_hits, content_kw) = score_text(&text, course, Some(MAX_CONTENT_HITS_PER_KEYWORD));
            let mut kw = name_kw.clone();
            kw.extend(content_kw);
            (name_hits * FILENAME_WEIGHT + content_hits, kw)
        })
        .collect();
    let (idx, score) = best(&combined)?;
    Some(Match { course: idx, score, reason: format!("content mentions {}", dedup(&combined[idx].1).join(", ")) })
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
        assert!(m.reason.starts_with("content mentions"));
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
    fn content_skipped_when_disabled() {
        assert!(match_file(Path::new("lecture5.pdf"), &courses(), false, no_content).is_none());
    }
}
