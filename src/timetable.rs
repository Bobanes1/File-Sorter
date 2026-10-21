//! The timetable: lessons each week (imported from a calendar file or link, `.ics`, or
//! entered by hand), kept in `timetable.json`.
//!
//! When sorting it is only a light hint: during a lesson one may well work on something
//! else, so it never decides against the name or content of a file. It only settles a tie
//! between modules that fit equally well, in favour of the one whose lesson was running.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::SystemTime;

use chrono::{DateTime, Datelike, Duration, Local, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Utc};
use serde::{Deserialize, Serialize};

use crate::config::Config;

pub const TIMETABLE_FILE: &str = "timetable.json";
/// A file that came this long before a lesson or after it still counts as during it.
const AROUND: i64 = 15;

/// A lesson: every week (or every few weeks) on one weekday, or once.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Lesson {
    pub id: u64,
    /// What the calendar calls it («Statistik I – Vorlesung»).
    pub title: String,
    /// The module's label, or empty.
    pub module: String,
    /// «Vorlesung», «Übung», …
    pub kind: String,
    /// 0 = Monday.
    pub weekday: u32,
    /// `HH:MM`.
    pub start: String,
    pub end: String,
    pub room: String,
    /// First and last day (`YYYY-MM-DD`).
    pub from: String,
    pub until: String,
    /// Every this many weeks (1 = weekly).
    pub every: u32,
    /// Days it doesn't take place (`YYYY-MM-DD`).
    pub skipped: Vec<String>,
    /// Where it came from (a calendar file or link), empty if entered by hand.
    pub source: String,
}

fn day(text: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(text, "%Y-%m-%d").ok()
}

fn time(text: &str) -> Option<NaiveTime> {
    NaiveTime::parse_from_str(text, "%H:%M").ok()
}

pub fn day_text(date: NaiveDate) -> String {
    date.format("%Y-%m-%d").to_string()
}

impl Lesson {
    pub fn start_time(&self) -> Option<NaiveTime> {
        time(&self.start)
    }

    pub fn end_time(&self) -> Option<NaiveTime> {
        time(&self.end)
    }

    /// Whether it takes place on `date`.
    pub fn on(&self, date: NaiveDate) -> bool {
        let (Some(from), Some(until)) = (day(&self.from), day(&self.until)) else { return false };
        // The first date on its weekday from `from` on.
        let first =
            from + Duration::days((self.weekday as i64 - from.weekday().num_days_from_monday() as i64).rem_euclid(7));
        date >= first
            && date <= until
            && date.weekday().num_days_from_monday() == self.weekday
            && (date - first).num_days() % (7 * self.every.max(1) as i64) == 0
            && !self.skipped.contains(&day_text(date))
    }

    /// Whether it is running at `at` (with a little time before and after).
    pub fn running(&self, at: NaiveDateTime) -> bool {
        let (Some(start), Some(end)) = (self.start_time(), self.end_time()) else { return false };
        let date = at.date();
        self.on(date)
            && at >= date.and_time(start) - Duration::minutes(AROUND)
            && at <= date.and_time(end) + Duration::minutes(AROUND)
    }
}

/// A calendar link the timetable is read from again every day.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Subscription {
    pub url: String,
    /// When it was last read (Unix seconds).
    pub read: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Timetable {
    pub lessons: Vec<Lesson>,
    pub subscriptions: Vec<Subscription>,
}

impl Timetable {
    pub fn load(file: &Path) -> Timetable {
        fs::read_to_string(file).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
    }

    pub fn save(&self, file: &Path) -> std::io::Result<()> {
        crate::cloud::save_json(self, file)
    }

    pub fn next_id(&self) -> u64 {
        self.lessons.iter().map(|l| l.id).max().unwrap_or(0) + 1
    }

    /// Put in what was read from `source`, instead of what came from there before.
    pub fn replace_source(&mut self, source: &str, mut lessons: Vec<Lesson>) {
        self.lessons.retain(|l| l.source != source);
        for (id, lesson) in (self.next_id()..).zip(lessons.iter_mut()) {
            lesson.id = id;
            lesson.source = source.to_string();
        }
        self.lessons.extend(lessons);
    }
}

