//! Submission deadlines found in exercise sheets ("Abgabe: 24.10.2026, 23:59",
//! "due Oct 24"), or added by hand, kept in `deadlines.json`.
//!
//! Finding dates in free text can go wrong, so every deadline can be deleted, and a
//! deleted one isn't found again in the same file.

use std::fs;
use std::path::{Path, PathBuf};

use chrono::{Datelike, Local, NaiveDate, NaiveDateTime, NaiveTime};
use serde::{Deserialize, Serialize};

use crate::index::fold;

pub const DEADLINES_FILE: &str = "deadlines.json";
/// Words that announce a deadline (folded: lower-case, no accents).
const TRIGGERS: &[&str] = &[
    "abgabe",
    "abzugeben",
    "einsendeschluss",
    "einreichen",
    "einzureichen",
    "deadline",
    "due",
    "submit",
    "submission",
    "hand in",
    "fallig",
    "faellig",
    "spatestens",
    "abgabetermin",
];
/// How far after the word the date may be.
const LOOK_AHEAD: usize = 90;
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

/// A time shortly after a date: "23:59", "23.59 Uhr", "18 Uhr", "11:59 pm".
fn time_after(chars: &[char], from: usize) -> Option<NaiveTime> {
    let limit = (from + 20).min(chars.len());
    let mut i = from;
    while i < limit {
        if let Some((h, e)) = number(chars, i, 2) {
            let (mut hour, mut minute, mut end) = (h, 0, e);
            if matches!(chars.get(e), Some(':' | '.'))
                && let Some((m, me)) = number(chars, e + 1, 2)
                && me - (e + 1) == 2
            {
                minute = m;
                end = me;
            } else {
                let (w, _) = word(chars, skip_spaces(chars, e));
                if w != "uhr" && w != "h" {
                    i = e;
                    continue;
                }
            }
            let (w, _) = word(chars, skip_spaces(chars, end));
            if w == "pm" && hour < 12 {
                hour += 12;
            } else if w == "am" && hour == 12 {
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

/// Deadlines in `text`: a deadline word followed closely by a date. `reference` is when
/// the file was made or downloaded, for dates without a year.
pub fn find(text: &str, reference: NaiveDate) -> Vec<Found> {
    let raw: Vec<char> = text.chars().collect();
    let folded: Vec<char> = fold(text).chars().collect();
    let haystack: String = folded.iter().collect();
    let mut found: Vec<Found> = Vec::new();
    for trigger in TRIGGERS {
        for (byte, _) in haystack.match_indices(trigger) {
            let start = haystack[..byte].chars().count();
            // Whole words only ("due" not in "residue", "abgabe" may start "Abgabetermin").
            if start > 0 && folded[start - 1].is_alphanumeric() {
                continue;
            }
            let end = (start + trigger.chars().count() + LOOK_AHEAD).min(folded.len());
            let mut i = start + trigger.chars().count();
            while i < end {
                let boundary = i == 0 || !folded[i - 1].is_alphanumeric();
                if boundary && let Some((year, m, d, date_end)) = date_at(&folded, i) {
                    let date = match year {
                        Some(y) => NaiveDate::from_ymd_opt(y, m, d),
                        None => infer_year(m, d, reference),
                    };
                    if let Some(date) = date.filter(|date| (*date - reference).num_days().abs() <= 366) {
                        let from = start.saturating_sub(CONTEXT / 3);
                        let to = (date_end + CONTEXT / 2).min(raw.len());
                        let context: String =
                            raw[from..to].iter().collect::<String>().split_whitespace().collect::<Vec<_>>().join(" ");
                        if !found.iter().any(|f| f.date == date) {
                            found.push(Found { date, time: time_after(&folded, date_end), context });
                        }
                    }
                    break;
                }
                i += 1;
            }
        }
    }
    found.sort_by_key(|f| (f.date, f.time));
    found
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Deadlines {
    pub items: Vec<Deadline>,
    /// Files already looked at (path and size), so they aren't read again.
    scanned: Vec<(PathBuf, u64)>,
}

impl Deadlines {
    pub fn load(file: &Path) -> Deadlines {
        fs::read_to_string(file).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
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
        let title = file.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let mut added = 0;
        for f in found {
            let date = f.date.format("%Y-%m-%d").to_string();
            let known = self.items.iter().any(|d| d.file.as_deref() == Some(file) && d.date == date);
            if known {
                continue;
            }
            let id = self.next_id();
            self.items.push(Deadline {
                id,
                title: title.clone(),
                module: module.to_string(),
                file: Some(file.to_path_buf()),
                date,
                time: f.time.map(|t| t.format("%H:%M").to_string()),
                context: f.context,
                done: false,
                deleted: false,
            });
            added += 1;
        }
        added
    }

    pub fn add_manual(&mut self, title: &str, module: &str, date: NaiveDate, time: Option<NaiveTime>) -> u64 {
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
        });
        id
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
        list.add_manual("Vortrag", "Stats", day(2026, 10, 22), None);
        list.add_manual("Später", "Stats", day(2026, 12, 22), None);
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
        list.add_manual("Blatt 3", "Informatik › Prog", day(2026, 10, 24), NaiveTime::from_hms_opt(23, 59, 0));
        let text = ics(&list.items[0], "Abgabe: Blatt 3, Prog", "Morgen fällig");
        assert!(text.contains("DTSTART:20261024T235900\r\n"));
        assert!(text.contains("SUMMARY:Abgabe: Blatt 3\\, Prog\r\n"));
        assert!(text.starts_with("BEGIN:VCALENDAR\r\n") && text.ends_with("END:VCALENDAR\r\n"));
    }
}
