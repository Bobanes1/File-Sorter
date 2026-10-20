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
const FINDER_VERSION: u32 = 3;

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

/// How a deadline word may be written.
#[derive(Clone, Copy, PartialEq)]
enum Fit {
    /// Only as a word of its own ("due", not "residue").
    Word,
    /// May go on ("deadline" in "deadlines", "submit" in "submitted").
    Starts,
    /// German compounds: may end a longer word ("Projektabgabe", "Semesterendprüfung") and
    /// go on with a few endings ("Abgabetermin", "Prüfungsdatum"), but not with any other
    /// ("prüfungsrelevant", "Prüfungsordnung").
    Compound,
    /// Only as the end of a longer word ("Zwischentest", not "Test").
    Tail,
}

/// Words (folded: lower-case, no accents) that announce a date, what kind it is, where the
/// date is, and how the word may be written.
const TRIGGERS: &[(&str, Kind, Look, Fit)] = &[
    ("abgabe", Kind::Due, Look::Ahead, Fit::Compound),
    ("einsendeschluss", Kind::Due, Look::Ahead, Fit::Word),
    ("einreichung", Kind::Due, Look::Ahead, Fit::Word),
    ("frist", Kind::Due, Look::Ahead, Fit::Compound),
    ("deadline", Kind::Due, Look::Ahead, Fit::Starts),
    ("due", Kind::Due, Look::Ahead, Fit::Word),
    ("submit", Kind::Due, Look::Ahead, Fit::Starts),
    ("submission", Kind::Due, Look::Ahead, Fit::Starts),
    ("upload", Kind::Due, Look::Ahead, Fit::Starts),
    ("hand in", Kind::Due, Look::Ahead, Fit::Word),
    ("turn in", Kind::Due, Look::Ahead, Fit::Word),
    ("spatestens", Kind::Due, Look::Ahead, Fit::Word),
    ("fallig", Kind::Due, Look::Ahead, Fit::Starts),
    ("faellig", Kind::Due, Look::Ahead, Fit::Word),
    ("abzugeben", Kind::Due, Look::Behind, Fit::Word),
    ("abgeben", Kind::Due, Look::Behind, Fit::Word),
    ("abgegeben", Kind::Due, Look::Behind, Fit::Word),
    ("einzureichen", Kind::Due, Look::Behind, Fit::Word),
    ("einreichen", Kind::Due, Look::Behind, Fit::Word),
    ("eingereicht", Kind::Due, Look::Behind, Fit::Word),
    ("hochzuladen", Kind::Due, Look::Behind, Fit::Word),
    ("hochladen", Kind::Due, Look::Behind, Fit::Word),
    ("hochgeladen", Kind::Due, Look::Behind, Fit::Word),
    ("einzuwerfen", Kind::Due, Look::Behind, Fit::Word),
    ("einwerfen", Kind::Due, Look::Behind, Fit::Word),
    ("eingeworfen", Kind::Due, Look::Behind, Fit::Word),
    ("abzuliefern", Kind::Due, Look::Behind, Fit::Word),
    ("abliefern", Kind::Due, Look::Behind, Fit::Word),
    ("einzusenden", Kind::Due, Look::Behind, Fit::Word),
    ("bearbeitungszeit", Kind::Due, Look::Period, Fit::Starts),
    ("klausur", Kind::Exam, Look::Ahead, Fit::Compound),
    ("prufung", Kind::Exam, Look::Ahead, Fit::Compound),
    ("pruefung", Kind::Exam, Look::Ahead, Fit::Compound),
    ("exam", Kind::Exam, Look::Ahead, Fit::Word),
    ("exams", Kind::Exam, Look::Ahead, Fit::Word),
    ("examination", Kind::Exam, Look::Ahead, Fit::Word),
    ("midterm", Kind::Exam, Look::Ahead, Fit::Starts),
    ("quiz", Kind::Exam, Look::Ahead, Fit::Word),
    ("test", Kind::Exam, Look::Ahead, Fit::Tail),
    ("testat", Kind::Exam, Look::Ahead, Fit::Word),
    ("kolloquium", Kind::Exam, Look::Ahead, Fit::Word),
    ("leistungskontrolle", Kind::Exam, Look::Ahead, Fit::Word),
    ("lernkontrolle", Kind::Exam, Look::Ahead, Fit::Word),
    ("leistungsnachweis", Kind::Exam, Look::Ahead, Fit::Word),
    ("prasentation", Kind::Exam, Look::Ahead, Fit::Compound),
    ("presentation", Kind::Exam, Look::Ahead, Fit::Word),
    ("vortrag", Kind::Exam, Look::Ahead, Fit::Word),
];
/// How a compound may go on ("Abgabe|termin", "Prüfung|sdatum", "Frist|ende").
const COMPOUND_ENDINGS: &[&str] = &[
    "n", "en", "s", "termin", "termine", "stermin", "stermine", "datum", "sdatum", "tag", "stag", "frist", "sfrist",
    "ende",
];
/// Compounds that are something else ("Überprüfung", "Wärmeabgabe", "Selbsttest", "Protest").
const NOT_COMPOUNDS: &[&str] =
    &["uber", "ueber", "warme", "energie", "stimm", "steuer", "selbst", "gast", "pro", "con"];
/// A date right after one of these is a number of something ("Aufgabe 3.2.", "Kapitel 4.1.").
const NUMBERED: &[&str] = &[
    "aufgabe",
    "aufgaben",
    "teilaufgabe",
    "ubung",
    "uebung",
    "kapitel",
    "kap",
    "abschnitt",
    "seite",
    "seiten",
    "s",
    "nr",
    "satz",
    "lemma",
    "theorem",
    "definition",
    "def",
    "korollar",
    "bsp",
    "beispiel",
    "exercise",
    "exercises",
    "section",
    "chapter",
    "problem",
    "question",
    "task",
    "punkte",
    "punkt",
    "gleichung",
    "abbildung",
    "abb",
    "tabelle",
    "tab",
    "folie",
    "folien",
    "slide",
    "slides",
    "version",
    "v",
    "um",
];
/// A date right after one of these is when something was made, not when it is due.
const NOT_DUE: &[&str] = &[
    "stand",
    "ausgabe",
    "ausgegeben",
    "erstellt",
    "aktualisiert",
    "veroffentlicht",
    "released",
    "published",
    "issued",
    "updated",
    "posted",
    "beginn",
    "abgabebeginn",
    "geoffnet",
    "opened",
    "opens",
    "start",
];
/// Words in a sentence that say something is handed in ("Bitte bis 9.11. in Moodle hochladen").
const HAND_IN_WORDS: &[&str] = &[
    "hochlad",
    "hochzulad",
    "abgeb",
    "abzugeb",
    "abgegeb",
    "einreich",
    "einzureich",
    "eingereich",
    "einwerf",
    "einzuwerf",
    "eingeworf",
    "briefkasten",
    "ablief",
    "abzulief",
    "einsend",
    "einzusend",
    "upload",
    "submit",
    "losung",
    "loesung",
    "solution",
];
/// Short weekdays ("Mo, 3.11.").
const SHORT_WEEKDAYS: &[&str] =
    &["mo", "di", "mi", "do", "fr", "sa", "so", "mon", "tue", "wed", "thu", "fri", "sat", "sun"];