// ---------- reading .ics ----------

/// One event of a calendar file.
#[derive(Debug, Clone, Default)]
struct Event {
    uid: String,
    summary: String,
    location: String,
    start: Option<NaiveDateTime>,
    end: Option<NaiveDateTime>,
    all_day: bool,
    rule: Option<String>,
    exdates: Vec<NaiveDate>,
    /// An event that moves one date of a series (`RECURRENCE-ID`).
    replaces: Option<NaiveDate>,
}

/// Lines of a calendar file with folded lines joined.
fn unfold(text: &str) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for line in text.lines() {
        match line.strip_prefix([' ', '\t']) {
            Some(rest) if !lines.is_empty() => lines.last_mut().expect("not empty").push_str(rest),
            _ => lines.push(line.trim_end_matches('\r').to_string()),
        }
    }
    lines
}

fn unescape(text: &str) -> String {
    text.replace("\\n", " ").replace("\\N", " ").replace("\\,", ",").replace("\\;", ";").replace("\\\\", "\\")
}

/// A date or time value: `20261020T101500Z` (UTC, made local), `20261020T101500` (local),
/// `20261020` (a whole day).
fn date_time(value: &str) -> Option<(NaiveDateTime, bool)> {
    let value = value.trim();
    if value.len() == 8 {
        let date = NaiveDate::parse_from_str(value, "%Y%m%d").ok()?;
        return Some((date.and_hms_opt(0, 0, 0)?, true));
    }
    if let Some(utc) = value.strip_suffix('Z') {
        let naive = NaiveDateTime::parse_from_str(utc, "%Y%m%dT%H%M%S").ok()?;
        let local: DateTime<Local> = Utc.from_utc_datetime(&naive).with_timezone(&Local);
        return Some((local.naive_local(), false));
    }
    Some((NaiveDateTime::parse_from_str(value, "%Y%m%dT%H%M%S").ok()?, false))
}

fn events(text: &str) -> Vec<Event> {
    let mut out = Vec::new();
    let mut current: Option<Event> = None;
    for line in unfold(text) {
        if line == "BEGIN:VEVENT" {
            current = Some(Event::default());
            continue;
        }
        if line == "END:VEVENT" {
            out.extend(current.take());
            continue;
        }
        let Some(event) = current.as_mut() else { continue };
        let Some((name, value)) = line.split_once(':') else { continue };
        let key = name.split(';').next().unwrap_or("").to_ascii_uppercase();
        match key.as_str() {
            "UID" => event.uid = value.to_string(),
            "SUMMARY" => event.summary = unescape(value).trim().to_string(),
            "LOCATION" => event.location = unescape(value).trim().to_string(),
            "DTSTART" => {
                if let Some((at, all_day)) = date_time(value) {
                    event.start = Some(at);
                    event.all_day = all_day;
                }
            }
            "DTEND" => event.end = date_time(value).map(|(at, _)| at),
            "RRULE" => event.rule = Some(value.to_string()),
            "EXDATE" => event.exdates.extend(value.split(',').filter_map(|v| date_time(v).map(|(at, _)| at.date()))),
            "RECURRENCE-ID" => event.replaces = date_time(value).map(|(at, _)| at.date()),
            _ => {}
        }
    }
    out
}

fn weekday_of(code: &str) -> Option<u32> {
    let code: String = code.chars().filter(|c| c.is_ascii_alphabetic()).collect();
    ["MO", "TU", "WE", "TH", "FR", "SA", "SU"].iter().position(|d| *d == code).map(|p| p as u32)
}

