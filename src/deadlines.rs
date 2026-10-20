//! Deadlines and dates found in course files ("Abgabe: 24.10.2026, 23:59", "bis zum 7.11.
//! abzugeben", "due Oct 24", "Klausur: 20.01.2027, 14:00"), or added by hand, kept in
//! `deadlines.json`.
//!
//! Finding dates in free text can go wrong, so every entry shows the passage it came from,
//! can be corrected or deleted, and a deleted one isn't found again in the same file.

use std::fs;
use std::path::{Path, PathBuf};

use chrono::{Datelike, Local, NaiveDate, NaiveDateTime, NaiveTime};
use serde::{Deserialize, Serialize};

use crate::index::fold;

pub const DEADLINES_FILE: &str = "deadlines.json";
/// Bumped when finding gets better: files are then read again (deleted entries stay deleted).
const FINDER_VERSION: u32 = 2;

/// What kind of date it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// Something has to be handed in.
    #[default]
    Due,
    /// An exam, test, presentation: a date to be there.
    Exam,
}

/// Where the date is, seen from the word.
#[derive(Clone, Copy, PartialEq)]
enum Look {
    /// After it: "Abgabe: 3.11.".
    Ahead,
    /// Before it: "bis zum 7.11. abzugeben".
    Behind,
    /// A period after it, of which the end counts: "Bearbeitungszeit: 27.10. – 10.11.".
    Period,
}

/// Words (folded: lower-case, no accents) that announce a date, what kind it is, where the
/// date is, and whether the word may go on ("abgabe" also finds "Abgabetermin", but
/// "prufung" must not find "prüfungsrelevant").
const TRIGGERS: &[(&str, Kind, Look, bool)] = &[
    ("abgabe", Kind::Due, Look::Ahead, true),
    ("einsendeschluss", Kind::Due, Look::Ahead, false),
    ("einreichung", Kind::Due, Look::Ahead, false),
    ("deadline", Kind::Due, Look::Ahead, true),
    ("due", Kind::Due, Look::Ahead, false),
    ("submit", Kind::Due, Look::Ahead, true),
    ("submission", Kind::Due, Look::Ahead, false),
    ("hand in", Kind::Due, Look::Ahead, false),
    ("turn in", Kind::Due, Look::Ahead, false),
    ("spatestens", Kind::Due, Look::Ahead, false),
    ("fallig", Kind::Due, Look::Ahead, false),
    ("faellig", Kind::Due, Look::Ahead, false),
    ("abzugeben", Kind::Due, Look::Behind, false),
    ("abgeben", Kind::Due, Look::Behind, false),
    ("einzureichen", Kind::Due, Look::Behind, false),
    ("einreichen", Kind::Due, Look::Behind, false),
    ("hochzuladen", Kind::Due, Look::Behind, false),
    ("hochladen", Kind::Due, Look::Behind, false),
    ("bearbeitungszeit", Kind::Due, Look::Period, true),
    ("klausur", Kind::Exam, Look::Ahead, false),
    ("nachklausur", Kind::Exam, Look::Ahead, false),
    ("probeklausur", Kind::Exam, Look::Ahead, false),
    ("prufung", Kind::Exam, Look::Ahead, false),
    ("pruefung", Kind::Exam, Look::Ahead, false),
    ("zwischenprufung", Kind::Exam, Look::Ahead, false),
    ("modulprufung", Kind::Exam, Look::Ahead, false),
    ("endprufung", Kind::Exam, Look::Ahead, false),
    ("exam", Kind::Exam, Look::Ahead, false),
    ("final exam", Kind::Exam, Look::Ahead, false),
    ("midterm", Kind::Exam, Look::Ahead, false),
    ("testat", Kind::Exam, Look::Ahead, false),
    ("kolloquium", Kind::Exam, Look::Ahead, false),
    ("prasentation", Kind::Exam, Look::Ahead, false),
    ("presentation", Kind::Exam, Look::Ahead, false),
    ("vortrag", Kind::Exam, Look::Ahead, false),
];
/// How far from the word the date may be (within the same sentence).
const LOOK_AHEAD: usize = 90;
const LOOK_BEHIND: usize = 60;
/// How much text around it is kept to show where it was found.
const CONTEXT: usize = 70;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Deadline {
    pub id: u64,
    /// What is due ("Blatt 3"), the file name or what the user typed.
    pub title: String,
    /// The module's label ("Informatik › Programmieren"), empty if none.
    pub module: String,
    pub file: Option<PathBuf>,
    /// `YYYY-MM-DD`.
    pub date: String,
    /// `HH:MM`, if a time was given.
    pub time: Option<String>,
    /// The text around the date, to check it was read right.
    pub context: String,
    pub done: bool,
    /// Deleted by the user: hidden, and not found again in the same file.
    pub deleted: bool,
    #[serde(default)]
    pub kind: Kind,
    /// The date as it was found, when the user corrected it (so it isn't found again).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub found_date: Option<String>,
}