/// Weekdays, for "Abgabe: Montag, 10:15 Uhr" (no date: the next Monday).
const WEEKDAYS: &[(&str, u32)] = &[
    ("montag", 0),
    ("dienstag", 1),
    ("mittwoch", 2),
    ("donnerstag", 3),
    ("freitag", 4),
    ("samstag", 5),
    ("sonntag", 6),
    ("monday", 0),
    ("tuesday", 1),
    ("wednesday", 2),
    ("thursday", 3),
    ("friday", 4),
    ("saturday", 5),
    ("sunday", 6),
];
/// Names of what is handed in, for a title: "Übungsblatt 3", "Serie 5", "Teil A" need a
/// number or letter after them, "Projekt" or "Essay" don't.
const NUMBERED_WORKS: &[&str] = &[
    "ubungsblatt",
    "uebungsblatt",
    "aufgabenblatt",
    "ubungsserie",
    "blatt",
    "serie",
    "ubung",
    "uebung",
    "hausaufgabe",
    "hausaufgaben",
    "hausubung",
    "assignment",
    "homework",
    "sheet",
    "lab",
    "praktikum",
    "teil",
    "part",
    "tutorium",
    "meilenstein",
    "milestone",
    "exercise",
    "set",
];
const WORKS: &[&str] = &[
    "projekt",
    "project",
    "seminararbeit",
    "hausarbeit",
    "essay",
    "bericht",
    "laborbericht",
    "report",
    "poster",
    "protokoll",
    "portfolio",
    "bachelorarbeit",
    "masterarbeit",
    "semesterarbeit",
    "projektarbeit",
];
/// How far from the word the date may be (within the same sentence, over at most a few lines).
const LOOK_AHEAD: usize = 160;
const LOOK_BEHIND: usize = 120;
const MAX_LINE_BREAKS: usize = 4;
/// How far a weekday or relative date («in zwei Wochen») may be from the word.
const WEEKDAY_AHEAD: usize = 50;
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
    /// Only a weekday was given ("Abgabe: Montag"): the date is the next one after the file came.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub estimated: bool,
    /// When it was found or added (Unix seconds), to mark new entries.
    #[serde(default)]
    pub added: i64,
    /// The reminders already sent («day», «hours3», …).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reminded: Vec<String>,
    /// Comes again (every week, …): `date` is the next time it is due.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repeat: Option<Repeat>,
    /// Found by the AI model.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub ai: bool,
}

/// How often a deadline comes again.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Every {
    Week,
    TwoWeeks,
    Month,
}

impl Every {
    pub const ALL: [Every; 3] = [Every::Week, Every::TwoWeeks, Every::Month];

    pub fn after(self, date: NaiveDate) -> Option<NaiveDate> {
        match self {
            Every::Week => Some(date + chrono::Duration::weeks(1)),
            Every::TwoWeeks => Some(date + chrono::Duration::weeks(2)),
            Every::Month => date.checked_add_months(chrono::Months::new(1)),
        }
    }
}

/// A deadline that comes again, until a last date (or for good).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Repeat {
    pub every: Every,
    /// `YYYY-MM-DD`: the last time it can be due.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub until: Option<String>,
}