/// The lessons in a calendar file: weekly series as one lesson per weekday, single events
/// that repeat weekly at the same time and place joined into one, the rest as single dates.
/// Whole-day events (holidays, deadlines) are left out.
pub fn read_ics(text: &str) -> Vec<Lesson> {
    let all = events(text);
    let mut lessons: Vec<Lesson> = Vec::new();
    let mut singles: Vec<Event> = Vec::new();
    // Moved dates of a series: the series skips them, the moved one is a single date.
    let moved: Vec<(String, NaiveDate)> = all.iter().filter_map(|e| e.replaces.map(|d| (e.uid.clone(), d))).collect();
    for event in all {
        let (Some(start), Some(end)) = (event.start, event.end.or(event.start)) else { continue };
        if event.all_day || event.summary.is_empty() {
            continue;
        }
        let base = Lesson {
            title: event.summary.clone(),
            weekday: start.date().weekday().num_days_from_monday(),
            start: start.format("%H:%M").to_string(),
            end: end.format("%H:%M").to_string(),
            room: event.location.clone(),
            from: day_text(start.date()),
            until: day_text(start.date()),
            every: 1,
            ..Default::default()
        };
        let Some(rule) = event.rule.as_ref().filter(|_| event.replaces.is_none()) else {
            singles.push(event);
            continue;
        };
        let parts: Vec<(&str, &str)> = rule.split(';').filter_map(|p| p.split_once('=')).collect();
        let get = |k: &str| parts.iter().find(|(key, _)| key.eq_ignore_ascii_case(k)).map(|(_, v)| *v);
        if !get("FREQ").is_some_and(|f| f.eq_ignore_ascii_case("WEEKLY")) {
            singles.push(event);
            continue;
        }
        let every: u32 = get("INTERVAL").and_then(|i| i.parse().ok()).unwrap_or(1).max(1);
        let days: Vec<u32> = match get("BYDAY") {
            Some(list) => list.split(',').filter_map(weekday_of).collect(),
            None => vec![base.weekday],
        };
        let until = match (get("UNTIL").and_then(date_time), get("COUNT").and_then(|c| c.parse::<i64>().ok())) {
            (Some((until, _)), _) => until.date(),
            (None, Some(count)) => {
                let weeks = (count + days.len() as i64 - 1) / days.len().max(1) as i64;
                start.date() + Duration::weeks((weeks - 1).max(0) * every as i64) + Duration::days(6)
            }
            // Without an end: until the end of the semester, about half a year.
            (None, None) => start.date() + Duration::days(183),
        };
        let mut skipped: Vec<String> = event.exdates.iter().map(|d| day_text(*d)).collect();
        skipped.extend(moved.iter().filter(|(uid, _)| *uid == event.uid).map(|(_, d)| day_text(*d)));
        for weekday in days {
            // The first date on that weekday on or after the start.
            let offset = (weekday as i64 - base.weekday as i64).rem_euclid(7);
            let first = start.date() + Duration::days(offset);
            lessons.push(Lesson {
                weekday,
                from: day_text(first),
                until: day_text(until),
                every,
                skipped: skipped.clone(),
                ..base.clone()
            });
        }
    }
    lessons.extend(join_weekly(singles));
    lessons
}

/// Single events at the same weekday, time, title and place a whole number of weeks apart
/// become one weekly lesson (with the weeks in between skipped).
fn join_weekly(mut singles: Vec<Event>) -> Vec<Lesson> {
    singles.sort_by_key(|e| e.start);
    let mut groups: Vec<(Lesson, Vec<NaiveDate>)> = Vec::new();
    for event in singles {
        let (Some(start), Some(end)) = (event.start, event.end.or(event.start)) else { continue };
        let lesson = Lesson {
            title: event.summary,
            weekday: start.date().weekday().num_days_from_monday(),
            start: start.format("%H:%M").to_string(),
            end: end.format("%H:%M").to_string(),
            room: event.location,
            every: 1,
            ..Default::default()
        };
        let same = |l: &Lesson| {
            l.title == lesson.title
                && l.weekday == lesson.weekday
                && l.start == lesson.start
                && l.end == lesson.end
                && l.room == lesson.room
        };
        match groups.iter_mut().find(|(l, _)| same(l)) {
            Some((_, dates)) => dates.push(start.date()),
            None => groups.push((lesson, vec![start.date()])),
        }
    }
    groups
        .into_iter()
        .map(|(mut lesson, dates)| {
            let (first, last) = (dates[0], *dates.last().expect("one at least"));
            lesson.from = day_text(first);
            lesson.until = day_text(last);
            let mut week = first;
            while week <= last {
                if !dates.contains(&week) {
                    lesson.skipped.push(day_text(week));
                }
                week += Duration::weeks(1);
            }
            lesson
        })
        .collect()
}

