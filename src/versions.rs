//! Tell versions of the same file apart: "Skript.pdf", "Skript_v2.pdf", "Skript (1).pdf",
//! "Skript final.pdf" are one document in several versions, while "Blatt 1" and "Blatt 2",
//! "Klausur 2023" and "Klausur 2024" or "Protokoll 24.10.2026" and "Protokoll 31.10.2026"
//! are different documents.

use std::fs;
use std::path::{Path, PathBuf};

use crate::matcher::normalise;

/// Words that only say "this is a newer version".
const VERSION_WORDS: &[&str] = &[
    "final",
    "finale",
    "neu",
    "neue",
    "new",
    "newest",
    "latest",
    "updated",
    "update",
    "aktualisiert",
    "aktuell",
    "korrigiert",
    "korr",
    "corrected",
    "fixed",
    "revised",
    "uberarbeitet",
    "copy",
    "kopie",
];
/// Words followed by a version number ("v 2" after normalising "v2").
const NUMBER_WORDS: &[&str] = &["v", "ver", "version", "rev", "revision"];

fn is_number(token: &str, max_len: usize) -> bool {
    !token.is_empty() && token.len() <= max_len && token.chars().all(|c| c.is_ascii_digit())
}

/// A stem without a trailing " (1)", as File Sorter, browsers and Windows add for copies.
fn without_copy_number(stem: &str) -> &str {
    let trimmed = stem.trim_end();
    if let Some(open) = trimmed.rfind(" (")
        && trimmed.ends_with(')')
        && is_number(&trimmed[open + 2..trimmed.len() - 1], 3)
    {
        return &trimmed[..open];
    }
    trimmed
}

/// The words of a stem with version marks taken out, and the version number if there was one.
fn split(stem: &str) -> (Vec<String>, Option<u32>) {
    let tokens: Vec<String> = normalise(without_copy_number(stem)).split(' ').map(str::to_string).collect();
    let mut kept = Vec::new();
    let mut number = None;
    let mut i = 0;
    while i < tokens.len() {
        let t = tokens[i].as_str();
        let next = tokens.get(i + 1).map(String::as_str);
        if NUMBER_WORDS.contains(&t) && next.is_some_and(|n| is_number(n, 3)) {
            number = next.and_then(|n| n.parse().ok());
            i += 2;
        } else if VERSION_WORDS.contains(&t) {
            i += 1;
        } else {
            kept.push(tokens[i].clone());
            i += 1;
        }
    }
    (kept, number)
}

/// The name shared by all versions of a document ("skript" for "Skript_v2"), or `None` if
/// too little is left to be sure.
pub fn base_name(stem: &str) -> Option<String> {
    let base = split(stem).0.join(" ");
    (base.chars().filter(|c| c.is_alphanumeric()).count() >= 3).then_some(base)
}

/// The explicit version number ("v3" → 3), if the name has one.
pub fn version_number(stem: &str) -> Option<u32> {
    split(stem).1
}

fn stem_and_ext(path: &Path) -> (String, String) {
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    (stem, ext)
}

/// Files directly in `folder` that are other versions of `new` (same type, same base name).
pub fn other_versions(folder: &Path, new: &Path) -> Vec<PathBuf> {
    let (stem, ext) = stem_and_ext(new);
    let Some(base) = base_name(&stem) else { return Vec::new() };
    let Ok(entries) = fs::read_dir(folder) else { return Vec::new() };
    let mut found: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file() && p != new)
        .filter(|p| {
            let (s, e) = stem_and_ext(p);
            e == ext && !s.starts_with("~$") && base_name(&s).as_deref() == Some(base.as_str())
        })
        .collect();
    found.sort();
    found
}

/// Words at the start of a document that are compared.
const TITLE_WORDS: usize = 80;
/// Title pages this much alike (Jaccard of word triples) are the same document.
const SAME_TITLE: f64 = 0.85;
/// Title pages this little alike are different documents, whatever their names say.
const DIFFERENT_TITLE: f64 = 0.2;
/// Files read to compare, at most.
const MAX_COMPARED: usize = 10;

/// Word triples from the start of a text.
fn shingles(text: &str) -> std::collections::HashSet<String> {
    let words: Vec<String> = normalise(text).split(' ').take(TITLE_WORDS).map(str::to_string).collect();
    words.windows(3).map(|w| w.join(" ")).collect()
}

/// How alike two title pages are (0 to 1), or `None` if one says too little to tell.
pub fn alike(a: &str, b: &str) -> Option<f64> {
    let (a, b) = (shingles(a), shingles(b));
    if a.len() < 10 || b.len() < 10 {
        return None;
    }
    Some(a.intersection(&b).count() as f64 / a.union(&b).count() as f64)
}

