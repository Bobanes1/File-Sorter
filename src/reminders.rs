//! Reminders before deadlines and exams: a system notification (Windows' own, or
//! `notify-send` on Linux) at the chosen times before a date, each sent once.

use chrono::{Duration, NaiveDateTime, NaiveTime};
use serde::{Deserialize, Serialize};

use crate::deadlines::{Deadline, Deadlines};

/// When before a deadline to remind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Lead {
    /// A day before.
    Day,
    /// Three hours before.
    Hours3,
    /// An hour before.
    Hour,
    /// At 8:00 on the day (when it is due later that day).
    Morning,
}

impl Lead {
    pub const ALL: [Lead; 4] = [Lead::Day, Lead::Morning, Lead::Hours3, Lead::Hour];

    fn key(self) -> &'static str {
        match self {
            Lead::Day => "day",
            Lead::Hours3 => "hours3",
            Lead::Hour => "hour",
            Lead::Morning => "morning",
        }
    }

    /// When to remind of something due at `due`, if this lead fits it.
    fn at(self, due: NaiveDateTime) -> Option<NaiveDateTime> {
        match self {
            Lead::Day => Some(due - Duration::days(1)),
            Lead::Hours3 => Some(due - Duration::hours(3)),
            Lead::Hour => Some(due - Duration::hours(1)),
            Lead::Morning => {
                let morning = due.date().and_time(NaiveTime::from_hms_opt(8, 0, 0)?);
                (morning < due).then_some(morning)
            }
        }
    }
}

pub fn default_leads() -> Vec<Lead> {
    vec![Lead::Day, Lead::Hours3]
}

/// The deadlines to remind of now (by id), and mark them reminded. Of several reminders that
/// came due meanwhile (the app wasn't running, or the deadline was found late), only one is
/// sent; nothing for what is already past.
pub fn take_due(deadlines: &mut Deadlines, leads: &[Lead], now: NaiveDateTime) -> Vec<Deadline> {
    let mut out = Vec::new();
    for d in deadlines.items.iter_mut().filter(|d| !d.done && !d.deleted) {
        let Some(due) = d.due() else { continue };
        if due <= now {
            continue;
        }
        let passed: Vec<Lead> = leads
            .iter()
            .copied()
            .filter(|l| l.at(due).is_some_and(|at| at <= now) && !d.reminded.iter().any(|r| r == l.key()))
            .collect();
        if passed.is_empty() {
            continue;
        }
        for lead in passed {
            d.reminded.push(lead.key().to_string());
        }
        out.push(d.clone());
    }
    out
}

/// Show a system notification. Returns false where there is none.
pub fn notify(title: &str, body: &str) -> bool {
    #[cfg(windows)]
    {
        match windows_toast::show(title, body) {
            Ok(()) => true,
            Err(e) => {
                log::info!("Could not show a notification: {e}");
                false
            }
        }
    }
    #[cfg(not(windows))]
    {
        std::process::Command::new("notify-send")
            .args(["--app-name=File Sorter", "--icon=appointment-soon", title, body])
            .status()
            .is_ok_and(|s| s.success())
    }
}

#[cfg(windows)]
mod windows_toast {
    use windows::Data::Xml::Dom::XmlDocument;
    use windows::UI::Notifications::{ToastNotification, ToastNotificationManager};
    use windows::core::{HSTRING, Result};

    /// The name Windows shows the notifications under.
    const APP_ID: &str = "FileSorter.FileSorter";

    fn escape(text: &str) -> String {
        text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
    }

    /// Tell Windows the name and icon for the notifications, and that a click opens File
    /// Sorter (the `filesorter:` link starts it, which shows the open window).
    fn register() {
        use winreg::RegKey;
        use winreg::enums::HKEY_CURRENT_USER;
        let Ok(exe) = std::env::current_exe() else { return };
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let icon = dirs::data_local_dir().map(|d| d.join("FileSorter").join("file-sorter.png"));
        if let Some(icon) = &icon
            && !icon.exists()
        {
            let _ = std::fs::create_dir_all(icon.parent().expect("has a folder"));
            let _ = std::fs::write(icon, include_bytes!("../assets/icon.png"));
        }
        if let Ok((key, _)) = hkcu.create_subkey(format!("Software\\Classes\\AppUserModelId\\{APP_ID}")) {
            let _ = key.set_value("DisplayName", &"File Sorter");
            if let Some(icon) = &icon {
                let _ = key.set_value("IconUri", &icon.display().to_string());
            }
        }
        if let Ok((key, _)) = hkcu.create_subkey("Software\\Classes\\filesorter") {
            let _ = key.set_value("", &"URL:File Sorter");
            let _ = key.set_value("URL Protocol", &"");
            if let Ok((command, _)) = key.create_subkey("shell\\open\\command") {
                let _ = command.set_value("", &format!("\"{}\"", exe.display()));
            }
        }
    }

    pub fn show(title: &str, body: &str) -> Result<()> {
        static REGISTERED: std::sync::Once = std::sync::Once::new();
        REGISTERED.call_once(register);
        let xml = format!(
            "<toast activationType=\"protocol\" launch=\"filesorter:week\" scenario=\"reminder\">\
             <visual><binding template=\"ToastGeneric\"><text>{}</text><text>{}</text></binding></visual>\
             <actions><action content=\"OK\" arguments=\"dismiss\" activationType=\"system\"/></actions></toast>",
            escape(title),
            escape(body)
        );
        let document = XmlDocument::new()?;
        document.LoadXml(&HSTRING::from(xml))?;
        let toast = ToastNotification::CreateToastNotification(&document)?;
        ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(APP_ID))?.Show(&toast)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::deadlines::Kind;
    use chrono::NaiveDate;

    fn at(d: u32, h: u32, m: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(2026, 10, d).unwrap().and_hms_opt(h, m, 0).unwrap()
    }

    #[test]
    fn reminds_once_at_each_time_and_catches_up_with_one() {
        let mut list = Deadlines::default();
        let due = at(24, 23, 59);
        list.add_manual("Blatt 3", "Prog", Kind::Due, due.date(), Some(due.time()));
        let leads = default_leads();
        assert!(take_due(&mut list, &leads, at(23, 12, 0)).is_empty(), "too early");
        assert_eq!(take_due(&mut list, &leads, at(23, 23, 59)).len(), 1, "a day before");
        assert!(take_due(&mut list, &leads, at(24, 12, 0)).is_empty(), "already reminded");
        assert_eq!(take_due(&mut list, &leads, at(24, 21, 0)).len(), 1, "three hours before");
        assert!(take_due(&mut list, &leads, at(24, 22, 0)).is_empty());
        // Not running for days: one reminder, not two; past ones none.
        let mut late = Deadlines::default();
        late.add_manual("Blatt 4", "Prog", Kind::Due, due.date(), Some(due.time()));
        assert_eq!(take_due(&mut late, &leads, at(24, 22, 0)).len(), 1);
        assert!(take_due(&mut late, &leads, at(24, 22, 30)).is_empty());
        let mut past = Deadlines::default();
        past.add_manual("Blatt 5", "Prog", Kind::Due, due.date(), Some(due.time()));
        assert!(take_due(&mut past, &leads, at(25, 9, 0)).is_empty());
    }

    #[test]
    fn morning_reminder_only_for_later_that_day() {
        assert_eq!(Lead::Morning.at(at(24, 14, 0)), Some(at(24, 8, 0)));
        assert_eq!(Lead::Morning.at(at(24, 7, 0)), None);
    }
}