/// Which module a calendar entry is about, by the modules' keywords (only a clear one).
pub fn guess_module(config: &Config, title: &str) -> Option<String> {
    let weights = crate::matcher::Weights::new(&config.courses);
    let mut scores: Vec<(usize, usize)> =
        (0..config.courses.len()).map(|i| (weights.score_name(title, i).0, i)).filter(|(s, _)| *s > 0).collect();
    scores.sort_by_key(|s| std::cmp::Reverse(s.0));
    match scores.as_slice() {
        [(best, i), rest @ ..] if rest.first().is_none_or(|(second, _)| second < best) => {
            Some(config.courses[*i].label())
        }
        _ => None,
    }
}

/// The kind of a lesson from its title («Übung», «Vorlesung», …).
pub fn guess_kind(title: &str) -> String {
    let words = crate::matcher::normalise(title);
    let has = |list: &[&str]| words.split(' ').any(|w| list.iter().any(|l| w.starts_with(l)));
    if has(&["ubung", "uebung", "exercise", "tutorium", "tutorial", "ue"]) {
        "Übung".into()
    } else if has(&["seminar"]) {
        "Seminar".into()
    } else if has(&["praktikum", "lab"]) {
        "Praktikum".into()
    } else if has(&["vorlesung", "vl", "lecture"]) {
        "Vorlesung".into()
    } else {
        String::new()
    }
}

/// The timetable saved in the state folder, read again when the file changed.
pub fn load(config: &Config) -> Option<Timetable> {
    static CACHE: Mutex<Option<(PathBuf, SystemTime, Timetable)>> = Mutex::new(None);
    let file = config.state_file(TIMETABLE_FILE)?;
    let modified = fs::metadata(&file).and_then(|m| m.modified()).ok()?;
    let mut cache = CACHE.lock().ok()?;
    if let Some((path, time, table)) = cache.as_ref()
        && *path == file
        && *time == modified
    {
        return Some(table.clone());
    }
    let table = Timetable::load(&file);
    *cache = Some((file, modified, table.clone()));
    Some(table)
}