/// Other versions of `new` in `folder`, looking at the content too: files the name calls
/// versions are left out when their title page is clearly a different document, and files
/// with another name count when their title page is the same («Skript.pdf» and
/// «Skript_final_neu_WS.pdf»; only files whose name shares a word with the new one are read).
/// `text_of` reads the start of a file.
pub fn versions_by_content(
    folder: &Path,
    new: &Path,
    new_text: &str,
    text_of: impl Fn(&Path) -> String,
) -> Vec<PathBuf> {
    let by_name = other_versions(folder, new);
    let mut found: Vec<PathBuf> = by_name
        .into_iter()
        .filter(|other| alike(new_text, &text_of(other)).is_none_or(|a| a >= DIFFERENT_TITLE))
        .collect();
    if !found.is_empty() {
        return found;
    }
    let (stem, ext) = stem_and_ext(new);
    let words: Vec<String> = split(&stem).0.into_iter().filter(|w| w.chars().count() >= 3).collect();
    let Ok(entries) = fs::read_dir(folder) else { return found };
    let mut candidates: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file() && p != new)
        .filter(|p| {
            let (s, e) = stem_and_ext(p);
            e == ext && !s.starts_with("~$") && split(&s).0.iter().any(|w| words.contains(w))
        })
        .collect();
    candidates.sort();
    for other in candidates.into_iter().take(MAX_COMPARED) {
        if alike(new_text, &text_of(&other)).is_some_and(|a| a >= SAME_TITLE) {
            found.push(other);
        }
    }
    found
}

/// True if `new` is an older version than `existing`: a lower version number ("v1" arriving
/// while "v2" is there), or none while the existing one has one ("Skript" after
/// "Skript_v2"). Otherwise the file that arrives is the newer one.
pub fn is_older(new: &Path, existing: &Path) -> bool {
    match (version_number(&stem_and_ext(new).0), version_number(&stem_and_ext(existing).0)) {
        (Some(a), Some(b)) => a < b,
        (None, Some(_)) => true,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_share_a_base_name() {
        let base = |s: &str| base_name(s).unwrap_or_default();
        for name in [
            "Skript",
            "Skript_v2",
            "Skript (1)",
            "Skript final",
            "Skript-neu",
            "Skript Version 3 korrigiert",
            "skript_rev2",
        ] {
            assert_eq!(base(name), "skript", "{name}");
        }
    }

    #[test]
    fn different_documents_stay_different() {
        let base = |s: &str| base_name(s).unwrap_or_default();
        assert_ne!(base("Blatt 1"), base("Blatt 2"));
        assert_ne!(base("Klausur 2023"), base("Klausur 2024"));
        assert_ne!(base("Blatt3"), base("Blatt3_Lösung"));
        assert_ne!(base("VL 1 2026"), base("VL 2 2026"));
        assert_ne!(base("Protokoll 24.10.2026"), base("Protokoll 31.10.2026"));
        assert_eq!(base_name("v2"), None, "nothing left");
        assert_eq!(base_name("neu (1)"), None);
    }

    #[test]
    fn version_numbers() {
        assert_eq!(version_number("Skript_v2"), Some(2));
        assert_eq!(version_number("Skript Version 10"), Some(10));
        assert_eq!(version_number("Skript"), None);
        assert!(is_older(Path::new("a/Skript_v1.pdf"), Path::new("a/Skript_v2.pdf")));
        assert!(!is_older(Path::new("a/Skript_v3.pdf"), Path::new("a/Skript_v2.pdf")));
        assert!(is_older(Path::new("a/Skript.pdf"), Path::new("a/Skript_v2.pdf")));
        assert!(!is_older(Path::new("a/Skript_v2.pdf"), Path::new("a/Skript.pdf")));
    }

    #[test]
    fn content_tells_versions_apart() {
        let tmp = tempfile::tempdir().unwrap();
        let title = "Einführung in die Programmierung Kapitel 3 Rekursion und Iteration Prof. Dr. Huber \
                     Wintersemester 2026 Inhalt Rekursive Funktionen Basisfall Rekursionsschritt Beispiele";
        let other = "Statistik Übungsblatt 2 Wahrscheinlichkeit Zufallsvariablen Erwartungswert Varianz \
                     Aufgabe 1 Berechnen Sie den Erwartungswert der folgenden Verteilung";
        let texts: std::collections::HashMap<&str, &str> =
            [("Skript.pdf", title), ("Skript_final_neu_WS.pdf", title), ("Skript (1).pdf", other)].into();
        for name in ["Skript_final_neu_WS.pdf", "Skript (1).pdf", "Notizen.pdf"] {
            fs::write(tmp.path().join(name), name).unwrap();
        }
        let text_of = |p: &Path| texts.get(p.file_name().unwrap().to_str().unwrap()).copied().unwrap_or("").to_string();
        // «Skript (1).pdf» is named like a version, but is another document; the long name is the same one.
        let found = versions_by_content(tmp.path(), &tmp.path().join("Skript.pdf"), title, text_of);
        let names: Vec<_> = found.iter().map(|p| p.file_name().unwrap().to_string_lossy().into_owned()).collect();
        assert_eq!(names, ["Skript_final_neu_WS.pdf"]);
        assert!(alike(title, "kurz").is_none(), "too little text to tell");
    }

    #[test]
    fn finds_other_versions_in_the_folder() {
        let tmp = tempfile::tempdir().unwrap();
        for name in ["Skript.pdf", "Skript (1).pdf", "Skript.docx", "Skript_Anhang.pdf", "Blatt1.pdf"] {
            fs::write(tmp.path().join(name), name).unwrap();
        }
        fs::create_dir(tmp.path().join("Skript v3.pdf")).unwrap();
        let new = tmp.path().join("Skript_v2.pdf");
        let found: Vec<String> = other_versions(tmp.path(), &new)
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(found, ["Skript (1).pdf", "Skript.pdf"]);
    }
}
