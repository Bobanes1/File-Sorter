//! Where a file was downloaded from, to sort it like earlier files from the same place.
//!
//! On Windows, browsers note the address of a download next to the file (the
//! `Zone.Identifier` stream, "Mark of the Web"): the file's own address (`HostUrl`) and
//! the page the link was on (`ReferrerUrl`). For Moodle, ILIAS and OLAT that page names
//! the course, so every file from one course gets the same key, and once a few files
//! from there went into a module, the next one can go there too even if its name says
//! nothing ("Blatt3.pdf").

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::config::Config;

pub const ORIGINS_FILE: &str = "origins.json";
/// At most this many places are remembered (the ones used longest ago are dropped).
const MAX_PLACES: usize = 1000;
/// A file the user assigned by hand counts this much more than one sorted automatically.
const CHOSEN_WEIGHT: u32 = 3;

/// Where one file came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Source {
    /// The course or page, e.g. `moodle.uni.ch/course/12345`.
    pub key: String,
    /// The server, e.g. `moodle.uni.ch` (shown to the user).
    pub host: String,
}

/// The addresses noted for a download: the page it was linked from first, then the file's own.
pub fn download_urls(path: &Path) -> Vec<String> {
    #[cfg(windows)]
    {
        let mut stream = path.as_os_str().to_os_string();
        stream.push(":Zone.Identifier");
        let Ok(bytes) = fs::read(&stream) else { return Vec::new() };
        parse_zone_identifier(&String::from_utf8_lossy(&bytes))
    }
    #[cfg(not(windows))]
    {
        let _ = path;
        Vec::new()
    }
}

/// `ReferrerUrl` and `HostUrl` from the text of a `Zone.Identifier` stream, in that order.
#[cfg_attr(not(windows), allow(dead_code))]
fn parse_zone_identifier(text: &str) -> Vec<String> {
    let value = |name: &str| {
        text.lines()
            .filter_map(|line| line.trim().split_once('='))
            .find(|(key, _)| key.trim().eq_ignore_ascii_case(name))
            .map(|(_, v)| v.trim().to_string())
            .filter(|v| v.starts_with("http://") || v.starts_with("https://"))
    };
    [value("ReferrerUrl"), value("HostUrl")].into_iter().flatten().collect()
}

/// Where `path` was downloaded from, if the browser noted it.
pub fn source_of(path: &Path) -> Option<Source> {
    let urls = download_urls(path);
    // A course page beats the file's own address, which is often different for every file.
    urls.iter().find_map(|u| course_key(u)).or_else(|| urls.iter().find_map(|u| source_key(u)))
}

/// Host, path and query of a web address (without user, port, fragment).
fn split_url(url: &str) -> Option<(String, String, String)> {
    let rest = url.split_once("://")?.1;
    let rest = rest.split('#').next().unwrap_or(rest);
    let (authority, path_query) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    let host = authority.rsplit('@').next()?.split(':').next()?.to_lowercase();
    if host.is_empty() {
        return None;
    }
    let (path, query) = path_query.split_once('?').unwrap_or((path_query, ""));
    Some((host, path.to_string(), query.to_string()))
}

fn query_value<'a>(query: &'a str, name: &str) -> Option<&'a str> {
    query.split('&').filter_map(|pair| pair.split_once('=')).find(|(k, _)| *k == name).map(|(_, v)| v)
}

fn digits(text: &str) -> Option<&str> {
    let end = text.find(|c: char| !c.is_ascii_digit()).unwrap_or(text.len());
    (end > 0).then(|| &text[..end])
}