/// The modules (labels) whose lesson was running at `at`.
pub fn running_modules(config: &Config, at: NaiveDateTime) -> Vec<String> {
    let Some(table) = load(config) else { return Vec::new() };
    let mut modules: Vec<String> =
        table.lessons.iter().filter(|l| !l.module.is_empty() && l.running(at)).map(|l| l.module.clone()).collect();
    modules.dedup();
    modules
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    const ICS: &str = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\n\
BEGIN:VEVENT\r\nUID:stat-vl@uni\r\nSUMMARY:Statistik I – Vorlesung\r\nLOCATION:HG F 1\r\n\
DTSTART;TZID=Europe/Zurich:20261020T101500\r\nDTEND;TZID=Europe/Zurich:20261020T120000\r\n\
RRULE:FREQ=WEEKLY;UNTIL=20261222T230000Z;BYDAY=TU,TH\r\nEXDATE;TZID=Europe/Zurich:20261103T101500\r\nEND:VEVENT\r\n\
BEGIN:VEVENT\r\nUID:stat-vl@uni\r\nRECURRENCE-ID;TZID=Europe/Zurich:20261110T101500\r\nSUMMARY:Statistik I – Vorlesung\r\n\
LOCATION:HG E 7\r\nDTSTART;TZID=Europe/Zurich:20261111T080000\r\nDTEND;TZID=Europe/Zurich:20261111T100000\r\nEND:VEVENT\r\n\
BEGIN:VEVENT\r\nUID:prog-ue-1\r\nSUMMARY:Übung Programmieren\\, Gruppe 3\r\nLOCATION:CAB G 11\r\n\
DTSTART:20261021T140000\r\nDTEND:20261021T160000\r\nEND:VEVENT\r\n\
BEGIN:VEVENT\r\nUID:prog-ue-2\r\nSUMMARY:Übung Programmieren\\, Gruppe 3\r\nLOCATION:CAB G 11\r\n\
DTSTART:20261104T140000\r\nDTEND:20261104T160000\r\nEND:VEVENT\r\n\
BEGIN:VEVENT\r\nUID:holiday\r\nSUMMARY:Feiertag\r\nDTSTART;VALUE=DATE:20261101\r\nEND:VEVENT\r\n\
BEGIN:VEVENT\r\nUID:la-count\r\nSUMMARY:Lineare Algebra\r\nDTSTART:20261019T080000\r\nDTEND:20261019T100000\r\n\
RRULE:FREQ=WEEKLY;COUNT=3;INTERVAL=2\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";

    #[test]
    fn reads_weekly_series_single_dates_and_moved_ones() {
        let lessons = read_ics(ICS);
        let titles: Vec<(&str, u32, &str)> =
            lessons.iter().map(|l| (l.title.as_str(), l.weekday, l.start.as_str())).collect();
        assert!(titles.contains(&("Statistik I – Vorlesung", 1, "10:15")), "{titles:?}");
        assert!(titles.contains(&("Statistik I – Vorlesung", 3, "10:15")), "Thursday too");
        assert!(titles.contains(&("Statistik I – Vorlesung", 2, "08:00")), "the moved date");
        assert!(titles.contains(&("Übung Programmieren, Gruppe 3", 2, "14:00")));
        assert!(!titles.iter().any(|t| t.0 == "Feiertag"), "whole days are no lessons");
        let tuesday = lessons.iter().find(|l| l.weekday == 1).unwrap();
        assert!(tuesday.on(d(2026, 10, 27)));
        assert!(!tuesday.on(d(2026, 11, 3)), "EXDATE");
        assert!(!tuesday.on(d(2026, 11, 10)), "moved to Wednesday");
        assert!(!tuesday.on(d(2027, 1, 5)), "after UNTIL");
        // Two single dates two weeks apart: one lesson, the week between skipped.
        let exercise = lessons.iter().find(|l| l.title.starts_with("Übung")).unwrap();
        assert!(exercise.on(d(2026, 10, 21)) && exercise.on(d(2026, 11, 4)) && !exercise.on(d(2026, 10, 28)));
        // Every two weeks, three times.
        let la = lessons.iter().find(|l| l.title == "Lineare Algebra").unwrap();
        assert!(la.on(d(2026, 10, 19)) && !la.on(d(2026, 10, 26)) && la.on(d(2026, 11, 16)));
        assert!(!la.on(d(2026, 11, 30)), "after three");
    }

    #[test]
    fn running_lessons_and_guesses() {
        let lesson = Lesson {
            weekday: 1,
            start: "10:15".into(),
            end: "12:00".into(),
            from: "2026-10-20".into(),
            until: "2026-12-22".into(),
            every: 1,
            ..Default::default()
        };
        let at = |h: u32, m: u32| d(2026, 10, 27).and_hms_opt(h, m, 0).unwrap();
        assert!(lesson.running(at(10, 5)) && lesson.running(at(12, 10)));
        assert!(!lesson.running(at(9, 50)) && !lesson.running(at(12, 30)));
        // The first day needn't be on the lesson's weekday.
        let thursday = Lesson { weekday: 3, from: "2026-10-19".into(), every: 2, ..lesson.clone() };
        assert!(thursday.on(d(2026, 10, 22)) && !thursday.on(d(2026, 10, 29)) && thursday.on(d(2026, 11, 5)));
        assert_eq!(guess_kind("Übung Programmieren, Gruppe 3"), "Übung");
        assert_eq!(guess_kind("Statistik I – Vorlesung"), "Vorlesung");
        let mut config = crate::sorter::tests::test_config(Path::new("/tmp/x"));
        config.courses = vec![
            crate::config::Course::new("Statistik", &["stat"]),
            crate::config::Course::new("Programmieren", &["prog"]),
        ];
        assert_eq!(guess_module(&config, "Statistik I – Vorlesung").as_deref(), Some("Statistik"));
        assert_eq!(guess_module(&config, "Übung Programmieren, Gruppe 3").as_deref(), Some("Programmieren"));
        assert_eq!(guess_module(&config, "Sport"), None);
    }
}
