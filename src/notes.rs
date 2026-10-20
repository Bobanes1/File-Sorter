//! Notes per module, kept in `notes.json` (by the module's label).

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

pub const NOTES_FILE: &str = "notes.json";

#[derive(Debug, Default, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Notes {
    pub by_module: BTreeMap<String, String>,
}

impl Notes {
    pub fn load(file: &Path) -> Notes {
        fs::read_to_string(file).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
    }

    pub fn save(&self, file: &Path) -> std::io::Result<()> {
        crate::cloud::save_json(self, file)
    }

    /// The modules whose notes have every word of `query` (at the start of a word), with the
    /// passage around the first one.
    pub fn search(&self, query: &str) -> Vec<(String, String)> {
        let q = crate::index::Query::parse(query);
        let mut out = Vec::new();
        for (module, text) in &self.by_module {
            let folded = crate::index::fold(text);
            let has = |w: &String| {
                folded
                    .match_indices(w.as_str())
                    .any(|(b, _)| b == 0 || !folded[..b].chars().next_back().is_some_and(char::is_alphanumeric))
            };
            if q.terms.is_empty() && q.phrases.is_empty() {
                continue;
            }
            if !q.terms.iter().all(has)
                || !q.phrases.iter().all(|p| folded.contains(p.as_str()))
                || q.excluded.iter().any(has)
            {
                continue;
            }
            let first = q.terms.iter().chain(&q.phrases).filter_map(|w| folded.find(w.as_str())).min().unwrap_or(0);
            let at = folded[..first].chars().count();
            let passage: String = text.chars().skip(at.saturating_sub(40)).take(160).collect();
            out.push((module.clone(), passage.split_whitespace().collect::<Vec<_>>().join(" ")));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_notes_by_word_beginnings() {
        let mut notes = Notes::default();
        notes.by_module.insert("Statistik".into(), "Prüfung: Formelblatt erlaubt, Taschenrechner nicht".into());
        notes.by_module.insert("Prog".into(), "Rekursion nochmals üben".into());
        assert_eq!(notes.search("formel")[0].0, "Statistik");
        assert_eq!(notes.search("rekursion uben").len(), 1);
        assert!(notes.search("formel -taschenrechner").is_empty());
        assert!(notes.search("").is_empty());
    }
}