impl Repeat {
    pub fn until_day(&self) -> Option<NaiveDate> {
        NaiveDate::parse_from_str(self.until.as_deref()?, "%Y-%m-%d").ok()
    }
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
    /// What is due, if the text says it ("Serie 5", "Blatt 3", "Projektabgabe").
    pub title: Option<String>,
    /// Only a weekday was given: the date is the next such day.
    pub estimated: bool,
    /// Found by the AI model (in a sentence the rules here don't know).
    pub ai: bool,
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
    // French, Italian and Spanish names the prefixes above don't cover (whole words only:
    // «gen…» or «set…» alone would match too much).
    const OTHER: &[(&str, u32)] = &[
        ("gennaio", 1),
        ("enero", 1),
        ("fevrier", 2),
        ("avril", 4),
        ("abril", 4),
        ("maggio", 5),
        ("juin", 6),
        ("giugno", 6),
        ("juillet", 7),
        ("luglio", 7),
        ("aout", 8),
        ("agosto", 8),
        ("settembre", 9),
        ("ottobre", 10),
        ("dicembre", 12),
        ("diciembre", 12),
    ];
    if word.len() < 3 || !word.chars().all(|c| c.is_ascii_alphabetic()) {
        return None;
    }
    if let Some((_, m)) = OTHER.iter().find(|(name, _)| word == *name) {
        return Some(*m);
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

fn is_weekday(word: &str) -> bool {
    WEEKDAYS.iter().any(|(w, _)| *w == word) || SHORT_WEEKDAYS.contains(&word)
}

/// The word just before `i`, over spaces, one mark and a weekday ("Stand: 20.10." and
/// "Abgabebeginn: Dienstag, 21. Oktober" give "stand" and "abgabebeginn").
fn word_before(folded: &[char], i: usize) -> String {
    let (word, start) = word_before_at(folded, i);
    if is_weekday(&word) { word_before_at(folded, start).0 } else { word }
}

fn word_before_at(folded: &[char], i: usize) -> (String, usize) {
    let mut j = i;
    while j > 0 && folded[j - 1] == ' ' {
        j -= 1;
    }
    if j > 0 && matches!(folded[j - 1], ':' | '.' | ',' | '(') {
        j -= 1;
        while j > 0 && folded[j - 1] == ' ' {
            j -= 1;
        }
    }
    let end = j;
    while j > 0 && folded[j - 1].is_alphabetic() {
        j -= 1;
    }
    (folded[j..end].iter().collect(), j)
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
    // A time right after it ("24.10. 10:15", "3.11 um") is no year; "12.10 Uhr" is no date.
    let time_follows =
        |end: usize| matches!(chars.get(end), Some(':' | '.')) && chars.get(end + 1).is_some_and(char::is_ascii_digit);
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
        // 24/10/2026, 24/10/26, 11/28/2026 (the order is clear when one is above 12).
        if chars.get(end) == Some(&'/') {
            let (second, e) = number(chars, end + 1, 2)?;
            if chars.get(e) != Some(&'/') {
                return None;
            }
            let (y, ye) = number(chars, e + 1, 4)?;
            let year = match ye - (e + 1) {
                4 => y as i32,
                2 => 2000 + y as i32,
                _ => return None,
            };
            let (m, d) = if second > 12 { (first, second) } else { (second, first) };
            return Some((Some(year), m, d, ye));
        }
        let day = first;
        // 24.10. / 24.10.2026 / 24.10.26 / 3.11
        if chars.get(end) == Some(&'.') {
            let j = skip_spaces(chars, end + 1);
            if let Some((m, e)) = number(chars, j, 2) {
                if chars.get(e) == Some(&'.') {
                    let k = skip_spaces(chars, e + 1);
                    return match number(chars, k, 4) {
                        Some((y, ye)) if ye - k == 4 && !time_follows(ye) => Some((Some(y as i32), m, day, ye)),
                        Some((y, ye)) if ye - k == 2 && k == e + 1 && !time_follows(ye) => {
                            Some((Some(2000 + y as i32), m, day, ye))
                        }
                        _ => Some((None, m, day, e + 1)),
                    };
                }
                // "3.11" without the last dot: only with a two-digit month, so "3.5 Personen"
                // stays a number, and not "12.10 Uhr" (a time) or "3.11.5" (a section).
                let unit = word(chars, skip_spaces(chars, e)).0;
                if j == end + 1
                    && e - j == 2
                    && !time_follows(e)
                    && !matches!(chars.get(e), Some(',' | '/'))
                    && !["uhr", "h"].contains(&unit.as_str())
                {
                    return Some((None, m, day, e));
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

/// A time shortly after a date: "23:59", "23.59 Uhr", "18 Uhr", "11:59 pm", "5pm", "von 14
/// bis 16 Uhr" (the start). Not another date ("10.11.2026" is no time 10:11), and not past
/// the end of the sentence.
fn time_after(chars: &[char], from: usize) -> Option<NaiveTime> {
    time_in(chars, from, (from + 24).min(chars.len()))
}

/// The first time that starts between `from` and `to`.
fn time_in(chars: &[char], from: usize, to: usize) -> Option<NaiveTime> {
    let mut i = from;
    while i < to.min(chars.len()) {
        if chars[i] == '.' && chars.get(i + 1) == Some(&' ') && chars.get(i + 2).is_some_and(|c| c.is_alphabetic()) {
            return None;
        }
        if let Some((_, e)) = number(chars, i, 2) {
            if let Some((time, _)) = time_at(chars, i) {
                return Some(time);
            }
            i = e;
            continue;
        }
        i += 1;
    }
    None
}

/// A time written right before a date: "bis 12 Uhr am 28.10.", "18:00, 3.11.".
fn time_before(chars: &[char], date: usize, stops: &[bool]) -> Option<NaiveTime> {
    let mut from = date;
    while from > 0 && date - from < 22 && !stops[from - 1] {
        from -= 1;
    }
    (from..date).find_map(|i| {
        let (time, end) = time_at(chars, i).filter(|&(_, end)| end <= date)?;
        let gap: String = chars[end..date].iter().collect();
        gap.split(|c: char| !c.is_alphanumeric())
            .filter(|w| !w.is_empty())
            .all(|w| ["am", "on", "den", "the"].contains(&w) || is_weekday(w))
            .then_some(time)
    })
}

/// A time at `i` and where it ends (after "Uhr" or "pm").
fn time_at(chars: &[char], i: usize) -> Option<(NaiveTime, usize)> {
    let after_word = |at: usize| word(chars, skip_spaces(chars, at));
    let (h, e) = number(chars, i, 2)?;
    if i > 0 && chars[i - 1].is_alphanumeric() {
        return None;
    }
    let (mut hour, mut minute, mut end) = (h, 0, e);
    match chars.get(e) {
        Some(':') if let Some((m, me)) = number(chars, e + 1, 2).filter(|&(_, me)| me - (e + 1) == 2) => {
            minute = m;
            end = me;
        }
        // "23.59 Uhr", but "10.11." or "10.11.2026" is a date.
        Some('.')
            if let Some((m, me)) = number(chars, e + 1, 2).filter(|&(_, me)| me - (e + 1) == 2)
                && chars.get(me) != Some(&'.')
                && ["uhr", "h"].contains(&after_word(me).0.as_str()) =>
        {
            minute = m;
            end = me;
        }
        _ if ["uhr", "h", "pm"].contains(&after_word(e).0.as_str()) => {}
        // "von 14 bis 16 Uhr", "14–16 Uhr": the start.
        _ if range_to_hour(chars, e) => {}
        _ => return None,
    }
    let (suffix, suffix_end) = after_word(end);
    if suffix == "pm" && hour < 12 {
        hour += 12;
    } else if suffix == "am" && hour == 12 {
        hour = 0;
    }
    if ["uhr", "h", "pm"].contains(&suffix.as_str()) {
        end = suffix_end;
    }
    Some((NaiveTime::from_hms_opt(hour, minute, 0)?, end))
}

/// Whether "bis 16 Uhr" or "–16 Uhr" follows at `at`.
fn range_to_hour(chars: &[char], at: usize) -> bool {
    let mut j = skip_spaces(chars, at);
    match chars.get(j) {
        Some('-' | '–') => j += 1,
        _ if word(chars, j).0 == "bis" => j = word(chars, j).1,
        _ => return false,
    }
    let j = skip_spaces(chars, j);
    number(chars, j, 2).is_some_and(|(_, e)| ["uhr", "h"].contains(&word(chars, skip_spaces(chars, e)).0.as_str()))
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
/// letter. A dot after a number ("2. November", "3.11.") or a short word or abbreviation
/// ("Prof. Dr. Huber", "Mo. 3.11.", "Kap. 2") doesn't end one.
fn sentence_ends(raw: &[char]) -> Vec<bool> {
    const ABBREVIATIONS: &[&str] = &["prof", "inkl", "evtl", "bspw", "sept", "zzgl", "jahrg", "gebr", "abs"];
    let mut ends = vec![false; raw.len()];
    for i in 1..raw.len() {
        if !matches!(raw[i], '.' | '!' | '?') || !raw[i - 1].is_alphabetic() {
            continue;
        }
        let next = raw[i + 1..].iter().position(|c| !c.is_whitespace()).map(|n| raw[i + 1 + n]);
        if !raw.get(i + 1).is_some_and(|c| c.is_whitespace()) || !next.is_some_and(char::is_uppercase) {
            continue;
        }
        let start = raw[..i].iter().rposition(|c| !c.is_alphabetic()).map_or(0, |p| p + 1);
        let before: String = raw[start..i].iter().collect::<String>().to_lowercase();
        let short = before.chars().count() <= 3 && before != "uhr";
        if raw[i] == '.' && (short || ABBREVIATIONS.contains(&before.as_str())) {
            continue;
        }
        ends[i] = true;
    }
    ends
}

/// Where the search for a date stops: the end of the sentence, the start of the exercises
/// ("Aufgabe 1"), or after a few lines.
fn stops(raw: &[char], folded: &[char]) -> Vec<bool> {
    const STARTS: &[&str] = &["aufgabe", "exercise", "problem", "question", "task"];
    let mut stops = sentence_ends(raw);
    for i in 1..raw.len() {
        if raw[i - 1] == '\n' && !raw[i].is_whitespace() && STARTS.contains(&word(folded, i).0.as_str()) {
            stops[i] = true;
        }
    }
    stops
}

/// The date written at `i` (with the year worked out), and where it ends. Not a number of
/// something ("Aufgabe 3.2.").
fn full_date(folded: &[char], i: usize, reference: NaiveDate) -> Option<(NaiveDate, usize)> {
    if i > 0 && folded[i - 1].is_alphanumeric() {
        return None;
    }
    if !folded.get(i).is_some_and(|c| c.is_alphanumeric()) {
        return None;
    }
    let (year, m, d, end) = date_at(folded, i)?;
    if folded[i].is_ascii_digit() && NUMBERED.contains(&word_before(folded, i).as_str()) {
        return None;
    }
    let date = match year {
        Some(y) => NaiveDate::from_ymd_opt(y, m, d)?,
        None => infer_year(m, d, reference)?,
    };
    Some((date, end))
}

/// All dates on a stretch of text: (date, start, end).
fn dates_in(folded: &[char], from: usize, to: usize, reference: NaiveDate) -> Vec<(NaiveDate, usize, usize)> {
    let mut dates = Vec::new();
    let mut i = from;
    while i < to {
        match full_date(folded, i, reference) {
            Some((date, end)) if end <= to => {
                dates.push((date, i, end));
                i = end;
            }
            _ => i += 1,
        }
    }
    dates
}

/// Lines of `raw` as (start, end), without the empty ones.
fn lines(raw: &[char]) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut start = 0;
    for i in 0..=raw.len() {
        if i == raw.len() || raw[i] == '\n' {
            if raw[start..i].iter().any(|c| !c.is_whitespace()) {
                out.push((start, i));
            }
            start = i + 1;
        }
    }
    out
}

/// A table: the word is one of the column titles on its line ("Blatt  Ausgabe  Abgabe") and
/// the lines below hold one date per date column ("Blatt 1  20.10.  27.10."). Returns the
/// date of the word's column in each row, and the row's name ("Blatt 1").
fn table_rows(
    raw: &[char],
    folded: &[char],
    at: usize,
    reference: NaiveDate,
) -> Vec<(NaiveDate, usize, usize, Option<String>)> {
    const DATE_COLUMNS: &[&str] = &[
        "ausgabe",
        "abgabe",
        "besprechung",
        "ruckgabe",
        "korrektur",
        "datum",
        "termin",
        "deadline",
        "due",
        "released",
        "release",
        "issued",
        "out",
        "frist",
        "beginn",
        "start",
        "ende",
    ];
    let all = lines(raw);
    let Some(row) = all.iter().position(|&(s, e)| s <= at && at < e) else { return Vec::new() };
    let (start, end) = all[row];
    let titles: Vec<(usize, String)> =
        word_starts(raw, start, end).into_iter().map(|s| (s, word(folded, s).0)).collect();
    if titles.len() < 2 || titles.len() > 8 || !dates_in(folded, start, end, reference).is_empty() {
        return Vec::new();
    }
    let is_date_column = |(s, w): &(usize, String)| *s == at || DATE_COLUMNS.iter().any(|c| w.starts_with(c));
    let columns: Vec<usize> = titles.iter().filter(|t| is_date_column(t)).map(|(s, _)| *s).collect();
    let Some(column) = columns.iter().position(|&s| s == at) else { return Vec::new() };
    let mut rows = Vec::new();
    let mut misses = 0;
    for &(s, e) in all.iter().skip(row + 1).take(40) {
        let dates = dates_in(folded, s, e, reference);
        if dates.len() != columns.len() {
            misses += 1;
            if misses >= 2 {
                break;
            }
            continue;
        }
        misses = 0;
        let label: String = raw[s..dates[0].1].iter().collect();
        if columns.len() == 1 && label.contains(':') {
            // "Abgabebeginn: 21.10." is a list of dates, not a table.
            return Vec::new();
        }
        // "Serie 1 Mo" without the weekday of the date.
        let mut words: Vec<&str> = label.split_whitespace().collect();
        while words.last().is_some_and(|w| is_weekday(&fold(w.trim_end_matches([',', '.'])))) {
            words.pop();
        }
        let label = words.join(" ").trim_end_matches([':', '-', '|', '–']).trim().to_string();
        let (date, ds, de) = dates[column];
        rows.push((date, ds, de, (!label.is_empty() && label.chars().count() <= 40).then_some(label)));
    }
    // One date column could be any list: then it takes a short header and two rows.
    if columns.len() == 1 && (titles.len() > 4 || rows.len() < 2) {
        return Vec::new();
    }
    rows
}

fn word_starts(raw: &[char], from: usize, to: usize) -> Vec<usize> {
    (from..to).filter(|&i| !raw[i].is_whitespace() && (i == from || raw[i - 1].is_whitespace())).collect()
}

/// Whether the trigger found at `start..after` stands as it may be written; returns where its
/// whole word starts and ends.
fn fits(folded: &[char], start: usize, after: usize, fit: Fit) -> Option<(usize, usize)> {
    let mut word_start = start;
    while word_start > 0 && folded[word_start - 1].is_alphabetic() {
        word_start -= 1;
    }
    let mut word_end = after;
    while word_end < folded.len() && folded[word_end].is_alphabetic() {
        word_end += 1;
    }
    let digit_next = folded.get(word_end).is_some_and(char::is_ascii_digit) && word_end == after;
    let glued = word_start > 0 && folded[word_start - 1].is_ascii_digit();
    let prefix: String = folded[word_start..start].iter().collect();
    let ending: String = folded[after..word_end].iter().collect();
    let prefix_ok = |min: usize| prefix.chars().count() >= min && !NOT_COMPOUNDS.iter().any(|n| prefix.ends_with(n));
    let ending_ok = ending.is_empty() || COMPOUND_ENDINGS.contains(&ending.as_str());
    let ok = match fit {
        Fit::Word => prefix.is_empty() && ending.is_empty() && !digit_next,
        Fit::Starts => prefix.is_empty(),
        Fit::Compound => (prefix.is_empty() || prefix_ok(3)) && ending_ok,
        Fit::Tail => prefix_ok(4) && ending_ok,
    };
    (ok && !glued).then_some((word_start, word_end))
}

/// The name of what is handed in, in a stretch of text: "Übungsblatt 3", "Serie 5",
/// "Problem Set 4", "Teil A", or without a number "Projekt" (then `false`).
fn work_name(raw: &[char], from: usize, to: usize) -> Option<(String, bool)> {
    let text: String = raw[from..to.min(raw.len())].iter().collect();
    // Words and numbers, "Blatt7" split in two.
    let mut tokens: Vec<String> = Vec::new();
    for piece in text.split(|c: char| !c.is_alphanumeric()) {
        let split = piece.find(|c: char| c.is_ascii_digit()).filter(|&p| p > 0);
        match split {
            Some(p) if piece[p..].chars().all(|c| c.is_ascii_digit()) => {
                tokens.push(piece[..p].to_string());
                tokens.push(piece[p..].to_string());
            }
            _ if !piece.is_empty() => tokens.push(piece.to_string()),
            _ => {}
        }
    }
    let is_number = |t: &str| {
        (t.len() <= 3 && t.chars().all(|c| c.is_ascii_digit()))
            || (t.len() == 1 && t.chars().all(|c| c.is_ascii_uppercase()))
            || (t.len() <= 4 && t.chars().all(|c| "IVX".contains(c)))
    };
    for (i, token) in tokens.iter().enumerate() {
        let folded = fold(token);
        let next = tokens.get(i + 1).map(String::as_str).unwrap_or("");
        if NUMBERED_WORKS.contains(&folded.as_str()) && is_number(next) {
            // "Problem Set 4", "Exercise Sheet 2"
            let first = match tokens.get(i.wrapping_sub(1)).map(|t| fold(t)) {
                Some(before) if i > 0 && ["problem", "exercise", "ubungs"].contains(&before.as_str()) => i - 1,
                _ => i,
            };
            return Some((format!("{} {next}", tokens[first..=i].join(" ")), true));
        }
    }
    let token = tokens.iter().find(|t| WORKS.contains(&fold(t).as_str()))?;
    Some((capitalised(token), false))
}

/// The name of what is handed in, from the first lines of the document ("Analysis I – Serie
/// 5" gives "Serie 5").
fn document_work(raw: &[char]) -> Option<(String, bool)> {
    let names: Vec<_> = lines(raw).into_iter().take(6).filter_map(|(s, e)| work_name(raw, s, e)).collect();
    names.iter().find(|(_, numbered)| *numbered).or(names.first()).cloned()
}

/// What is due: a numbered name near the date ("Teil A"), a compound word ("Projektabgabe"),
/// a numbered name at the top of the document ("Serie 5"), then names without number.
fn due_title(near: Option<(String, bool)>, named: Option<String>, document: &Option<(String, bool)>) -> Option<String> {
    let numbered = |n: &Option<(String, bool)>| n.as_ref().filter(|(_, numbered)| *numbered).map(|(t, _)| t.clone());
    numbered(&near)
        .or(named)
        .or_else(|| numbered(document))
        .or_else(|| near.map(|(t, _)| t))
        .or_else(|| document.as_ref().map(|(t, _)| t.clone()))
}

/// The passage around `a..b`, from and to whole words.
fn passage(raw: &[char], a: usize, b: usize) -> String {
    let mut from = a.saturating_sub(CONTEXT / 3);
    while from > 0 && from < a && !raw[from - 1].is_whitespace() {
        from += 1;
    }
    let mut to = (b + CONTEXT / 2).min(raw.len());
    while to < raw.len() && to > b && !raw[to].is_whitespace() {
        to -= 1;
    }
    raw[from..to].iter().collect::<String>().split_whitespace().collect::<Vec<_>>().join(" ")
}

/// How far around a date the sentence asked about reaches (in characters, each way).
const SENTENCE_REACH: usize = 140;
/// At most this many dated sentences of a document are asked about.
const MAX_ASKED: usize = 40;

/// Every full date in `text` (within a year of `reference`), with the sentence it is in and
/// a time right after it.
pub fn dated_sentences(text: &str, reference: NaiveDate) -> Vec<(NaiveDate, Option<NaiveTime>, String)> {
    let raw: Vec<char> = text.chars().collect();
    let folded: Vec<char> = fold(text).chars().collect();
    // The usual sentence ends, and also every «. » before a capital (an abbreviation cut
    // off only shortens what is asked about; «18h.» must end a sentence).
    let mut stops = stops(&raw, &folded);
    for i in 0..raw.len() {
        let next = raw[i + 1..].iter().position(|c| !c.is_whitespace()).map(|n| raw[i + 1 + n]);
        if matches!(raw[i], '.' | '!' | '?')
            && raw.get(i + 1).is_some_and(|c| c.is_whitespace())
            && next.is_some_and(char::is_uppercase)
        {
            stops[i] = true;
        }
    }
    let mut out = Vec::new();
    for (date, start, end) in dates_in(&folded, 0, folded.len(), reference) {
        if (date - reference).num_days().abs() > 366 {
            continue;
        }
        let mut from = start;
        while from > 0 && start - from < SENTENCE_REACH && !stops[from - 1] {
            from -= 1;
        }
        let mut to = end;
        while to < raw.len() && to - end < SENTENCE_REACH && !stops[to] {
            to += 1;
        }
        let sentence =
            raw[from..to.min(raw.len())].iter().collect::<String>().split_whitespace().collect::<Vec<_>>().join(" ");
        out.push((date, time_after(&folded, end), sentence));
    }
    out
}

/// What [`find`] finds, checked by the AI model when it is there: dates in sentences the
/// rules here don't know (other languages, unusual wording) that it is sure are deadlines or
/// exams are added, and found dates it is very sure are something else are left out.
pub fn find_with_ai(text: &str, reference: NaiveDate) -> Vec<Found> {
    let mut found = find(text, reference);
    if !crate::laya::available() {
        return found;
    }
    let mut sentences = dated_sentences(text, reference);
    sentences.truncate(MAX_ASKED);
    if sentences.is_empty() {
        return found;
    }
    let texts: Vec<String> = sentences.iter().map(|(_, _, s)| s.clone()).collect();
    let Some(kinds) = crate::laya::date_kinds(&texts) else { return found };
    use crate::laya::DateKind;
    // A date the rules found, but every sentence it is in is clearly about something else.
    found.retain(|f| {
        let verdicts: Vec<(DateKind, f64)> =
            sentences.iter().zip(&kinds).filter(|((d, _, _), _)| *d == f.date).map(|(_, k)| *k).collect();
        verdicts.is_empty() || !verdicts.iter().all(|(k, p)| *k == DateKind::Other && *p >= 0.9)
    });
    for ((date, time, sentence), (kind, p)) in sentences.into_iter().zip(kinds) {
        let kind = match kind {
            DateKind::Due => Kind::Due,
            DateKind::Exam => Kind::Exam,
            DateKind::Other => continue,
        };
        if p < 0.75 || found.iter().any(|f| f.date == date) {
            continue;
        }
        let context: String = sentence.chars().take(CONTEXT * 2).collect();
        found.push(Found { date, time, context, kind, word: String::new(), title: None, estimated: false, ai: true });
    }
    found
}

/// A date given relative to when the file came: a weekday («Abgabe: Montag»), «morgen»,
/// «übermorgen», «in zwei Wochen», «nächste Woche Freitag», «KW 45» (its Friday). Returns
/// the date and where the words end. «Jeweils Montag» is every week, not one date.
fn relative_after(folded: &[char], from: usize, stops: &[bool], reference: NaiveDate) -> Option<(NaiveDate, usize)> {
    const NUMBERS: &[(&str, i64)] = &[
        ("ein", 1),
        ("einem", 1),
        ("einer", 1),
        ("eine", 1),
        ("one", 1),
        ("a", 1),
        ("zwei", 2),
        ("two", 2),
        ("drei", 3),
        ("three", 3),
        ("vier", 4),
        ("four", 4),
        ("funf", 5),
        ("five", 5),
        ("sechs", 6),
        ("six", 6),
        ("sieben", 7),
        ("seven", 7),
        ("acht", 8),
        ("eight", 8),
        ("zehn", 10),
        ("ten", 10),
        ("vierzehn", 14),
    ];
    const NEXT: &[&str] = &["nachste", "nachsten", "nachster", "kommende", "kommenden", "next", "folgenden"];
    let limit = (from + WEEKDAY_AHEAD).min(folded.len());
    // The words up to the end of the sentence: (word, start, end).
    let mut words: Vec<(String, usize, usize)> = Vec::new();
    let mut i = from;
    while i < limit && !stops[i] {
        if folded[i].is_alphanumeric() && (i == 0 || !folded[i - 1].is_alphanumeric()) {
            let end = (i..folded.len()).find(|&j| !folded[j].is_alphanumeric()).unwrap_or(folded.len());
            words.push((folded[i..end].iter().collect(), i, end));
            i = end;
            continue;
        }
        i += 1;
    }
    let days = |n: i64| reference + chrono::Duration::days(n);
    let weekday_of = |w: &str| WEEKDAYS.iter().find(|(name, _)| *name == w).map(|(_, d)| *d as i64);
    let next_weekday = |day: i64| {
        let ahead = (day - reference.weekday().num_days_from_monday() as i64).rem_euclid(7);
        days(if ahead == 0 { 7 } else { ahead })
    };
    let number = |w: &str| {
        w.parse::<i64>()
            .ok()
            .filter(|n| (1..=60).contains(n))
            .or_else(|| NUMBERS.iter().find(|(name, _)| *name == w).map(|(_, n)| *n))
    };
    for (k, (w, start, end)) in words.iter().enumerate() {
        let next = |n: usize| words.get(k + n).map(|(w, _, e)| (w.as_str(), *e));
        let before = k.checked_sub(1).map(|b| words[b].0.as_str());
        match w.as_str() {
            "jeweils" | "jeden" | "immer" | "every" | "each" => return None,
            "morgen" | "tomorrow" if before != Some("am") => return Some((days(1), *end)),
            "ubermorgen" => return Some((days(2), *end)),
            "in" => {
                let (Some((n, _)), Some((unit, unit_end))) = (next(1), next(2)) else { continue };
                let Some(n) = number(n) else { continue };
                let per = match unit {
                    "tag" | "tage" | "tagen" | "day" | "days" => 1,
                    "woche" | "wochen" | "week" | "weeks" => 7,
                    _ => continue,
                };
                return Some((days(n * per), unit_end));
            }
            "kw" | "kalenderwoche" | "week" => {
                let Some((n, n_end)) = next(1) else { continue };
                let Some(week) = n.parse::<u32>().ok().filter(|w| (1..=53).contains(w)) else { continue };
                // That week's Friday, the one closest after the file came.
                let friday = |year| NaiveDate::from_isoywd_opt(year, week, chrono::Weekday::Fri);
                let date = friday(reference.year())
                    .filter(|d| *d >= reference - chrono::Duration::days(60))
                    .or_else(|| friday(reference.year() + 1))?;
                return Some((date, n_end));
            }
            w if NEXT.contains(&w) => {
                if let Some((_, unit_end)) = next(1).filter(|(u, _)| ["woche", "week"].contains(u)) {
                    let monday = days(7 - reference.weekday().num_days_from_monday() as i64);
                    return Some(match next(2).and_then(|(d, e)| weekday_of(d).map(|d| (d, e))) {
                        Some((day, day_end)) => (monday + chrono::Duration::days(day), day_end),
                        None => (monday + chrono::Duration::days(4), unit_end),
                    });
                }
            }
            w => {
                if let Some(day) = weekday_of(w) {
                    let _ = start;
                    return Some((next_weekday(day), *end));
                }
            }
        }
    }
    None
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

/// A date found for a word: where it is, and what is due if the table says it.
struct Hit {
    date: NaiveDate,
    start: usize,
    end: usize,
    title: Option<String>,
    estimated: bool,
}

/// The first date after the word within the sentence (and a few lines), skipping dates that
/// say when something was made ("Ausgabe: 20.10.").
fn date_ahead(folded: &[char], raw: &[char], after: usize, stops: &[bool], reference: NaiveDate) -> Option<Hit> {
    let limit = (after + LOOK_AHEAD).min(folded.len());
    let mut breaks = 0;
    let mut i = after;
    while i < limit && !stops[i] {
        if raw[i] == '\n' {
            breaks += 1;
            if breaks > MAX_LINE_BREAKS {
                break;
            }
        }
        if let Some((date, end)) = full_date(folded, i, reference) {
            if NOT_DUE.contains(&word_before(folded, i).as_str()) {
                i = end;
                continue;
            }
            return Some(Hit { date, start: i, end, title: None, estimated: false });
        }
        i += 1;
    }
    None
}

/// The nearest date before the word within the sentence.
fn date_behind(folded: &[char], start: usize, stops: &[bool], reference: NaiveDate) -> Option<Hit> {
    let mut i = start;
    while i > 0 && start - i < LOOK_BEHIND && !stops[i - 1] {
        i -= 1;
        if let Some((date, end)) = full_date(folded, i, reference).filter(|&(_, end)| end <= start) {
            if NOT_DUE.contains(&word_before(folded, i).as_str()) {
                continue;
            }
            return Some(Hit { date, start: i, end, title: None, estimated: false });
        }
    }
    None
}

/// Deadlines and exam dates in `text`: a deadline word with a date close to it in the same
/// sentence or table column, or "bis <date>" in a sentence about handing something in.
/// `reference` is about when the file was published, for dates without a year.
pub fn find(text: &str, reference: NaiveDate) -> Vec<Found> {
    let raw: Vec<char> = text.chars().collect();
    let folded: Vec<char> = fold(text).chars().collect();
    let haystack: String = folded.iter().collect();
    let stops = stops(&raw, &folded);
    let document = document_work(&raw);
    let mut found: Vec<Found> = Vec::new();
    let add = |found: &mut Vec<Found>, f: Found| {
        if !found.iter().any(|g| g.date == f.date) && (f.date - reference).num_days().abs() <= 366 {
            found.push(f);
        }
    };
    for &(trigger, kind, look, fit) in TRIGGERS {
        for (byte, _) in haystack.match_indices(trigger) {
            let start = haystack[..byte].chars().count();
            let after = start + trigger.chars().count();
            let Some((word_start, word_end)) = fits(&folded, start, after, fit) else { continue };
            let word: String = raw[word_start..word_end].iter().collect();
            // A compound names what is due ("Projektabgabe"), "Abgabetermin" doesn't.
            let named = (word_start < start && trigger != "frist").then(|| word.clone());
            let mut hits: Vec<Hit> = Vec::new();
            if look != Look::Behind {
                hits = table_rows(&raw, &folded, word_start, reference)
                    .into_iter()
                    .map(|(date, start, end, title)| Hit { date, start, end, title, estimated: false })
                    .collect();
            }
            if hits.is_empty() {
                let hit = match look {
                    Look::Ahead => date_ahead(&folded, &raw, word_end, &stops, reference),
                    Look::Period => date_ahead(&folded, &raw, word_end, &stops, reference).map(|h| {
                        let (date, start, end) = period_end(&folded, h.end, h.date, reference);
                        Hit { date, start, end, ..h }
                    }),
                    Look::Behind => date_behind(&folded, word_start, &stops, reference),
                };
                // Only a weekday after a hand-in word: the next such day.
                let hit = hit.or_else(|| {
                    if kind != Kind::Due || look != Look::Ahead {
                        return None;
                    }
                    let (date, end) = relative_after(&folded, word_end, &stops, reference)?;
                    Some(Hit { date, start: end, end, title: None, estimated: true })
                });
                hits.extend(hit);
            }
            for hit in hits {
                let (from, to) = (word_start.min(hit.start), word_end.max(hit.end));
                let time = time_after(&folded, hit.end).or_else(|| time_before(&folded, hit.start, &stops));
                let title = match kind {
                    Kind::Exam => {
                        // "Final exam", "Midterm exam"
                        let (before, at) = word_before_at(&folded, word_start);
                        let word = match ["final", "midterm"].contains(&before.as_str()) {
                            true => raw[at..word_end].iter().collect(),
                            false => word.clone(),
                        };
                        Some(exam_name(&raw, &word, word_end))
                    }
                    Kind::Due => hit.title.or_else(|| due_title(work_name(&raw, from, to), named.clone(), &document)),
                };
                let f = Found {
                    ai: false,
                    date: hit.date,
                    time,
                    context: passage(&raw, from, to),
                    kind,
                    word: word.clone(),
                    title,
                    estimated: hit.estimated,
                };
                add(&mut found, f);
            }
        }
    }
    // "Bitte bis 9.11. in Moodle hochladen", "Laden Sie Ihre Lösung bis Do 13.11. hoch".
    let mut i = 0;
    while i < folded.len() {
        let Some((date, end)) = full_date(&folded, i, reference) else {
            i += 1;
            continue;
        };
        let lead: String = folded[i.saturating_sub(30)..i].iter().collect();
        let lead_words: Vec<&str> = lead.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).collect();
        let by = lead_words.iter().rev().take(4).any(|w| *w == "bis" || *w == "by");
        let from = (0..i).rev().take(200).find(|&j| stops[j]).map_or(i.saturating_sub(200), |j| j + 1);
        let to = (end..folded.len()).take(200).find(|&j| stops[j]).unwrap_or((end + 200).min(folded.len()));
        let sentence: String = folded[from..to].iter().collect();
        let hand_in = sentence
            .split(|c: char| !c.is_alphanumeric())
            .any(|w| w == "hoch" || w.contains("abgabe") || HAND_IN_WORDS.iter().any(|h| w.starts_with(h)));
        if by && hand_in && !NOT_DUE.contains(&word_before(&folded, i).as_str()) {
            let time = time_after(&folded, end).or_else(|| time_before(&folded, i, &stops));
            let title = due_title(work_name(&raw, from, to), None, &document);
            let f = Found {
                ai: false,
                date,
                time,
                context: passage(&raw, i.saturating_sub(30).max(from), end),
                kind: Kind::Due,
                word: String::new(),
                title,
                estimated: false,
            };
            add(&mut found, f);
        }
        i = end;
    }
    found.sort_by_key(|f| (f.date, f.time));
    found
}

/// An exam's name: the word without "-termin"/"-datum", and a number after it
/// ("Leistungskontrolle 1", "Klausur", "Zwischenprüfung").
fn exam_name(raw: &[char], word: &str, word_end: usize) -> String {
    let mut name = word.to_string();
    for ending in ["stermine", "stermin", "termine", "termin", "sdatum", "datum", "stag", "tag"] {
        let lower = name.to_lowercase();
        if lower.ends_with(ending) && lower.len() > ending.len() + 3 {
            name.truncate(name.len() - ending.len());
            break;
        }
    }
    let next = skip_spaces(raw, word_end);
    if let Some((n, e)) = number(raw, next, 2)
        && !raw.get(e).is_some_and(|c| *c == '.' || *c == '/')
    {
        name = format!("{name} {n}");
    }
    capitalised(&name)
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Deadlines {
    pub items: Vec<Deadline>,
    /// Files already looked at (path and size), so they aren't read again.
    scanned: Vec<(PathBuf, u64)>,
    /// Which [`FINDER_VERSION`] looked at them.
    finder: u32,
    /// The AI model looked at them too (once it is there, everything is read again once).
    #[serde(default)]
    with_ai: bool,
    /// When the list was last looked at (Unix seconds): entries added later are new.
    pub seen: i64,
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

    /// The AI model is there now: read every file again once, with it. True if that is new.
    pub fn read_again_with_ai(&mut self) -> bool {
        if self.with_ai {
            return false;
        }
        self.with_ai = true;
        self.scanned.clear();
        true
    }

    pub fn save(&self, file: &Path) -> std::io::Result<()> {
        crate::cloud::save_json(self, file)
    }

    fn next_id(&self) -> u64 {
        self.items.iter().map(|d| d.id).max().unwrap_or(0) + 1
    }

    /// How many documents were looked through.
    pub fn scanned_count(&self) -> usize {
        self.scanned.len()
    }

    /// Done entries due from `days_back` days ago on, latest first (to open them again).
    pub fn recently_done(&self, today: NaiveDate, days_back: i64) -> Vec<&Deadline> {
        let mut list: Vec<&Deadline> = self
            .items
            .iter()
            .filter(|d| d.done && !d.deleted && d.day().is_some_and(|day| (today - day).num_days() <= days_back))
            .collect();
        list.sort_by_key(|d| std::cmp::Reverse(d.due()));
        list
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
            // Named after what the text says is due ("Serie 5"), else after the file.
            let title = f.title.clone().unwrap_or_else(|| match f.kind {
                Kind::Due => stem.clone(),
                Kind::Exam => capitalised(&f.word),
            });
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
                estimated: f.estimated,
                added: chrono::Utc::now().timestamp(),
                reminded: Vec::new(),
                repeat: None,
                ai: f.ai,
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
            estimated: false,
            added: chrono::Utc::now().timestamp(),
            reminded: Vec::new(),
            repeat: None,
            ai: false,
        });
        id
    }

    /// Correct an entry (a date that was read wrong, a better title).
    /// Mark done. One that comes again moves on to its next date instead (the one done stays in
    /// the list as done); after its last date it is done for good.
    pub fn complete(&mut self, id: u64) {
        let Some(i) = self.items.iter().position(|d| d.id == id) else { return };
        let Some(repeat) = self.items[i].repeat.clone() else {
            self.items[i].done = true;
            return;
        };
        let next = self.items[i].day().and_then(|d| repeat.every.after(d));
        let next = next.filter(|n| repeat.until_day().is_none_or(|u| *n <= u));
        let Some(next) = next else {
            self.items[i].done = true;
            return;
        };
        let done_id = self.next_id();
        let mut done = self.items[i].clone();
        done.id = done_id;
        done.repeat = None;
        done.done = true;
        let d = &mut self.items[i];
        d.date = next.format("%Y-%m-%d").to_string();
        d.reminded.clear();
        d.estimated = false;
        self.items.push(done);
    }

    /// Let a deadline come again (or not).
    pub fn set_repeat(&mut self, id: u64, repeat: Option<Repeat>) {
        if let Some(d) = self.items.iter_mut().find(|d| d.id == id) {
            d.repeat = repeat;
        }
    }

    pub fn edit(&mut self, id: u64, title: &str, module: &str, kind: Kind, date: NaiveDate, time: Option<NaiveTime>) {
        let Some(d) = self.items.iter_mut().find(|d| d.id == id) else { return };
        let date = date.format("%Y-%m-%d").to_string();
        if d.file.is_some() && d.found_date.is_none() && d.date != date {
            d.found_date = Some(d.date.clone());
        }
        if d.date != date {
            d.estimated = false;
            d.reminded.clear();
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
    ]
    .into_iter()
    .chain(deadline.repeat.as_ref().map(|r| {
        let rule = match r.every {
            Every::Week => "FREQ=WEEKLY",
            Every::TwoWeeks => "FREQ=WEEKLY;INTERVAL=2",
            Every::Month => "FREQ=MONTHLY",
        };
        let until = r.until_day().map(|u| format!(";UNTIL={}T235959", u.format("%Y%m%d"))).unwrap_or_default();
        format!("RRULE:{rule}{until}")
    }))
    .chain([
        format!("SUMMARY:{}", escape(summary)),
        format!("DESCRIPTION:{}", escape(&description)),
        "BEGIN:VALARM".to_string(),
        "TRIGGER:-P1D".to_string(),
        "ACTION:DISPLAY".to_string(),
        format!("DESCRIPTION:{}", escape(reminder)),
        "END:VALARM".to_string(),
        "END:VEVENT".to_string(),
        "END:VCALENDAR".to_string(),
    ])
    .collect::<Vec<_>>()
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
            // Only the week: its Friday (estimated).
            ("Abgabe in KW 45 über Moodle. Vorlesung vom 20.10.2026.", &[(day(2026, 11, 6), None, Kind::Due)]),
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

    /// Deadlines that were missed before: dates without the last dot or with slashes, compound
    /// words, other phrasings, abbreviations, weekdays only, times before the date.
    #[test]
    fn finds_deadlines_written_in_other_ways() {
        type Expected<'a> = (NaiveDate, Option<&'a str>, Kind, &'a str);
        let cases: &[(&str, &[Expected])] = &[
            (
                "Übungsblatt 4\nAbgabe: 3.11 bis 10 Uhr",
                &[(day(2026, 11, 3), Some("10:00"), Kind::Due, "Übungsblatt 4")],
            ),
            (
                "Assignment 3\nDue: 24/10/2026 at 23:59",
                &[(day(2026, 10, 24), Some("23:59"), Kind::Due, "Assignment 3")],
            ),
            ("Homework 6\nDue date: 11/28/2026", &[(day(2026, 11, 28), None, Kind::Due, "Homework 6")]),
            (
                "Projektabgabe: 15.12.2026, 18:00 Uhr. Zwischenabgabe am 17.11.2026.",
                &[
                    (day(2026, 11, 17), None, Kind::Due, "Zwischenabgabe"),
                    (day(2026, 12, 15), Some("18:00"), Kind::Due, "Projektabgabe"),
                ],
            ),
            ("Klausurtermin: 12.02.2027, 9:00 Uhr", &[(day(2027, 2, 12), Some("09:00"), Kind::Exam, "Klausur")]),
            (
                "Die Semesterendprüfung findet am 22.01.2027 statt.",
                &[(day(2027, 1, 22), None, Kind::Exam, "Semesterendprüfung")],
            ),
            (
                "Leistungskontrolle 1: 18.11.2026, 10:15–11:00",
                &[(day(2026, 11, 18), Some("10:15"), Kind::Exam, "Leistungskontrolle 1")],
            ),
            (
                "Endklausur am 4. Februar 2027 von 14 bis 16 Uhr.",
                &[(day(2027, 2, 4), Some("14:00"), Kind::Exam, "Endklausur")],
            ),
            ("Was Datum\nZwischentest 26.11.2026", &[(day(2026, 11, 26), None, Kind::Exam, "Zwischentest")]),
            ("Midterm exam: Thursday, November 19, 2026", &[(day(2026, 11, 19), None, Kind::Exam, "Midterm exam")]),
            (
                "Blatt 6\nDie Lösungen müssen bis Freitag, den 7. November 2026 um 14 Uhr über Ilias abgegeben werden.",
                &[(day(2026, 11, 7), Some("14:00"), Kind::Due, "Blatt 6")],
            ),
            ("Abgabe bei Prof. Dr. Huber am 5.11.2026 in der Vorlesung.", &[(day(2026, 11, 5), None, Kind::Due, "")]),
            (
                "Serie 3\nAbgabe: Montag, 10:15 Uhr in der Übungsstunde.",
                &[(day(2026, 10, 26), Some("10:15"), Kind::Due, "Serie 3")],
            ),
            (
                "Blatt 2\nBitte bis 12 Uhr am 28.10. in den Briefkasten einwerfen.",
                &[(day(2026, 10, 28), Some("12:00"), Kind::Due, "Blatt 2")],
            ),
            (
                "Abgabe bis spätestens 9 Uhr am Montag, den 2.11.2026.",
                &[(day(2026, 11, 2), Some("09:00"), Kind::Due, "")],
            ),
            (
                "Übung 5\nLaden Sie Ihre Lösung bis Donnerstag 13.11. auf Moodle hoch.",
                &[(day(2026, 11, 13), None, Kind::Due, "Übung 5")],
            ),
            ("Lab 2\nUpload your report by Nov 4, 5pm.", &[(day(2026, 11, 4), Some("17:00"), Kind::Due, "Lab 2")]),
            (
                "Seminar\nEinreichungsfrist für die Seminararbeit: 31. Januar 2027",
                &[(day(2027, 1, 31), None, Kind::Due, "Seminararbeit")],
            ),
            (
                "Abgabe Teil A: 5.11.2026. Abgabe Teil B: 26.11.2026.",
                &[(day(2026, 11, 5), None, Kind::Due, "Teil A"), (day(2026, 11, 26), None, Kind::Due, "Teil B")],
            ),
            (
                "Abgabe Übung 4\nAbgabebeginn: Dienstag, 21. Oktober 2026, 08:00\nFälligkeitsdatum: Dienstag, 4. November 2026, 08:00",
                &[(day(2026, 11, 4), Some("08:00"), Kind::Due, "Übung 4")],
            ),
        ];
        for (text, expected) in cases {
            let found: Vec<_> = find(text, day(2026, 10, 20))
                .into_iter()
                .map(|f| (f.date, f.time.map(|t| t.format("%H:%M").to_string()), f.kind, f.title.unwrap_or_default()))
                .collect();
            let expected: Vec<_> =
                expected.iter().map(|(d, t, k, n)| (*d, t.map(String::from), *k, n.to_string())).collect();
            assert_eq!(found, expected, "{text}");
        }
        // Only a weekday: the next one after the file came, marked as estimated.
        assert!(find("Abgabe: Montag, 10:15 Uhr", day(2026, 10, 20))[0].estimated);
    }

    #[test]
    fn relative_dates_count_from_when_the_file_came() {
        let reference = day(2026, 10, 20); // a Tuesday
        let first = |text: &str| find(text, reference).first().map(|f| (f.date, f.estimated));
        assert_eq!(first("Abgabe: morgen bis 12 Uhr"), Some((day(2026, 10, 21), true)));
        assert_eq!(first("Abgabe übermorgen"), Some((day(2026, 10, 22), true)));
        assert_eq!(first("Abgabe in zwei Wochen"), Some((day(2026, 11, 3), true)));
        assert_eq!(first("Abgabe in 10 Tagen"), Some((day(2026, 10, 30), true)));
        assert_eq!(first("Due in two weeks"), Some((day(2026, 11, 3), true)));
        assert_eq!(first("Abgabe nächste Woche Freitag"), Some((day(2026, 10, 30), true)));
        assert_eq!(first("Abgabe nächste Woche"), Some((day(2026, 10, 30), true)), "its Friday");
        assert_eq!(first("Abgabe bis Ende KW 2"), Some((day(2027, 1, 15), true)), "next year's week 2");
        assert_eq!(first("Abgabe: Freitag"), Some((day(2026, 10, 23), true)));
        assert_eq!(first("Abgabe am Morgen der Vorlesung"), None, "«am Morgen» is no date");
        assert_eq!(first("Abgabe jeweils in der Übung"), None);
        // A written date always wins.
        assert_eq!(first("Abgabe morgen, 21.10.2026"), Some((day(2026, 10, 21), false)));
    }

    /// A semester plan: one deadline per row, from the «Abgabe» column, named after the row.
    #[test]
    fn semester_plans_give_one_deadline_per_row() {
        let text = "Organisatorisches\n\nBlatt Ausgabe Abgabe\n\nBlatt 1 20.10. 27.10.\n\nBlatt 2 27.10. 3.11.\n\n\
                    Blatt 3 Mo 3.11. Mo 10.11.\n\nWeihnachtspause\n\nBlatt 4 10.11. 17.11.\n";
        let found: Vec<_> =
            find(text, day(2026, 10, 15)).into_iter().map(|f| (f.date, f.title.unwrap_or_default())).collect();
        let expected = [(27, 10, "Blatt 1"), (3, 11, "Blatt 2"), (10, 11, "Blatt 3"), (17, 11, "Blatt 4")];
        let expected: Vec<_> = expected.iter().map(|&(d, m, t)| (day(2026, m, d), t.to_string())).collect();
        assert_eq!(found, expected);
    }

    /// Numbers, words and repeating weekdays that are no deadline.
    #[test]
    fn no_deadlines_in_numbers_and_look_alikes() {
        let texts = [
            "Abgabe der Aufgabe 3.2. und Kapitel 4.1. in der Übung",
            "Abgabe in Gruppen zu 3.5 Personen über Moodle",
            "Gemäss Prüfungsordnung vom 01.09.2026 gilt ...",
            "Überprüfung der Messwerte am 12.11.2026 im Labor.",
            "Messung der Wärmeabgabe am 3.11. im Labor.",
            "Präsentationstechniken – 20.10.2026",
            "Stand: 20.10.2026\n\nDie Abgabe der Übungen ist freiwillig.",
            "Assignments are due every Friday at 5pm.",
            "Abgabetermin für alle Serien: jeweils Montag 10:00 Uhr.",
            "Klausuranmeldung bis 15.11.2026 im Portal.",
            "Rückgabe der korrigierten Blätter am 10.11. Die Übung am 3.11. entfällt.",
            "Abgabe um 12.10 Uhr",
        ];
        for text in texts {
            assert_eq!(find(text, day(2026, 10, 20)), [], "{text}");
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

    #[test]
    fn recurring_deadlines_move_on_when_done() {
        let mut list = Deadlines::default();
        let id = list.add_manual("Wochenquiz", "Stats", Kind::Due, day(2026, 10, 23), None);
        list.set_repeat(id, Some(Repeat { every: Every::Week, until: Some("2026-11-06".into()) }));
        let ics_text = ics(&list.items[0], "Wochenquiz", "Morgen");
        assert!(ics_text.contains("RRULE:FREQ=WEEKLY;UNTIL=20261106T235959"), "{ics_text}");
        list.complete(id);
        let series = list.items.iter().find(|d| d.id == id).unwrap();
        assert_eq!((series.date.as_str(), series.done), ("2026-10-30", false), "next week");
        assert_eq!(list.items.iter().filter(|d| d.done && d.date == "2026-10-23").count(), 1, "the done one stays");
        list.complete(id);
        list.complete(id);
        let series = list.items.iter().find(|d| d.id == id).unwrap();
        assert!(series.done && series.date == "2026-11-06", "after the last date it is done for good");
        // Monthly, without an end.
        let id = list.add_manual("Bericht", "", Kind::Due, day(2026, 1, 31), None);
        list.set_repeat(id, Some(Repeat { every: Every::Month, until: None }));
        list.complete(id);
        assert_eq!(list.items.iter().find(|d| d.id == id).unwrap().date, "2026-02-28");
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
                println!(
                    "  -> {} {:?} [{:?}{}]  «{}»",
                    found.date,
                    found.time,
                    found.title,
                    if found.estimated { " geschätzt" } else { "" },
                    found.context
                );
                let time = found.time.map(|t| t.format("%H:%M").to_string()).unwrap_or_default();
                let kind = if found.kind == super::Kind::Exam { "exam" } else { "due" };
                println!("@@\t{}\t{}\t{time}\t{kind}", f.file_name().unwrap().to_string_lossy(), found.date);
            }
        }
    }
}
