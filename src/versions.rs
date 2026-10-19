//! Tell versions of the same file apart: "Skript.pdf", "Skript_v2.pdf", "Skript (1).pdf",
//! "Skript final 2026-10-24.pdf" are one document in several versions, while "Blatt 1"
//! and "Blatt 2" or "Klausur 2023" and "Klausur 2024" are different documents.

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

fn is_year(token: &str) -> bool {
    token.len() == 4 && is_number(token, 4) && (token.starts_with("19") || token.starts_with("20"))
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
        let after = tokens.get(i + 2).map(String::as_str);
        if NUMBER_WORDS.contains(&t) && next.is_some_and(|n| is_number(n, 3)) {
            number = next.and_then(|n| n.parse().ok());
            i += 2;
        } else if VERSION_WORDS.contains(&t) {
            i += 1;
        } else if t.len() == 8 && is_number(t, 8) && is_year(&t[..4]) {
            i += 1; // 20261024
        } else if is_year(t) && next.is_some_and(|n| is_number(n, 2)) && after.is_some_and(|n| is_number(n, 2)) {
            i += 3; // 2026-10-24
        } else if is_number(t, 2) && next.is_some_and(|n| is_number(n, 2)) && after.is_some_and(is_year) {
            i += 3; // 24.10.2026
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

/// True if `new` is an older version than `existing` by their version numbers ("v1" arriving
/// while "v2" is there). Without numbers, the file that arrives is the newer one.
pub fn is_older(new: &Path, existing: &Path) -> bool {
    match (version_number(&stem_and_ext(new).0), version_number(&stem_and_ext(existing).0)) {
        (Some(a), Some(b)) => a < b,
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
            "Skript 2026-10-24",
            "Skript_24.10.2026",
            "Skript 20261024",
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
        assert!(!is_older(Path::new("a/Skript.pdf"), Path::new("a/Skript_v2.pdf")));
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