/// A key for a course page of Moodle, ILIAS or OLAT; `None` for other addresses.
fn course_key(url: &str) -> Option<Source> {
    let (host, path, query) = split_url(url)?;
    let make = |what: &str, id: &str| Some(Source { key: format!("{host}/{what}/{id}"), host: host.clone() });
    let lower = path.to_lowercase();
    // Moodle: the course page, or one of its sections.
    if lower.ends_with("/course/view.php") || lower.ends_with("/course/section.php") {
        let id = query_value(&query, "id").and_then(digits)?;
        return make(if lower.ends_with("section.php") { "section" } else { "course" }, id);
    }
    // ILIAS: goto.php?target=crs_123 / fold_123, or ilias.php?ref_id=123.
    if let Some(target) = query_value(&query, "target") {
        for kind in ["crs_", "grp_", "fold_"] {
            if let Some(id) = target.strip_prefix(kind).and_then(digits) {
                return make(kind.trim_end_matches('_'), id);
            }
        }
    }
    if let Some(id) = query_value(&query, "ref_id").and_then(digits) {
        return make("ref", id);
    }
    // ILIAS also writes goto_<client>_crs_123.html.
    if let Some(rest) = lower.rsplit('/').next().filter(|f| f.starts_with("goto_"))
        && let Some(i) = rest.find("_crs_").or_else(|| rest.find("_fold_"))
    {
        let id = digits(rest[i + 1..].split_once('_')?.1)?;
        return make("ref", id);
    }
    // OLAT / OpenOlat: .../RepositoryEntry/12345/...
    let parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
    if let Some(i) = parts.iter().position(|p| p.eq_ignore_ascii_case("RepositoryEntry"))
        && let Some(id) = parts.get(i + 1).copied().and_then(digits)
    {
        return make("repo", id);
    }
    None
}

/// A key for any address: a course page as above, or else the host and the first two
/// folders of the path (`www.inf.uni.ch/courses/prog1`).
pub fn source_key(url: &str) -> Option<Source> {
    if let Some(source) = course_key(url) {
        return Some(source);
    }
    let (host, path, query) = split_url(url)?;
    let lower = path.to_lowercase();
    // Moodle: a single resource's page or file. Per file, but the same if it is downloaded again.
    if lower.contains("/mod/")
        && lower.ends_with("/view.php")
        && let Some(id) = query_value(&query, "id").and_then(digits)
    {
        return Some(Source { key: format!("{host}/mod/{id}"), host });
    }
    let folders: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
    // The last part is the file itself.
    let folders = &folders[..folders.len().saturating_sub(1)];
    let key = std::iter::once(host.as_str()).chain(folders.iter().take(2).copied()).collect::<Vec<_>>().join("/");
    Some(Source { key, host })
}

/// What was learned about one place: how often files from there went into which module.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct Place {
    modules: HashMap<String, u32>,
    /// When a file from there was last sorted (seconds since 1970).
    last: i64,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Origins {
    places: HashMap<String, Place>,
}

impl Origins {
    pub fn load(file: &Path) -> Origins {
        fs::read_to_string(file).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
    }

    pub fn save(&self, file: &Path) -> std::io::Result<()> {
        crate::cloud::save_json(self, file)
    }

    /// The module (by label) that files from `key` clearly go to: the only one, or one
    /// chosen at least twice as often as any other.
    pub fn module_for(&self, key: &str) -> Option<&str> {
        let place = self.places.get(key)?;
        let mut ranked: Vec<(&String, &u32)> = place.modules.iter().collect();
        ranked.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
        match ranked.as_slice() {
            [] => None,
            [(label, _)] => Some(label),
            [(label, first), (_, second), ..] => (**first >= 2 * **second).then_some(label.as_str()),
        }
    }

    pub fn remember(&mut self, key: &str, module: &str, chosen: bool, now: i64) {
        let place = self.places.entry(key.to_string()).or_default();
        *place.modules.entry(module.to_string()).or_default() += if chosen { CHOSEN_WEIGHT } else { 1 };
        place.last = now;
        if self.places.len() > MAX_PLACES {
            let mut by_age: Vec<(i64, String)> = self.places.iter().map(|(k, p)| (p.last, k.clone())).collect();
            by_age.sort();
            for (_, key) in by_age.into_iter().take(self.places.len() - MAX_PLACES) {
                self.places.remove(&key);
            }
        }
    }

    /// A file from `key` was taken out of `module` again (it was sorted there by mistake).
    pub fn forget(&mut self, key: &str, module: &str) {
        if let Some(place) = self.places.get_mut(key) {
            place.modules.remove(module);
        }
    }
}

/// The index of the module that files from `source` go to, if that is clear.
pub fn lookup(config: &Config, source: &Source) -> Option<usize> {
    let file = config.state_file(ORIGINS_FILE)?;
    let origins = Origins::load(&file);
    let label = origins.module_for(&source.key)?;
    config.courses.iter().position(|c| c.label() == label)
}

/// Note that a file from `source` went into `module`.
pub fn remember(config: &Config, source: &Source, module: &str, chosen: bool) {
    update(config, |origins| origins.remember(&source.key, module, chosen, chrono::Local::now().timestamp()));
}