impl Deadline {
    pub fn day(&self) -> Option<NaiveDate> {
        NaiveDate::parse_from_str(&self.date, "%Y-%m-%d").ok()
    }

    pub fn clock(&self) -> Option<NaiveTime> {
        NaiveTime::parse_from_str(self.time.as_deref()?, "%H:%M").ok()
    }

    /// The moment it is due (the end of the day without a time).
    pub fn due(&self) -> Option<NaiveDateTime> {
        let end_of_day = NaiveTime::from_hms_opt(23, 59, 0)?;
        Some(self.day()?.and_time(self.clock().unwrap_or(end_of_day)))
    }
}

/// A deadline found in a text: the day, maybe a time, and the text around it.
#[derive(Debug, Clone, PartialEq)]
pub struct Found {
    pub date: NaiveDate,
    pub time: Option<NaiveTime>,
    pub context: String,
    pub kind: Kind,
    /// The word as written ("Klausur", "Zwischenprüfung"), to name an exam date.
    pub word: String,
}

fn month(word: &str) -> Option<u32> {
    const MONTHS: &[(&str, u32)] = &[
        ("jan", 1),
        ("feb", 2),
        ("mar", 3),
        ("apr", 4),
        ("mai", 5),
        ("may", 5),
        ("jun", 6),
        ("jul", 7),
        ("aug", 8),
        ("sep", 9),
        ("okt", 10),
        ("oct", 10),
        ("nov", 11),
        ("dez", 12),
        ("dec", 12),
    ];
    if word.len() < 3 || !word.chars().all(|c| c.is_ascii_alphabetic()) {
        return None;
    }
    MONTHS.iter().find(|(prefix, _)| word.starts_with(prefix)).map(|(_, m)| *m)
}

/// A number of at most `max` digits at `i`; returns it and where it ends.
fn number(chars: &[char], i: usize, max: usize) -> Option<(u32, usize)> {
    let digits: String = chars[i.min(chars.len())..].iter().take_while(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() || digits.len() > max || (i > 0 && chars[i - 1].is_ascii_digit()) {
        return None;
    }
    Some((digits.parse().ok()?, i + digits.len()))
}

fn word(chars: &[char], i: usize) -> (String, usize) {
    let w: String = chars[i.min(chars.len())..].iter().take_while(|c| c.is_alphabetic()).collect();
    let end = i + w.chars().count();
    (w, end)
}

fn skip_spaces(chars: &[char], mut i: usize) -> usize {
    while chars.get(i).is_some_and(|c| *c == ' ') {
        i += 1;
    }
    i
}

/// A date starting at `i`: (year if given, month, day, end).
fn date_at(chars: &[char], i: usize) -> Option<(Option<i32>, u32, u32, usize)> {
    let year_after = |end: usize| -> (Option<i32>, usize) {
        let j = skip_spaces(chars, end);
        let j = if chars.get(j) == Some(&',') { skip_spaces(chars, j + 1) } else { j };
        match number(chars, j, 4) {
            Some((y, e)) if e - j == 4 && (2000..2100).contains(&y) => (Some(y as i32), e),
            _ => (None, end),
        }
    };
    if let Some((first, end)) = number(chars, i, 4) {
        // 2026-10-24
        if end - i == 4 && chars.get(end) == Some(&'-') {
            let (m, e) = number(chars, end + 1, 2)?;
            if chars.get(e) != Some(&'-') {
                return None;
            }
            let (d, e) = number(chars, e + 1, 2)?;
            return Some((Some(first as i32), m, d, e));
        }
        if end - i > 2 {
            return None;
        }
        let day = first;
        // 24.10. / 24.10.2026 / 24.10.26
        if chars.get(end) == Some(&'.') {
            let j = skip_spaces(chars, end + 1);
            if let Some((m, e)) = number(chars, j, 2) {
                if chars.get(e) == Some(&'.') {
                    let k = skip_spaces(chars, e + 1);
                    // A time right after the date ("24.10. 10:15") is no year.
                    let time_follows = |end: usize| {
                        matches!(chars.get(end), Some(':' | '.'))
                            && chars.get(end + 1).is_some_and(char::is_ascii_digit)
                    };
                    return match number(chars, k, 4) {
                        Some((y, ye)) if ye - k == 4 && !time_follows(ye) => Some((Some(y as i32), m, day, ye)),
                        Some((y, ye)) if ye - k == 2 && k == e + 1 && !time_follows(ye) => {
                            Some((Some(2000 + y as i32), m, day, ye))
                        }
                        _ => Some((None, m, day, e + 1)),
                    };
                }
                return None;
            }
        }
        // 24. Oktober 2026 / 24 October / 24th of October
        let mut j = if chars.get(end) == Some(&'.') { end + 1 } else { end };
        let (suffix, after_suffix) = word(chars, j);
        if ["st", "nd", "rd", "th"].contains(&suffix.as_str()) {
            j = after_suffix;
        }
        let j = skip_spaces(chars, j);
        let (w, e) = word(chars, j);
        let (w, e) = if w == "of" { word(chars, skip_spaces(chars, e)) } else { (w, e) };
        let m = month(&w)?;
        let (year, end) = year_after(e);
        return Some((year, m, day, end));
    }
    // October 24 / Oct 24th, 2026
    let (w, e) = word(chars, i);
    let m = month(&w)?;
    let mut j = if chars.get(e) == Some(&'.') { e + 1 } else { e };
    j = skip_spaces(chars, j);
    let (day, mut end) = number(chars, j, 2)?;
    let (suffix, after_suffix) = word(chars, end);
    if ["st", "nd", "rd", "th"].contains(&suffix.as_str()) {
        end = after_suffix;
    }
    let (year, end) = year_after(end);
    Some((year, m, day, end))
}

/// A time shortly after a date: "23:59", "23.59 Uhr", "18 Uhr", "11:59 pm". Not another date
/// ("10.11.2026" is no time 10:11), and not past the end of the sentence.
fn time_after(chars: &[char], from: usize) -> Option<NaiveTime> {
    let limit = (from + 24).min(chars.len());
    let mut i = from;
    while i < limit {
        if chars[i] == '.' && chars.get(i + 1) == Some(&' ') && chars.get(i + 2).is_some_and(|c| c.is_alphabetic()) {
            return None;
        }
        if let Some((h, e)) = number(chars, i, 2) {
            let (mut hour, mut minute, mut end) = (h, 0, e);
            let after_word = |at: usize| word(chars, skip_spaces(chars, at)).0;
            match chars.get(e) {
                Some(':') if let Some((m, me)) = number(chars, e + 1, 2).filter(|&(_, me)| me - (e + 1) == 2) => {
                    minute = m;
                    end = me;
                }
                // "23.59 Uhr", but "10.11." or "10.11.2026" is a date.
                Some('.')
                    if let Some((m, me)) = number(chars, e + 1, 2).filter(|&(_, me)| me - (e + 1) == 2)
                        && chars.get(me) != Some(&'.')
                        && ["uhr", "h"].contains(&after_word(me).as_str()) =>
                {
                    minute = m;
                    end = me;
                }
                _ if ["uhr", "h"].contains(&after_word(e).as_str()) => {}
                _ => {
                    i = e;
                    continue;
                }
            }
            let suffix = after_word(end);
            if suffix == "pm" && hour < 12 {
                hour += 12;
            } else if suffix == "am" && hour == 12 {
                hour = 0;
            }
            return NaiveTime::from_hms_opt(hour, minute, 0);
        }
        i += 1;
    }
    None
}

/// The year for a date without one: the one that puts it closest after `reference` (a
/// sheet from September saying "24.10." means this October, one from December saying
/// "15.1." means next January).
fn infer_year(month: u32, day: u32, reference: NaiveDate) -> Option<NaiveDate> {
    let this = NaiveDate::from_ymd_opt(reference.year(), month, day)?;
    if this < reference - chrono::Duration::days(60) {
        NaiveDate::from_ymd_opt(reference.year() + 1, month, day)
    } else {
        Some(this)
    }
}

/// True where a sentence ends: ". " (or "! ", "? ") after a word, followed by a capital
/// letter. A dot after a number ("2. November", "3.11.") doesn't end one.
fn sentence_ends(raw: &[char]) -> Vec<bool> {
    let mut ends = vec![false; raw.len()];
    for i in 1..raw.len() {
        if !matches!(raw[i], '.' | '!' | '?') || !raw[i - 1].is_alphabetic() {
            continue;
        }
        let next = raw[i + 1..].iter().position(|c| !c.is_whitespace()).map(|n| raw[i + 1 + n]);
        if raw.get(i + 1).is_some_and(|c| c.is_whitespace()) && next.is_some_and(char::is_uppercase) {
            ends[i] = true;
        }
    }
    ends
}

/// The date written at `i` (with the year worked out), and where it ends.
fn full_date(folded: &[char], i: usize, reference: NaiveDate) -> Option<(NaiveDate, usize)> {
    if i > 0 && folded[i - 1].is_alphanumeric() {
        return None;
    }
    let (year, m, d, end) = date_at(folded, i)?;
    let date = match year {
        Some(y) => NaiveDate::from_ymd_opt(y, m, d)?,
        None => infer_year(m, d, reference)?,
    };
    Some((date, end))
}

/// A table: the word is one of several column titles on its line ("Ausgabe Abgabe
/// Besprechung") and the next line holds one date per column. Returns its column's date.
fn table_date(raw: &[char], folded: &[char], at: usize, reference: NaiveDate) -> Option<(NaiveDate, usize, usize)> {
    let line_start = raw[..at].iter().rposition(|c| *c == '\n').map_or(0, |p| p + 1);
    let line_end = raw[at..].iter().position(|c| *c == '\n').map_or(raw.len(), |p| at + p);
    let titles: Vec<usize> = word_starts(raw, line_start, line_end);
    let column = titles.iter().rposition(|&s| s <= at)?;
    if titles.len() < 2 {
        return None;
    }
    // The next line with something on it.
    let mut start = line_end;
    while start < raw.len() && raw[start].is_whitespace() {
        start += 1;
    }
    let end = raw[start..].iter().position(|c| *c == '\n').map_or(raw.len(), |p| start + p);
    let cells = word_starts(raw, start, end);
    if cells.len() != titles.len() {
        return None;
    }
    let mut dates = Vec::new();
    for &cell in &cells {
        let (date, date_end) = full_date(folded, cell, reference)?;
        dates.push((date, cell, date_end));
    }
    Some(dates[column])
}

fn word_starts(raw: &[char], from: usize, to: usize) -> Vec<usize> {
    (from..to).filter(|&i| !raw[i].is_whitespace() && (i == from || raw[i - 1].is_whitespace())).collect()
}

/// Deadlines and exam dates in `text`: a deadline word with a date close to it in the same
/// sentence. `reference` is about when the file was published, for dates without a year.
pub fn find(text: &str, reference: NaiveDate) -> Vec<Found> {
    let raw: Vec<char> = text.chars().collect();
    let folded: Vec<char> = fold(text).chars().collect();
    let haystack: String = folded.iter().collect();
    let ends = sentence_ends(&raw);
    let mut found: Vec<Found> = Vec::new();
    for &(trigger, kind, look, open_end) in TRIGGERS {
        for (byte, _) in haystack.match_indices(trigger) {
            let start = haystack[..byte].chars().count();
            let after = start + trigger.chars().count();
            // Whole words ("due" not in "residue"); some may go on ("Abgabetermin").
            if start > 0 && folded[start - 1].is_alphanumeric()
                || !open_end && folded.get(after).is_some_and(|c| c.is_alphanumeric())
            {
                continue;
            }
            let hit = match look {
                Look::Ahead | Look::Period => table_date(&raw, &folded, start, reference).or_else(|| {
                    let limit = (after + LOOK_AHEAD).min(folded.len());
                    let mut i = after;
                    while i < limit && !ends[i] {
                        if let Some((date, end)) = full_date(&folded, i, reference) {
                            return Some(match look {
                                Look::Period => period_end(&folded, end, date, reference),
                                _ => (date, i, end),
                            });
                        }
                        i += 1;
                    }
                    None
                }),
                Look::Behind => {
                    let mut hit = None;
                    let mut i = start;
                    while i > 0 && start - i < LOOK_BEHIND && !ends[i - 1] {
                        i -= 1;
                        if let Some((date, end)) = full_date(&folded, i, reference).filter(|&(_, end)| end <= start) {
                            hit = Some((date, i, end));
                            break;
                        }
                    }
                    hit
                }
            };
            let Some((date, date_start, date_end)) = hit else { continue };
            if (date - reference).num_days().abs() > 366 {
                continue;
            }
            // The passage around it, from and to whole words.
            let mut from = start.min(date_start).saturating_sub(CONTEXT / 3);
            while from > 0 && from < start.min(date_start) && !raw[from - 1].is_whitespace() {
                from += 1;
            }
            let mut to = (start.max(date_end) + CONTEXT / 2).min(raw.len());
            while to < raw.len() && to > start.max(date_end) && !raw[to].is_whitespace() {
                to -= 1;
            }
            let context: String =
                raw[from..to].iter().collect::<String>().split_whitespace().collect::<Vec<_>>().join(" ");
            let word_end = (after..raw.len()).find(|&i| !raw[i].is_alphanumeric()).unwrap_or(raw.len());
            let word: String = raw[start..word_end].iter().collect();
            if !found.iter().any(|f| f.date == date) {
                found.push(Found { date, time: time_after(&folded, date_end), context, kind, word });
            }
        }
    }
    found.sort_by_key(|f| (f.date, f.time));
    found
}

/// "27.10. – 10.11.2026": the end of a period that starts with the date just read.
fn period_end(folded: &[char], first_end: usize, first: NaiveDate, reference: NaiveDate) -> (NaiveDate, usize, usize) {
    let mut i = skip_spaces(folded, first_end);
    match folded.get(i) {
        Some('-' | '–' | '—') => i += 1,
        _ if word(folded, i).0 == "bis" => i = word(folded, i).1,
        _ => return (first, first_end, first_end),
    }
    let i = skip_spaces(folded, i);
    match full_date(folded, i, reference) {
        Some((date, end)) => (date, i, end),
        None => (first, first_end, first_end),
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Deadlines {
    pub items: Vec<Deadline>,
    /// Files already looked at (path and size), so they aren't read again.
    scanned: Vec<(PathBuf, u64)>,
    /// Which [`FINDER_VERSION`] looked at them.
    finder: u32,
}

impl Deadlines {
    pub fn load(file: &Path) -> Deadlines {
        let mut list: Deadlines =
            fs::read_to_string(file).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
        // Found with an older, weaker finder: read the files again (what was deleted stays deleted).
        if list.finder < FINDER_VERSION {
            list.scanned.clear();
            list.finder = FINDER_VERSION;
        }
        list
    }

    /// Read every file again (after the user asked to look again).
    pub fn forget_scanned(&mut self) {
        self.scanned.clear();
    }

    pub fn save(&self, file: &Path) -> std::io::Result<()> {
        crate::cloud::save_json(self, file)
    }

    fn next_id(&self) -> u64 {
        self.items.iter().map(|d| d.id).max().unwrap_or(0) + 1
    }

    pub fn was_scanned(&self, path: &Path, size: u64) -> bool {
        self.scanned.iter().any(|(p, s)| p == path && *s == size)
    }

    /// Add what was found in `file` (of `module`); returns how many are new. A deadline on
    /// the same day from the same file (also a deleted one) isn't added twice.
    pub fn add_found(&mut self, file: &Path, size: u64, module: &str, found: Vec<Found>) -> usize {
        self.scanned.retain(|(p, _)| p != file);
        self.scanned.push((file.to_path_buf(), size));
        // Keep the list of scanned files from growing forever.
        let excess = self.scanned.len().saturating_sub(5000);
        self.scanned.drain(..excess);
        let stem = file.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let mut added = 0;
        for f in found {
            // A hand-in is named after its sheet, an exam by what it is ("Klausur").
            let title = match f.kind {
                Kind::Due => stem.clone(),
                Kind::Exam => capitalised(&f.word),
            };
            let date = f.date.format("%Y-%m-%d").to_string();
            // Also not when the user corrected the date of what was found here.
            let known = self.items.iter().any(|d| {
                d.file.as_deref() == Some(file) && (d.date == date || d.found_date.as_deref() == Some(date.as_str()))
            });
            if known {
                continue;
            }
            let id = self.next_id();
            self.items.push(Deadline {
                id,
                title,
                module: module.to_string(),
                file: Some(file.to_path_buf()),
                date,
                time: f.time.map(|t| t.format("%H:%M").to_string()),
                context: f.context,
                done: false,
                deleted: false,
                kind: f.kind,
                found_date: None,
            });
            added += 1;
        }
        added
    }

    pub fn add_manual(
        &mut self,
        title: &str,
        module: &str,
        kind: Kind,
        date: NaiveDate,
        time: Option<NaiveTime>,
    ) -> u64 {
        let id = self.next_id();
        self.items.push(Deadline {
            id,
            title: title.trim().to_string(),
            module: module.to_string(),
            file: None,
            date: date.format("%Y-%m-%d").to_string(),
            time: time.map(|t| t.format("%H:%M").to_string()),
            context: String::new(),
            done: false,
            deleted: false,
            kind,
            found_date: None,
        });
        id
    }

    /// Correct an entry (a date that was read wrong, a better title).
    pub fn edit(&mut self, id: u64, title: &str, module: &str, kind: Kind, date: NaiveDate, time: Option<NaiveTime>) {
        let Some(d) = self.items.iter_mut().find(|d| d.id == id) else { return };
        let date = date.format("%Y-%m-%d").to_string();
        if d.file.is_some() && d.found_date.is_none() && d.date != date {
            d.found_date = Some(d.date.clone());
        }
        d.title = title.trim().to_string();
        d.module = module.to_string();
        d.kind = kind;
        d.date = date;
        d.time = time.map(|t| t.format("%H:%M").to_string());
    }

    /// Open deadlines from `days_back` days ago up to `days_ahead` days from `now`, soonest first.
    pub fn upcoming(&self, now: NaiveDateTime, days_back: i64, days_ahead: i64) -> Vec<&Deadline> {
        let mut list: Vec<&Deadline> = self
            .items
            .iter()
            .filter(|d| !d.deleted && !d.done)
            .filter(|d| {
                d.due().is_some_and(|due| {
                    let days = (due - now).num_days();
                    (-days_back..=days_ahead).contains(&days)
                })
            })
            .collect();
        list.sort_by_key(|d| d.due());
        list
    }

    /// Files moved (into another module, or with the study folder): their deadlines follow.
    /// `moved` gives a file's new place, or `None` if it didn't move.
    pub fn follow(&mut self, moved: impl Fn(&Path) -> Option<PathBuf>) -> bool {
        let mut changed = false;
        for d in &mut self.items {
            if let Some(new) = d.file.as_deref().and_then(&moved) {
                d.file = Some(new);
                changed = true;
            }
        }
        changed
    }
}

fn capitalised(word: &str) -> String {
    let mut chars = word.chars();
    chars.next().map(|c| c.to_uppercase().chain(chars).collect()).unwrap_or_default()
}

/// Parse a date typed by the user: "24.10.2026", "24.10.", "2026-10-24", "24. Okt".
pub fn parse_typed(text: &str, today: NaiveDate) -> Option<(NaiveDate, Option<NaiveTime>)> {
    let folded: Vec<char> = fold(text.trim()).chars().collect();
    let (year, m, d, end) = date_at(&folded, 0)?;
    let date = match year {
        Some(y) => NaiveDate::from_ymd_opt(y, m, d)?,
        None => infer_year(m, d, today)?,
    };
    Some((date, time_after(&folded, end)))
}

/// A calendar entry (.ics) for a deadline, which Outlook, Google and Apple calendars import.
pub fn ics(deadline: &Deadline, summary: &str, reminder: &str) -> String {
    let escape = |t: &str| t.replace('\\', "\\\\").replace(';', "\\;").replace(',', "\\,").replace('\n', "\\n");
    let stamp = chrono::Utc::now().format("%Y%m%dT%H%M%SZ");
    let day = deadline.day().unwrap_or_else(|| Local::now().date_naive());
    let (start, end) = match deadline.clock() {
        Some(t) => {
            let at = day.and_time(t);
            (
                format!("DTSTART:{}", at.format("%Y%m%dT%H%M%S")),
                format!("DTEND:{}", (at + chrono::Duration::minutes(15)).format("%Y%m%dT%H%M%S")),
            )
        }
        None => (
            format!("DTSTART;VALUE=DATE:{}", day.format("%Y%m%d")),
            format!("DTEND;VALUE=DATE:{}", (day + chrono::Duration::days(1)).format("%Y%m%d")),
        ),
    };
    let mut description = deadline.module.clone();
    if !deadline.context.is_empty() {
        description.push_str(&format!("\n«{}»", deadline.context));
    }
    [
        "BEGIN:VCALENDAR".to_string(),
        "VERSION:2.0".to_string(),
        "PRODID:-//File Sorter//Deadlines//DE".to_string(),
        "BEGIN:VEVENT".to_string(),
        format!("UID:file-sorter-deadline-{}-{}@local", deadline.id, deadline.date),
        format!("DTSTAMP:{stamp}"),
        start,
        end,
        format!("SUMMARY:{}", escape(summary)),
        format!("DESCRIPTION:{}", escape(&description)),
        "BEGIN:VALARM".to_string(),
        "TRIGGER:-P1D".to_string(),
        "ACTION:DISPLAY".to_string(),
        format!("DESCRIPTION:{}", escape(reminder)),
        "END:VALARM".to_string(),
        "END:VEVENT".to_string(),
        "END:VCALENDAR".to_string(),
    ]
    .join("\r\n")
        + "\r\n"
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    fn first(text: &str) -> Option<(NaiveDate, Option<String>)> {
        find(text, day(2026, 10, 1)).first().map(|f| (f.date, f.time.map(|t| t.format("%H:%M").to_string())))
    }

    #[test]
    fn finds_german_and_english_deadlines() {
        let cases = [
            ("Abgabe: 24.10.2026, 23:59 Uhr", (day(2026, 10, 24), Some("23:59"))),
            ("Abgabe bis 24.10. um 18 Uhr", (day(2026, 10, 24), Some("18:00"))),
            ("Abgabetermin: Freitag, 6. November 2026", (day(2026, 11, 6), None)),
            ("Einsendeschluss ist der 3. Dez.", (day(2026, 12, 3), None)),
            ("Die Lösungen sind spätestens am 12.11.26 abzugeben", (day(2026, 11, 12), None)),
            ("Due: October 24th, 2026 at 11:59 pm", (day(2026, 10, 24), Some("23:59"))),
            ("Deadline: Oct 30", (day(2026, 10, 30), None)),
            ("Submission due 2026-11-02 17:00", (day(2026, 11, 2), Some("17:00"))),
            ("Hand in by 5 November", (day(2026, 11, 5), None)),
            ("Abgabe: 15.1.", (day(2027, 1, 15), None)),
        ];
        for (text, (date, time)) in cases {
            assert_eq!(first(text), Some((date, time.map(String::from))), "{text}");
        }
    }

    /// Texts as they come out of real PDFs (Word/LibreOffice, tables, LaTeX-like layouts).
    #[test]
    fn real_sheet_layouts() {
        type Expected<'a> = &'a [(NaiveDate, Option<&'a str>, Kind)];
        let cases: &[(&str, Expected)] = &[
            (
                "Übungsblatt 4\n\nAusgabe: 20.10.2026 \u{a0}\u{a0} Abgabe: 03.11.2026, 12:00 Uhr\n\nAufgabe 1",
                &[(day(2026, 11, 3), Some("12:00"), Kind::Due)],
            ),
            // A table: the column under «Abgabe», and «10.11» is no time.
            (
                "Serie 5\n\nAusgabe Abgabe Besprechung\n\n27.10.2026 10.11.2026 12.11.2026\n\nAufgabe 1",
                &[(day(2026, 11, 10), None, Kind::Due)],
            ),
            ("Die Lösungen sind bis zum 7.11. in der Vorlesung abzugeben.", &[(day(2026, 11, 7), None, Kind::Due)]),
            ("Bitte bis 9.11. 18 Uhr in Moodle hochladen.", &[(day(2026, 11, 9), Some("18:00"), Kind::Due)]),
            ("Abgabetermin\n\nDienstag\n10.11.2026", &[(day(2026, 11, 10), None, Kind::Due)]),
            ("Bearbeitungszeitraum: 27.10. – 10.11.2026", &[(day(2026, 11, 10), None, Kind::Due)]),
            ("Bearbeitungszeit 27.10. bis 10.11.", &[(day(2026, 11, 10), None, Kind::Due)]),
            (
                "Klausur: Mittwoch, 20.01.2027, 14:00–16:00, Raum HG E 7\n\nZwischenprüfung am 25.11.2026 um 10:15 Uhr.",
                &[(day(2026, 11, 25), Some("10:15"), Kind::Exam), (day(2027, 1, 20), Some("14:00"), Kind::Exam)],
            ),
            (
                "Präsentation der Projekte: 3. Dezember 2026, 13:15 Uhr. Testat am 26.11.2026.",
                &[(day(2026, 11, 26), None, Kind::Exam), (day(2026, 12, 3), Some("13:15"), Kind::Exam)],
            ),
            (
                "Prof. Dr. M. Huber HS 2026\nAlgorithmen Abgabe: 4.11.2026\n\nÜbungsblatt 6",
                &[(day(2026, 11, 4), None, Kind::Due)],
            ),
            // The date of another sentence is not the deadline.
            ("Abgabe in KW 45 über Moodle. Vorlesung vom 20.10.2026.", &[]),
            ("Diese Inhalte sind prüfungsrelevant (Stand 20.10.2026).", &[]),
        ];
        for (text, expected) in cases {
            let found: Vec<_> = find(text, day(2026, 10, 20))
                .into_iter()
                .map(|f| (f.date, f.time.map(|t| t.format("%H:%M").to_string()), f.kind))
                .collect();
            let expected: Vec<_> = expected.iter().map(|(d, t, k)| (*d, t.map(String::from), *k)).collect();
            assert_eq!(found, expected, "{text}");
        }
    }

    #[test]
    fn found_passages_start_and_end_with_whole_words() {
        let text = "Bitte laden Sie Ihre Lösung bis spätestens Montag, 2. November 2026, 23:59 Uhr in Moodle hoch.";
        let context = &find(text, day(2026, 10, 20))[0].context;
        assert!(text.split_whitespace().any(|w| context.starts_with(w)), "{context}");
    }

    #[test]
    fn corrected_and_exam_entries() {
        let mut list = Deadlines::default();
        let file = Path::new("/s/Stats/Organisatorisches.pdf");
        let found = find("Klausur: 20.01.2027, 14:00", day(2026, 10, 20));
        assert_eq!(list.add_found(file, 1, "Stats", found.clone()), 1);
        assert_eq!(list.items[0].title, "Klausur");
        // The user corrects the date: the one read before isn't added again.
        let id = list.items[0].id;
        list.edit(id, "Klausur Statistik", "Stats", Kind::Exam, day(2027, 1, 21), None);
        list.forget_scanned();
        assert_eq!(list.add_found(file, 1, "Stats", found), 0);
        assert_eq!(list.items[0].date, "2027-01-21");
    }

    #[test]
    fn ignores_dates_without_a_deadline_word_and_odd_numbers() {
        assert_eq!(first("Vorlesung vom 24.10.2026"), None);
        assert_eq!(first("Residue 24.10.2026"), None, "\"due\" inside a word");
        assert_eq!(first("Abgabe in Gruppen zu 3.5 Personen"), None);
        assert_eq!(first("Abgabe: siehe Moodle"), None);
        assert_eq!(first("Abgabe 24.10.2031"), None, "years away");
        assert_eq!(first("Abgabe 31.02.2026"), None, "no such day");
    }

    #[test]
    fn keeps_found_deadlines_once_and_remembers_deleted_ones() {
        let mut list = Deadlines::default();
        let file = Path::new("/s/Prog/Blatt3.pdf");
        let found = find("Abgabe: 24.10.2026", day(2026, 10, 1));
        assert_eq!(list.add_found(file, 10, "Prog", found.clone()), 1);
        assert!(list.was_scanned(file, 10) && !list.was_scanned(file, 11));
        list.items[0].deleted = true;
        assert_eq!(list.add_found(file, 11, "Prog", found), 0, "deleted stays deleted");
        let now = day(2026, 10, 20).and_hms_opt(12, 0, 0).unwrap();
        assert!(list.upcoming(now, 7, 14).is_empty());
        list.add_manual("Vortrag", "Stats", Kind::Exam, day(2026, 10, 22), None);
        list.add_manual("Später", "Stats", Kind::Due, day(2026, 12, 22), None);
        let soon = list.upcoming(now, 7, 14);
        assert_eq!(soon.iter().map(|d| d.title.as_str()).collect::<Vec<_>>(), ["Vortrag"]);
    }

    #[test]
    fn typed_dates_and_calendar_files() {
        let today = day(2026, 10, 19);
        assert_eq!(parse_typed("24.10.", today), Some((day(2026, 10, 24), None)));
        assert_eq!(parse_typed("22.10. 10:15", today), Some((day(2026, 10, 22), NaiveTime::from_hms_opt(10, 15, 0))));
        assert_eq!(parse_typed("22.10.26 9:00", today), Some((day(2026, 10, 22), NaiveTime::from_hms_opt(9, 0, 0))));
        assert_eq!(parse_typed("22.10. 2026", today).unwrap().0, day(2026, 10, 22));
        assert_eq!(parse_typed("2026-11-02 17:00", today).unwrap().1, NaiveTime::from_hms_opt(17, 0, 0));
        assert_eq!(parse_typed("morgen", today), None);
        let mut list = Deadlines::default();
        list.add_manual(
            "Blatt 3",
            "Informatik › Prog",
            Kind::Due,
            day(2026, 10, 24),
            NaiveTime::from_hms_opt(23, 59, 0),
        );
        let text = ics(&list.items[0], "Abgabe: Blatt 3, Prog", "Morgen fällig");
        assert!(text.contains("DTSTART:20261024T235900\r\n"));
        assert!(text.contains("SUMMARY:Abgabe: Blatt 3\\, Prog\r\n"));
        assert!(text.starts_with("BEGIN:VCALENDAR\r\n") && text.ends_with("END:VCALENDAR\r\n"));
    }
}

#[cfg(test)]
mod samples {
    /// Prints what is found in real PDFs: DEADLINE_SAMPLES=<folder> cargo test samples -- --ignored --nocapture
    #[test]
    #[ignore]
    fn print_samples() {
        let dir = std::env::var("DEADLINE_SAMPLES").unwrap();
        let mut files: Vec<_> = std::fs::read_dir(dir).unwrap().flatten().map(|e| e.path()).collect();
        files.sort();
        let reference = chrono::NaiveDate::from_ymd_opt(2026, 10, 20).unwrap();
        for f in files {
            let text = crate::extract::extract_text(&f);
            println!(
                "=== {}\n  text: {:?}",
                f.file_name().unwrap().to_string_lossy(),
                text.chars().take(240).collect::<String>()
            );
            for found in super::find(&text, reference) {
                println!("  -> {} {:?}  «{}»", found.date, found.time, found.context);
            }
        }
    }
}