/// Note that a file from `source` didn't belong into `module` after all.
pub fn forget(config: &Config, source: &Source, module: &str) {
    update(config, |origins| origins.forget(&source.key, module));
}

fn update(config: &Config, change: impl FnOnce(&mut Origins)) {
    let Some(file) = config.state_file(ORIGINS_FILE) else { return };
    let mut origins = Origins::load(&file);
    change(&mut origins);
    if let Err(e) = origins.save(&file) {
        log::info!("Could not save where downloads came from: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(url: &str) -> String {
        source_key(url).map(|s| s.key).unwrap_or_default()
    }

    #[test]
    fn keys_for_learning_platforms() {
        assert_eq!(key("https://moodle.uni.ch/course/view.php?id=4711&section=2#x"), "moodle.uni.ch/course/4711");
        assert_eq!(key("https://moodle.uni.ch/course/section.php?id=88"), "moodle.uni.ch/section/88");
        assert_eq!(key("https://moodle.uni.ch/mod/resource/view.php?id=123"), "moodle.uni.ch/mod/123");
        assert_eq!(key("https://ilias.fh.ch/goto.php?target=crs_5550&client_id=fh"), "ilias.fh.ch/crs/5550");
        assert_eq!(key("https://ilias.fh.ch/ilias.php?ref_id=777&cmd=view"), "ilias.fh.ch/ref/777");
        assert_eq!(key("https://ilias.fh.ch/goto_fh_crs_5550.html"), "ilias.fh.ch/ref/5550");
        assert_eq!(key("https://lms.uzh.ch/auth/RepositoryEntry/16234/CourseNode/1"), "lms.uzh.ch/repo/16234");
        assert_eq!(key("https://www.inf.ethz.ch/courses/prog1/2026/blatt3.pdf"), "www.inf.ethz.ch/courses/prog1");
        assert_eq!(key("https://user@files.uni.ch:8443/skript.pdf"), "files.uni.ch");
        assert_eq!(source_key("not a url"), None);
        assert_eq!(
            source_key("https://moodle.uni.ch/course/view.php?id=").map(|s| s.host).as_deref(),
            Some("moodle.uni.ch")
        );
    }

    #[test]
    fn zone_identifier_prefers_the_course_page() {
        let text = "[ZoneTransfer]\r\nZoneId=3\r\nReferrerUrl=https://moodle.uni.ch/course/view.php?id=4711\r\n\
                    HostUrl=https://moodle.uni.ch/pluginfile.php/99/mod_resource/content/1/blatt3.pdf\r\n";
        let urls = parse_zone_identifier(text);
        assert_eq!(urls.len(), 2);
        assert!(urls[0].contains("course/view.php"));
        assert_eq!(parse_zone_identifier("[ZoneTransfer]\nZoneId=3\nHostUrl=about:internet\n"), Vec::<String>::new());
    }

    #[test]
    fn learns_clear_majorities_only() {
        let mut o = Origins::default();
        assert_eq!(o.module_for("k"), None);
        o.remember("k", "Prog", false, 1);
        assert_eq!(o.module_for("k"), Some("Prog"));
        o.remember("k", "Stats", false, 2);
        assert_eq!(o.module_for("k"), None, "a tie says nothing");
        o.remember("k", "Prog", false, 3);
        assert_eq!(o.module_for("k"), Some("Prog"));
        o.remember("k", "Stats", true, 4);
        assert_eq!(o.module_for("k"), Some("Stats"), "assigned by hand counts more");
        o.forget("k", "Stats");
        assert_eq!(o.module_for("k"), Some("Prog"));
        for i in 0..(MAX_PLACES + 5) {
            o.remember(&format!("p{i}"), "X", false, 10 + i as i64);
        }
        assert_eq!(o.places.len(), MAX_PLACES);
        assert!(o.places.contains_key(&format!("p{}", MAX_PLACES + 4)) && !o.places.contains_key("k"));
    }

    #[test]
    fn saved_and_loaded() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join(ORIGINS_FILE);
        let mut o = Origins::default();
        o.remember("moodle/course/1", "Prog", true, 5);
        o.save(&file).unwrap();
        assert_eq!(Origins::load(&file).module_for("moodle/course/1"), Some("Prog"));
        assert_eq!(Origins::load(&tmp.path().join("missing")).module_for("x"), None);
    }
}
