//! All texts shown in the window, in English and German.
//!
//! Every text is one method that names both languages, so a translation can't be
//! forgotten. German uses Swiss spelling ("ss", «…»).

use crate::cloud::Provider;
use crate::matcher::Reason;
use crate::sorter::LeftReason;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    En,
    De,
}

impl Lang {
    /// The language the system runs in: German for any `de*` locale, otherwise English.
    pub fn system() -> Lang {
        Self::from_locale(sys_locale::get_locale().as_deref().unwrap_or("en"))
    }

    pub fn from_locale(locale: &str) -> Lang {
        if locale.to_lowercase().starts_with("de") { Lang::De } else { Lang::En }
    }

    /// The configured language ("de" / "en"), or the system language if none is set.
    pub fn from_setting(setting: Option<&str>) -> Lang {
        match setting {
            Some(code) => Self::from_locale(code),
            None => Self::system(),
        }
    }

    pub fn code(self) -> &'static str {
        match self {
            Lang::En => "en",
            Lang::De => "de",
        }
    }

    fn pick(self, en: &'static str, de: &'static str) -> &'static str {
        match self {
            Lang::En => en,
            Lang::De => de,
        }
    }

    fn fmt(self, en: String, de: String) -> String {
        match self {
            Lang::En => en,
            Lang::De => de,
        }
    }

    // ---------- header and stats ----------
    pub fn language(self) -> &'static str {
        self.pick("Language", "Sprache")
    }
    pub fn stat_sorted_today(self) -> &'static str {
        self.pick("Sorted today", "Heute sortiert")
    }
    pub fn stat_waiting(self) -> &'static str {
        self.pick("Waiting for download", "Wartet auf Download")
    }
    pub fn stat_unmatched(self) -> &'static str {
        self.pick("Not assigned", "Nicht zugeordnet")
    }

    // ---------- folders ----------
    pub fn layout_hint(self) -> &'static str {
        self.pick("Layout: Study folder \\ Study program \\ Module", "Aufbau: Studienordner \\ Studienfach \\ Modul")
    }
    pub fn import_modules(self) -> &'static str {
        self.pick("Import module folders", "Modulordner einlesen")
    }

    // ---------- modules ----------
    pub fn modules(self) -> &'static str {
        self.pick("Modules", "Module")
    }
    pub fn no_modules_yet(self) -> &'static str {
        self.pick("No modules yet – import or add some.", "Noch keine Module – einlesen oder hinzufügen.")
    }
    pub fn add(self) -> &'static str {
        self.pick("Add", "Hinzufügen")
    }
    pub fn edit(self) -> &'static str {
        self.pick("Edit", "Bearbeiten")
    }
    pub fn delete(self) -> &'static str {
        self.pick("Delete", "Löschen")
    }
    pub fn select_module(self) -> &'static str {
        self.pick("Select a module", "Modul auswählen")
    }
    pub fn suggest_keywords(self) -> &'static str {
        self.pick("Suggest keywords from files", "Stichwörter aus Dateien vorschlagen")
    }
    pub fn suggest_tooltip(self) -> &'static str {
        self.pick(
            "Looks through the files already in this module's folder and adds terms that are typical for this \
             module (module code, lecturer, topics) but rare in your other modules.",
            "Liest die Dateien, die schon im Ordner dieses Moduls liegen, und fügt Begriffe hinzu, die typisch \
             für dieses Modul sind (Modulkürzel, Dozent/in, Themen), in deinen anderen Modulen aber kaum vorkommen.",
        )
    }
    pub fn scanning(self) -> &'static str {
        self.pick("Reading files…", "Dateien werden gelesen…")
    }
    pub fn files_go_to(self) -> &'static str {
        self.pick("Files go to:", "Dateien landen in:")
    }

    // ---------- controls ----------
    pub fn start(self) -> &'static str {
        self.pick("Start watching", "Überwachung starten")
    }
    pub fn stop(self) -> &'static str {
        self.pick("Stop watching", "Überwachung stoppen")
    }
    pub fn sort_now(self) -> &'static str {
        self.pick("Sort existing files", "Vorhandene Dateien sortieren")
    }
    pub fn sort_now_tooltip(self, range: crate::config::SortRange) -> String {
        use crate::config::SortRange;
        match range {
            SortRange::Day => self.pick(
                "Sort the files in the downloads folder from the last 24 hours",
                "Die Dateien der letzten 24 Stunden im Download-Ordner sortieren",
            ),
            SortRange::Week => self.pick(
                "Sort the files in the downloads folder from the last 7 days",
                "Die Dateien der letzten 7 Tage im Download-Ordner sortieren",
            ),
            SortRange::All => {
                self.pick("Sort all files in the downloads folder", "Alle Dateien im Download-Ordner sortieren")
            }
        }
        .into()
    }
    /// Short name of a range, for the menu button and the settings ("Last 24 hours").
    pub fn range_short(self, range: crate::config::SortRange) -> &'static str {
        use crate::config::SortRange;
        match range {
            SortRange::Day => self.pick("Last 24 hours", "Letzte 24 Stunden"),
            SortRange::Week => self.pick("Last 7 days", "Letzte 7 Tage"),
            SortRange::All => self.pick("All files", "Alle Dateien"),
        }
    }
    pub fn range_title(self, range: crate::config::SortRange) -> &'static str {
        use crate::config::SortRange;
        match range {
            SortRange::Day => self.pick("Only the last 24 hours", "Nur die letzten 24 Stunden"),
            SortRange::Week => self.pick("Only the last 7 days", "Nur die letzten 7 Tage"),
            SortRange::All => self.pick("All files", "Alle Dateien"),
        }
    }
    pub fn range_hint(self, range: crate::config::SortRange) -> &'static str {
        use crate::config::SortRange;
        match range {
            SortRange::Day => self.pick(
                "Files downloaded or changed since yesterday at this time",
                "Dateien, die seit gestern um diese Zeit heruntergeladen oder geändert wurden",
            ),
            SortRange::Week => self.pick(
                "Files downloaded or changed in the last 7 days",
                "Dateien, die in den letzten 7 Tagen heruntergeladen oder geändert wurden",
            ),
            SortRange::All => {
                self.pick("Everything in the downloads folder, however old", "Alles im Download-Ordner, egal wie alt")
            }
        }
    }

    // ---------- activity ----------
    pub fn activity(self) -> &'static str {
        self.pick("Activity", "Aktivität")
    }
    pub fn activity_empty(self) -> &'static str {
        self.pick(
            "Nothing happened yet. Start watching and download a file.",
            "Noch nichts passiert. Starte die Überwachung und lade eine Datei herunter.",
        )
    }
    pub fn waiting_for_download(self) -> &'static str {
        self.pick("Waiting for the download to finish…", "Wartet, bis der Download fertig ist…")
    }
    pub fn vanished(self) -> &'static str {
        self.pick("Download cancelled or file removed", "Download abgebrochen oder Datei entfernt")
    }
    pub fn moved_to(self, course: &str) -> String {
        self.fmt(format!("Moved to {course}"), format!("Verschoben nach {course}"))
    }
    pub fn copied_to(self, course: &str) -> String {
        self.fmt(format!("Copied to {course}"), format!("Kopiert nach {course}"))
    }
    pub fn duplicate_removed(self, course: &str) -> String {
        self.fmt(
            format!("Already in {course} – duplicate moved to the Recycle Bin"),
            format!("Schon in {course} – Duplikat in den Papierkorb verschoben"),
        )
    }
    pub fn saved_as(self, name: &str) -> String {
        self.fmt(
            format!("saved as «{name}» – another version is already there"),
            format!("gespeichert als «{name}» – dort lag schon eine andere Version"),
        )
    }

    // ---------- undo ----------
    pub fn undo(self) -> &'static str {
        self.pick("Undo", "Rückgängig")
    }
    pub fn undoing(self) -> &'static str {
        self.pick("Putting it back…", "Wird zurückgelegt…")
    }
    pub fn undone(self, folder: &str) -> String {
        self.fmt(format!("Put back – stays in {folder}"), format!("Zurückgelegt – bleibt in {folder}"))
    }
    pub fn undo_failed(self, error: &crate::undo::UndoError) -> String {
        use crate::undo::UndoError;
        match error {
            UndoError::Missing => self
                .pick(
                    "Can't put it back: the file isn't where File Sorter put it any more",
                    "Zurücklegen nicht möglich: die Datei liegt nicht mehr dort, wo File Sorter sie hingelegt hat",
                )
                .into(),
            UndoError::InUse => self
                .pick(
                    "Can't put it back while it is open in another program",
                    "Zurücklegen nicht möglich, solange sie in einem anderen Programm geöffnet ist",
                )
                .into(),
            UndoError::NotSorted => self.pick("Nothing to undo", "Nichts rückgängig zu machen").into(),
            UndoError::CopyChanged => self
                .pick(
                    "The copy was changed since (notes?), so it stays where it is",
                    "Die Kopie wurde inzwischen verändert (Notizen?) und bleibt deshalb, wo sie ist",
                )
                .into(),
            UndoError::Failed(e) => {
                self.fmt(format!("Can't put it back: {e}"), format!("Zurücklegen nicht möglich: {e}"))
            }
        }
    }
    pub fn snack_sorted(self, name: &str, course: &str, copied: bool) -> String {
        if copied {
            self.fmt(format!("«{name}» copied to {course}"), format!("«{name}» nach {course} kopiert"))
        } else {
            self.fmt(format!("«{name}» moved to {course}"), format!("«{name}» nach {course} verschoben"))
        }
    }
    pub fn snack_duplicate(self, name: &str) -> String {
        self.fmt(
            format!("«{name}» was already there – duplicate in the Recycle Bin"),
            format!("«{name}» war schon da – Duplikat im Papierkorb"),
        )
    }
    pub fn snack_undone(self, name: &str) -> String {
        self.fmt(format!("«{name}» put back"), format!("«{name}» zurückgelegt"))
    }
    pub fn move_to(self, module: &str) -> String {
        self.fmt(format!("Move to {module}"), format!("Nach {module} verschieben"))
    }
    pub fn reason(self, reason: &Reason) -> String {
        let words = reason.keywords().join(", ");
        match reason {
            Reason::FileName(_) => {
                self.fmt(format!("file name contains «{words}»"), format!("Dateiname enthält «{words}»"))
            }
            Reason::Content(_) => self.fmt(format!("content mentions «{words}»"), format!("Inhalt erwähnt «{words}»")),
            Reason::Chosen => self.pick("assigned by you", "von dir zugeordnet").into(),
            Reason::Origin(host) => self.fmt(
                format!("from {host}, like earlier files of this module"),
                format!("von {host}, wie frühere Dateien dieses Moduls"),
            ),
            Reason::Timetable(module) => {
                let short = module.rsplit(" › ").next().unwrap_or(module);
                self.fmt(
                    format!("modules fit equally well – {short} was on the timetable"),
                    format!("Module passten gleich gut – {short} lief laut Stundenplan gerade"),
                )
            }
            Reason::Learned(_) if words.is_empty() => {
                self.pick("like the module's other files", "ähnlich wie die anderen Dateien des Moduls").into()
            }
            Reason::Learned(_) => self.fmt(
                format!("like the module's other files («{words}»)"),
                format!("ähnlich wie die anderen Dateien des Moduls («{words}»)"),
            ),
        }
    }
    pub fn left_reason(self, reason: &LeftReason) -> String {
        match reason {
            LeftReason::Gone => self.pick("File disappeared", "Datei ist verschwunden").into(),
            LeftReason::NotDocument => self.pick("Not a document type", "Kein Dokument-Dateityp").into(),
            LeftReason::NoMatch => {
                self.pick("No matching module – stays where it is", "Kein passendes Modul – bleibt, wo sie ist").into()
            }
            LeftReason::AlreadyInFolder { .. } => {
                self.pick("Already in the module folder", "Liegt schon im Modulordner").into()
            }
            LeftReason::InUse { course } => self.fmt(
                format!("Still open in another program – goes to {course} as soon as it's free"),
                format!("Noch in einem anderen Programm geöffnet – kommt nach {course}, sobald sie frei ist"),
            ),
            LeftReason::AlreadyCopied { course, .. } => {
                self.fmt(format!("Already copied to {course} earlier"), format!("Wurde schon nach {course} kopiert"))
            }
            LeftReason::StillOpen { .. } => self
                .pick(
                    "Still open in another program – close it and try again",
                    "Noch in einem anderen Programm geöffnet – schliesse es und versuch es nochmal",
                )
                .into(),
            LeftReason::Kept => {
                self.pick("Stays here (sorting was undone)", "Bleibt hier (Sortieren rückgängig gemacht)").into()
            }
            LeftReason::Error(e) => self.fmt(format!("Error: {e}"), format!("Fehler: {e}")),
            LeftReason::Unsure { candidates } => {
                let names: Vec<&str> = candidates.iter().map(|c| c.rsplit(" › ").next().unwrap_or(c)).collect();
                self.fmt(
                    format!("Unsure: {}? Choose the module", names.join(" or ")),
                    format!("Unsicher: {}? Bitte Modul wählen", names.join(" oder ")),
                )
            }
        }
    }
    pub fn started(self, folder: &str) -> String {
        self.fmt(
            format!("Watching {folder} – new downloads are sorted automatically."),
            format!("Überwache {folder} – neue Downloads werden automatisch sortiert."),
        )
    }
    pub fn stopped_watching(self) -> &'static str {
        self.pick("Stopped watching.", "Überwachung gestoppt.")
    }
    pub fn older_version_moved(self, count: usize, folder: &str) -> String {
        match count {
            1 => self.fmt(
                format!("older version moved to «{folder}»"),
                format!("ältere Version nach «{folder}» verschoben"),
            ),
            n => self.fmt(
                format!("{n} older versions moved to «{folder}»"),
                format!("{n} ältere Versionen nach «{folder}» verschoben"),
            ),
        }
    }
    pub fn unpacked_into(self, folder: &str, files: usize) -> String {
        self.fmt(
            format!("unpacked into «{folder}» ({files} files)"),
            format!("entpackt in «{folder}» ({files} Dateien)"),
        )
    }
    pub fn not_unpacked(self, error: &str) -> String {
        self.fmt(format!("not unpacked: {error}"), format!("nicht entpackt: {error}"))
    }
    pub fn versions_folder_hint(self) -> &'static str {
        self.pick("Folder for older versions", "Ordner für ältere Versionen")
    }
    pub fn assign(self) -> &'static str {
        self.pick("Assign", "Zuordnen")
    }
    pub fn other_module(self) -> &'static str {
        self.pick("Other module", "Anderes Modul")
    }
    pub fn assign_hint(self) -> &'static str {
        self.pick("Put this file into a module of your choice", "Diese Datei in ein Modul deiner Wahl legen")
    }
    pub fn assign_title(self) -> &'static str {
        self.pick("Move into", "Verschieben nach")
    }
    pub fn assigning(self) -> &'static str {
        self.pick("Moving…", "Wird verschoben…")
    }
    pub fn learned(self, module: &str, count: usize, undone: bool) -> String {
        match (undone, count) {
            (true, _) => self
                .fmt(format!("Keywords for {module} taken back"), format!("Stichwörter für {module} zurückgenommen")),
            (false, 1) => {
                self.fmt(format!("Learned a keyword for {module}"), format!("Neues Stichwort für {module} gelernt"))
            }
            (false, _) => {
                self.fmt(format!("Learned keywords for {module}"), format!("Neue Stichwörter für {module} gelernt"))
            }
        }
    }
    pub fn sorting_existing(self, hours: Option<u32>) -> String {
        match hours {
            Some(h) if h % 24 != 0 || h == 24 => self.fmt(
                format!("Sorting the files from the last {h} hours…"),
                format!("Sortiere die Dateien der letzten {h} Stunden…"),
            ),
            Some(h) => self.fmt(
                format!("Sorting the files from the last {} days…", h / 24),
                format!("Sortiere die Dateien der letzten {} Tage…", h / 24),
            ),
            None => self
                .pick("Sorting files already in the folder…", "Sortiere die Dateien, die schon im Ordner liegen…")
                .into(),
        }
    }
    pub fn imported(self, added: usize, known: usize) -> String {
        self.fmt(
            format!("Imported {added} module(s) ({known} already known)"),
            format!("{added} Modul(e) eingelesen ({known} schon bekannt)"),
        )
    }
    pub fn keywords_added(self, count: usize, module: &str) -> String {
        self.fmt(
            format!("Added {count} keyword(s) to {module}"),
            format!("{count} Stichwort/Stichwörter zu {module} hinzugefügt"),
        )
    }
    pub fn no_new_keywords(self, module: &str, files: usize) -> String {
        self.fmt(
            format!("No new keywords found for {module} in {files} file(s)"),
            format!("Keine neuen Stichwörter für {module} in {files} Datei(en) gefunden"),
        )
    }
    pub fn no_files_to_scan(self, module: &str) -> String {
        self.fmt(format!("No files to read for {module}"), format!("Keine Dateien zum Lesen für {module}"))
    }
    pub fn folder_empty(self, folder: &str) -> String {
        self.fmt(
            format!("{folder} is empty or doesn't exist yet"),
            format!("{folder} ist leer oder existiert noch nicht"),
        )
    }
    pub fn scan_failed(self) -> &'static str {
        self.pick("Reading the files failed.", "Das Lesen der Dateien ist fehlgeschlagen.")
    }
    pub fn config_unreadable(self, path: &str, error: &str) -> String {
        self.fmt(
            format!("Could not read {path} ({error}); starting with default settings."),
            format!("{path} konnte nicht gelesen werden ({error}); starte mit Standard-Einstellungen."),
        )
    }

    // ---------- right-click menu and quick sorting ----------
    pub fn context_menu_label(self) -> &'static str {
        self.pick("Sort with File Sorter", "Mit File Sorter sortieren")
    }
    pub fn open_folder_hint(self) -> &'static str {
        self.pick("Click to show the file in its folder", "Klicken, um die Datei in ihrem Ordner zu zeigen")
    }
    pub fn quick_sorting(self) -> &'static str {
        self.pick("Sorting…", "Wird sortiert…")
    }
    pub fn quick_done(self, sorted: usize, total: usize) -> String {
        self.fmt(format!("Done – {sorted} of {total} sorted"), format!("Fertig – {sorted} von {total} einsortiert"))
    }
    pub fn closes_in(self, seconds: u32) -> String {
        self.fmt(format!("Closes in {seconds} s"), format!("Schliesst in {seconds} s"))
    }
    pub fn close(self) -> &'static str {
        self.pick("Close", "Schliessen")
    }
    pub fn keep_open(self) -> &'static str {
        self.pick("Keep open", "Offen lassen")
    }

    // ---------- background and tray ----------
    pub fn lean_background(self) -> &'static str {
        self.pick("Save resources in the background", "Im Hintergrund Ressourcen sparen")
    }
    pub fn lean_background_hint(self) -> &'static str {
        self.pick(
            "Less memory and computing while the window is closed or minimised; in return files are read a little less thoroughly (name and first page, no text recognition). With the window open, everything runs in full.",
            "Weniger Arbeitsspeicher und Rechenzeit, solange das Fenster geschlossen oder minimiert ist; dafür werden Dateien etwas weniger gründlich erkannt (Name und erste Seite, keine Texterkennung). Mit offenem Fenster läuft alles wieder voll.",
        )
    }
    pub fn background(self) -> &'static str {
        self.pick("Keep running in the background", "Im Hintergrund weiterlaufen")
    }
    pub fn background_hint(self) -> &'static str {
        self.pick(
            "Closing the window keeps sorting while watching; the icon next to the clock opens it again.",
            "Beim Schliessen des Fensters wird weiter sortiert; das Symbol neben der Uhr öffnet es wieder.",
        )
    }
    #[cfg_attr(not(windows), allow(dead_code))]
    pub fn tray_open(self) -> &'static str {
        self.pick("Open File Sorter", "File Sorter öffnen")
    }
    #[cfg_attr(not(windows), allow(dead_code))]
    pub fn tray_quit(self) -> &'static str {
        self.pick("Quit", "Beenden")
    }
    pub fn tray_tooltip(self, watching: Option<&str>) -> String {
        match watching {
            Some(folder) => {
                self.fmt(format!("File Sorter – watching {folder}"), format!("File Sorter – überwacht {folder}"))
            }
            None => self.pick("File Sorter – not watching", "File Sorter – Überwachung aus").into(),
        }
    }

    // ---------- keyword preview ----------
    pub fn kw_preview_title(self, module: &str) -> String {
        self.fmt(format!("Keywords for {module}"), format!("Stichwörter für {module}"))
    }
    pub fn kw_preview_text(self, files: usize) -> String {
        self.fmt(
            format!("Found in the {files} file(s) of this module. Untick what doesn't fit."),
            format!("Gefunden in den {files} Datei(en) dieses Moduls. Entferne das Häkchen bei Unpassendem."),
        )
    }
    pub fn kw_add(self, count: usize) -> String {
        self.fmt(format!("Add ({count})"), format!("Hinzufügen ({count})"))
    }
    pub fn kw_kind(self, kind: crate::keywords::Kind) -> &'static str {
        use crate::keywords::Kind;
        match kind {
            Kind::Code => self.pick("module code", "Modulkürzel"),
            Kind::Lecturer => self.pick("lecturer", "Dozent/in"),
            Kind::Title => self.pick("from titles", "aus Titeln"),
            Kind::Topic => self.pick("topic", "Thema"),
        }
    }
    pub fn in_files(self, n: usize, total: usize) -> String {
        self.fmt(format!("in {n} of {total} files"), format!("in {n} von {total} Dateien"))
    }

    // ---------- overview ----------
    pub fn field_watch(self) -> &'static str {
        self.pick("Watched folder (Downloads)", "Überwachter Ordner (Downloads)")
    }
    pub fn field_study(self) -> &'static str {
        self.pick("Study folder", "Studienordner")
    }
    pub fn change(self) -> &'static str {
        self.pick("Change", "Ändern")
    }
    pub fn keywords_title(self) -> &'static str {
        self.pick("Keywords", "Stichwörter")
    }
    pub fn keywords_hint(self) -> &'static str {
        self.pick("One per line: module code, short name, lecturer", "Eins pro Zeile: Modulkürzel, Kurzname, Dozent/in")
    }
    pub fn paused(self) -> &'static str {
        self.pick("Paused", "Pausiert")
    }

    // ---------- search and this week ----------
    pub fn tab_week(self) -> &'static str {
        self.pick("This week", "Diese Woche")
    }
    pub fn tab_semester(self) -> &'static str {
        self.pick("Timetable", "Stundenplan")
    }
    pub fn tab_grades(self) -> &'static str {
        self.pick("Grades", "Noten")
    }

    // ---------- timetable ----------
    pub fn timetable(self) -> &'static str {
        self.pick("Timetable", "Stundenplan")
    }
    pub fn previous_week(self) -> &'static str {
        self.pick("Previous week", "Vorherige Woche")
    }
    pub fn next_week(self) -> &'static str {
        self.pick("Next week", "Nächste Woche")
    }
    pub fn calendar_week(self) -> &'static str {
        self.pick("Week", "KW")
    }
    pub fn today(self) -> &'static str {
        self.pick("Today", "Heute")
    }
    pub fn add_lesson(self) -> &'static str {
        self.pick("Lesson", "Lektion")
    }
    pub fn lesson(self) -> &'static str {
        self.pick("Lesson", "Lektion")
    }
    pub fn calendar_link(self) -> &'static str {
        self.pick("Calendar link…", "Kalender-Link…")
    }
    pub fn calendar_link_hint(self) -> &'static str {
        self.pick(
            "Subscribe to the timetable link of your university (iCal/webcal); it is read again every day",
            "Den Stundenplan-Link deiner Hochschule abonnieren (iCal/webcal); er wird täglich neu gelesen",
        )
    }
    pub fn calendar_link_explained(self) -> &'static str {
        self.pick(
            "Paste the calendar link (iCal, .ics or webcal://) your university's portal offers for your timetable. File Sorter reads it now and again every day.",
            "Füge den Kalender-Link (iCal, .ics oder webcal://) ein, den das Portal deiner Hochschule für deinen Stundenplan anbietet. File Sorter liest ihn jetzt und danach täglich.",
        )
    }
    pub fn read_link(self) -> &'static str {
        self.pick("Read", "Lesen")
    }
    pub fn unsubscribe(self) -> &'static str {
        self.pick("Remove this link and its lessons", "Diesen Link und seine Lektionen entfernen")
    }
    pub fn subscribed(self, links: &str) -> String {
        self.fmt(format!("Read daily from: {links}"), format!("Täglich gelesen von: {links}"))
    }
    pub fn import_calendar(self) -> &'static str {
        self.pick("Import calendar…", "Kalender importieren…")
    }
    pub fn import_calendar_hint(self) -> &'static str {
        self.pick(
            "Read the lessons from a calendar file (.ics) exported from the university portal, Outlook or Google",
            "Die Lektionen aus einer Kalenderdatei (.ics) lesen, exportiert aus dem Uni-Portal, Outlook oder Google",
        )
    }
    pub fn import_explained(self, count: usize) -> String {
        self.fmt(
            format!("{count} lessons found. Check the module of each (guessed from its name) – what came from the same calendar before is replaced."),
            format!("{count} Lektionen gefunden. Prüfe bei jeder das Modul (am Namen erraten) – was früher aus demselben Kalender kam, wird ersetzt."),
        )
    }
    pub fn import_now(self) -> &'static str {
        self.pick("Import", "Importieren")
    }
    pub fn no_lessons_found(self) -> &'static str {
        self.pick("No lessons found in this calendar.", "In diesem Kalender wurden keine Lektionen gefunden.")
    }
    pub fn timetable_empty(self) -> &'static str {
        self.pick(
            "No timetable yet. Import the calendar from your university portal (.ics file or link), or add lessons by hand with «+ Lesson».",
            "Noch kein Stundenplan. Importiere den Kalender aus dem Portal deiner Hochschule (.ics-Datei oder Link) oder trage Lektionen mit «+ Lektion» von Hand ein.",
        )
    }
    pub fn timetable_sorting_note(self) -> &'static str {
        self.pick(
            "When sorting, the timetable only settles a tie: if two modules fit a file equally well, the one whose lesson was running wins – never against what the file says.",
            "Beim Sortieren entscheidet der Stundenplan nur bei Gleichstand: Passen zwei Module gleich gut, gewinnt das, dessen Lektion gerade lief – nie gegen den Inhalt der Datei.",
        )
    }
    pub fn weekday_short(self, day: u32) -> &'static str {
        let i = day.min(6) as usize;
        match self {
            Lang::De => ["Mo", "Di", "Mi", "Do", "Fr", "Sa", "So"][i],
            Lang::En => ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"][i],
        }
    }
    pub fn weekday_long(self, day: u32) -> &'static str {
        let i = day.min(6) as usize;
        match self {
            Lang::De => ["Montag", "Dienstag", "Mittwoch", "Donnerstag", "Freitag", "Samstag", "Sonntag"][i],
            Lang::En => ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"][i],
        }
    }
    pub fn cancelled(self) -> &'static str {
        self.pick("cancelled", "fällt aus")
    }
    pub fn lesson_title(self) -> &'static str {
        self.pick("Title", "Titel")
    }
    pub fn weekday_label(self) -> &'static str {
        self.pick("Day", "Tag")
    }
    pub fn time_from_to(self) -> &'static str {
        self.pick("From – to", "Von – bis")
    }
    pub fn every_label(self) -> &'static str {
        self.pick("Repeats", "Wiederholung")
    }
    pub fn every_weeks(self, n: u32) -> &'static str {
        match n {
            2 => self.pick("every 2 weeks", "alle 2 Wochen"),
            _ => self.pick("every week", "jede Woche"),
        }
    }
    pub fn lesson_room(self) -> &'static str {
        self.pick("Room", "Raum")
    }
    pub fn lesson_kind(self) -> &'static str {
        self.pick("Kind", "Art")
    }
    pub fn first_day(self) -> &'static str {
        self.pick("First day", "Erster Tag")
    }
    pub fn last_day(self) -> &'static str {
        self.pick("Last day", "Letzter Tag")
    }
    pub fn lesson_not_ready(self) -> &'static str {
        self.pick("Times like 10:15, dates like 20.10.2026", "Zeiten wie 10:15, Daten wie 20.10.2026")
    }
    pub fn cancel_on(self, date: chrono::NaiveDate) -> String {
        let d = date.format("%d.%m.");
        self.fmt(format!("Cancelled on {d}"), format!("Fällt am {d} aus"))
    }
    pub fn takes_place(self, date: chrono::NaiveDate) -> String {
        let d = date.format("%d.%m.");
        self.fmt(format!("Takes place on {d} after all"), format!("Findet am {d} doch statt"))
    }
    pub fn running_now(self) -> &'static str {
        self.pick("on now", "läuft gerade")
    }
    pub fn today_lessons(self) -> &'static str {
        self.pick("Today", "Heute")
    }

    // ---------- grades ----------
    pub fn grades_title(self) -> &'static str {
        self.pick("Grades & ECTS", "Noten & ECTS")
    }
    pub fn scale_swiss(self) -> &'static str {
        self.pick("1–6 (CH)", "1–6 (CH)")
    }
    pub fn scale_german(self) -> &'static str {
        self.pick("1.0–5.0 (DE)", "1,0–5,0 (DE)")
    }
    pub fn semester_label(self) -> &'static str {
        self.pick("Semester", "Semester")
    }
    pub fn grade_label(self) -> &'static str {
        self.pick("Grade", "Note")
    }
    pub fn status_label(self) -> &'static str {
        self.pick("Status", "Status")
    }
    pub fn passed(self) -> &'static str {
        self.pick("passed", "bestanden")
    }
    pub fn failed(self) -> &'static str {
        self.pick("failed", "nicht bestanden")
    }
    pub fn open_status(self) -> &'static str {
        self.pick("open", "offen")
    }
    pub fn average(self) -> &'static str {
        self.pick("Average (ECTS-weighted)", "Schnitt (nach ECTS gewichtet)")
    }
    pub fn ects_earned(self) -> &'static str {
        self.pick("ECTS earned", "ECTS erreicht")
    }
    pub fn ects_target(self) -> &'static str {
        self.pick("needed:", "nötig:")
    }

    pub fn average_simple(self) -> &'static str {
        self.pick("Plain average", "Einfacher Schnitt")
    }
    pub fn graded_count(self, n: usize) -> String {
        match n {
            1 => self.pick("1 module graded", "1 Modul benotet").into(),
            n => self.fmt(format!("{n} modules graded"), format!("{n} Module benotet")),
        }
    }
    pub fn add_grade_module(self) -> &'static str {
        self.pick("Module", "Modul")
    }
    pub fn new_grade_module(self) -> &'static str {
        self.pick("New module", "Neues Modul")
    }
    pub fn tests(self, n: usize) -> String {
        self.fmt(format!("Tests ({n})"), format!("Prüfungen ({n})"))
    }
    pub fn add_test(self) -> &'static str {
        self.pick("Test", "Prüfung")
    }
    pub fn test_name_hint(self) -> &'static str {
        self.pick("e.g. Midterm", "z. B. Zwischenprüfung")
    }
    pub fn weight_hint(self) -> &'static str {
        self.pick(
            "How much it counts: «1/6», «30 %», «0.25» or a relative weight like «2».",
            "Wie viel sie zählt: «1/6», «30 %», «0,25» oder ein relatives Gewicht wie «2».",
        )
    }
    pub fn provisional(self) -> &'static str {
        self.pick("provisional", "vorläufig")
    }
    pub fn grade_from_tests(self) -> &'static str {
        self.pick("from the tests", "aus den Prüfungen")
    }
    pub fn archived_tag(self) -> &'static str {
        self.pick("archived", "archiviert")
    }
    pub fn hide_from_grades(self) -> &'static str {
        self.pick(
            "Take out of the grades (the module stays for sorting)",
            "Aus den Noten nehmen (das Modul bleibt zum Sortieren)",
        )
    }
    pub fn delete_grade_module(self) -> &'static str {
        self.pick("Delete", "Löschen")
    }
    pub fn show_hidden_grades(self, n: usize) -> String {
        self.fmt(format!("Show taken out ({n})"), format!("Herausgenommene zeigen ({n})"))
    }
    pub fn show_again(self) -> &'static str {
        self.pick("Show again", "Wieder zeigen")
    }

    // ---------- notes ----------
    pub fn notes_title(self) -> &'static str {
        self.pick("Notes", "Notizen")
    }
    pub fn notes_hint(self) -> &'static str {
        self.pick(
            "Anything about this module: exam rules, what to repeat, contacts … Saved by itself and found by the search.",
            "Alles zu diesem Modul: Prüfungsregeln, was zu wiederholen ist, Kontakte … Wird von selbst gespeichert und von der Suche gefunden.",
        )
    }
    pub fn note_hit(self, module: &str) -> String {
        self.fmt(format!("Note · {module}"), format!("Notiz · {module}"))
    }
    pub fn search_hint(self) -> &'static str {
        self.pick("Search in files (Ctrl+F)", "In Dateien suchen (Ctrl+F)")
    }
    pub fn search_results(self, count: usize) -> String {
        match count {
            0 => self.pick("No results", "Keine Treffer").into(),
            1 => self.pick("1 file", "1 Datei").into(),
            n => self.fmt(format!("{n} files"), format!("{n} Dateien")),
        }
    }
    pub fn indexing(self, files: usize) -> String {
        self.fmt(
            format!("Reading the study folder… {files} files so far"),
            format!("Studienordner wird gelesen… bisher {files} Dateien"),
        )
    }
    pub fn indexed(self, files: usize) -> String {
        self.fmt(format!("{files} files searched"), format!("{files} Dateien durchsucht"))
    }
    pub fn all_modules(self) -> &'static str {
        self.pick("All modules", "Alle Module")
    }
    pub fn kind_names(self) -> [&'static str; 4] {
        match self {
            Lang::En => ["Lectures", "Exercises", "Solutions", "Exams"],
            Lang::De => ["Vorlesung", "Übung", "Lösung", "Prüfung"],
        }
    }
    pub fn any_type(self) -> &'static str {
        self.pick("All types", "Alle Typen")
    }
    pub fn slides(self) -> &'static str {
        self.pick("Slides", "Folien")
    }
    pub fn text_files(self) -> &'static str {
        self.pick("Text & notes", "Text & Notizen")
    }
    pub fn any_time(self) -> &'static str {
        self.pick("Any time", "Jederzeit")
    }
    pub fn last_week(self) -> &'static str {
        self.pick("Last 7 days", "Letzte 7 Tage")
    }
    pub fn last_month(self) -> &'static str {
        self.pick("Last month", "Letzter Monat")
    }
    pub fn this_semester(self) -> &'static str {
        self.pick("This semester", "Dieses Semester")
    }
    pub fn reset_filters(self) -> &'static str {
        self.pick("Reset", "Zurücksetzen")
    }
    pub fn search_tips(self) -> &'static str {
        self.pick(
            "Tip: the start of a word is enough · \"exact phrase\" · -word leaves out · ↑↓ and Enter open, Ctrl+Enter shows the folder",
            "Tipp: Wortanfang genügt · \"genaue Phrase\" · -wort schliesst aus · ↑↓ und Enter öffnen, Ctrl+Enter zeigt den Ordner",
        )
    }
    pub fn also_on_pages(self, pages: &[usize]) -> String {
        let list = pages.iter().map(|p| p.to_string()).collect::<Vec<_>>().join(", ");
        self.fmt(format!("also on page {list}"), format!("auch auf Seite {list}"))
    }
    pub fn recognised_hit(self) -> &'static str {
        self.pick("recognised text (scan)", "erkannter Text (Scan)")
    }
    pub fn no_results(self) -> &'static str {
        self.pick("No file contains all of these words.", "Keine Datei enthält alle diese Wörter.")
    }
    pub fn page(self, page: usize) -> String {
        self.fmt(format!("page {page}"), format!("Seite {page}"))
    }
    pub fn show_in_folder(self) -> &'static str {
        self.pick("Show in folder", "Im Ordner zeigen")
    }
    pub fn open_file(self) -> &'static str {
        self.pick("Open file", "Datei öffnen")
    }
    pub fn add_deadline(self) -> &'static str {
        self.pick("Deadline", "Frist")
    }
    pub fn no_deadlines(self) -> &'static str {
        self.pick(
            "Nothing open. Deadlines and exam dates in the documents of the study folder are found by themselves — \
             or check a file, or add one with «+ Deadline».",
            "Nichts offen. Abgabe- und Prüfungstermine in den Dokumenten des Studienordners werden von selbst \
             erkannt – oder eine Datei durchsuchen bzw. mit «+ Frist» von Hand eintragen.",
        )
    }
    pub fn deadlines_title(self) -> &'static str {
        self.pick("Deadlines & exams", "Abgaben & Prüfungen")
    }
    pub fn check_file(self) -> &'static str {
        self.pick("Check a file…", "Datei durchsuchen…")
    }
    pub fn check_file_hint(self) -> &'static str {
        self.pick(
            "Choose one document and see right away which deadlines are found in it",
            "Ein Dokument wählen und sofort sehen, welche Termine darin erkannt werden",
        )
    }
    pub fn checked_found(self, name: &str, found: usize, added: usize) -> String {
        let new = match added {
            0 => self.pick("already in the list", "bereits in der Liste").to_string(),
            n => self.fmt(format!("{n} new"), format!("{n} neu")),
        };
        match found {
            1 => self.fmt(format!("«{name}»: 1 date found ({new})."), format!("«{name}»: 1 Termin gefunden ({new}).")),
            n => self
                .fmt(format!("«{name}»: {n} dates found ({new})."), format!("«{name}»: {n} Termine gefunden ({new}).")),
        }
    }
    pub fn checked_nothing(self, name: &str) -> String {
        self.fmt(
            format!("No deadline or exam date found in «{name}». If there is one, add it by hand."),
            format!("In «{name}» wurde kein Abgabe- oder Prüfungstermin erkannt. Falls doch einer drinsteht: von Hand eintragen."),
        )
    }
    pub fn add_by_hand(self) -> &'static str {
        self.pick("Add by hand", "Von Hand eintragen")
    }
    pub fn scanned_status(self, documents: usize, scanning: bool) -> String {
        let base =
            self.fmt(format!("{documents} documents looked through"), format!("{documents} Dokumente durchsucht"));
        if scanning { format!("{base} · {}", self.pick("reading…", "wird gelesen …")) } else { base }
    }
    pub fn overdue_group(self, n: usize) -> String {
        self.fmt(format!("Overdue ({n})"), format!("Überfällig ({n})"))
    }
    pub fn this_week_group(self, n: usize) -> String {
        self.fmt(format!("This week ({n})"), format!("Diese Woche ({n})"))
    }
    pub fn next_week_group(self, n: usize) -> String {
        self.fmt(format!("Next week ({n})"), format!("Nächste Woche ({n})"))
    }
    pub fn later_group(self, n: usize) -> String {
        self.fmt(format!("Later ({n})"), format!("Später ({n})"))
    }
    pub fn done_group(self, n: usize) -> String {
        self.fmt(format!("Done ({n})"), format!("Erledigt ({n})"))
    }
    pub fn reopen(self) -> &'static str {
        self.pick("Reopen", "Wieder öffnen")
    }
    pub fn badge_new(self) -> &'static str {
        self.pick("new", "neu")
    }
    pub fn repeats(self) -> &'static str {
        self.pick("Repeats", "Wiederholt sich")
    }
    pub fn repeat_never(self) -> &'static str {
        self.pick("Once", "Einmalig")
    }
    pub fn repeat_every(self, every: crate::deadlines::Every) -> &'static str {
        use crate::deadlines::Every;
        match every {
            Every::Week => self.pick("Weekly", "Wöchentlich"),
            Every::TwoWeeks => self.pick("Every 2 weeks", "Alle 2 Wochen"),
            Every::Month => self.pick("Monthly", "Monatlich"),
        }
    }
    pub fn repeat_until(self) -> &'static str {
        self.pick("until", "bis")
    }
    pub fn repeat_until_hint(self) -> &'static str {
        self.pick("e.g. 20.12. (optional)", "z. B. 20.12. (freiwillig)")
    }
    pub fn repeat_until_day(self, day: &str) -> String {
        self.fmt(format!("until {day}"), format!("bis {day}"))
    }
    pub fn estimated(self) -> &'static str {
        self.pick("date estimated", "Datum geschätzt")
    }
    pub fn estimated_hint(self) -> &'static str {
        self.pick(
            "The text only names the weekday; this is the next one after the file came. Please check.",
            "Im Text steht nur der Wochentag; das ist der nächste nach dem Eingang der Datei. Bitte prüfen.",
        )
    }
    pub fn due_when(self, days: i64, late: bool, time: Option<&str>) -> String {
        let at = time.map(|t| self.fmt(format!(", {t}"), format!(", {t} Uhr"))).unwrap_or_default();
        match (late, days) {
            (true, 0) => self.fmt(format!("was due today{at}"), format!("war heute fällig{at}")),
            (true, d) => self.fmt(format!("overdue by {} days", -d), format!("seit {} Tagen überfällig", -d)),
            (false, 0) => self.fmt(format!("due today{at}"), format!("heute fällig{at}")),
            (false, 1) => self.fmt(format!("due tomorrow{at}"), format!("morgen fällig{at}")),
            (false, d) => self.fmt(format!("in {d} days{at}"), format!("in {d} Tagen{at}")),
        }
    }
    pub fn deadline_kind(self, kind: crate::deadlines::Kind) -> &'static str {
        match kind {
            crate::deadlines::Kind::Due => self.pick("Hand-in", "Abgabe"),
            crate::deadlines::Kind::Exam => self.pick("Exam / date", "Prüfung / Termin"),
        }
    }
    pub fn save(self) -> &'static str {
        self.pick("Save", "Speichern")
    }
    pub fn correct(self) -> &'static str {
        self.pick("Correct", "Korrigieren")
    }
    pub fn search_again(self) -> &'static str {
        self.pick("Look again", "Erneut suchen")
    }
    pub fn search_again_hint(self) -> &'static str {
        self.pick(
            "Look through all documents of the last months for deadlines and exam dates again",
            "Alle Dokumente der letzten Monate nochmals nach Abgaben und Prüfungsterminen durchsuchen",
        )
    }
    pub fn to_calendar(self) -> &'static str {
        self.pick("Add to calendar", "In den Kalender")
    }
    pub fn done(self) -> &'static str {
        self.pick("Done", "Erledigt")
    }
    pub fn deadlines_note(self) -> &'static str {
        self.pick(
            "Found automatically in the text — please check the date. Wrong ones can be deleted.",
            "Automatisch im Text erkannt – bitte Datum prüfen. Falsche lassen sich löschen.",
        )
    }
    pub fn deadline_summary(self, title: &str, module: &str) -> String {
        match module {
            "" => self.fmt(format!("Due: {title}"), format!("Abgabe: {title}")),
            m => self.fmt(format!("Due: {title} ({m})"), format!("Abgabe: {title} ({m})")),
        }
    }
    pub fn reminder_tomorrow(self) -> &'static str {
        self.pick("Due tomorrow", "Morgen fällig")
    }
    pub fn what_is_due(self) -> &'static str {
        self.pick("What is due", "Was ist fällig")
    }
    pub fn what_is_due_hint(self) -> &'static str {
        self.pick("e.g. Exercise sheet 3", "z. B. Übungsblatt 3")
    }
    pub fn module_label(self) -> &'static str {
        self.pick("Module", "Modul")
    }
    pub fn no_module(self) -> &'static str {
        self.pick("No module", "Kein Modul")
    }
    pub fn due_date(self) -> &'static str {
        self.pick("When (date, optionally time)", "Wann (Datum, optional Uhrzeit)")
    }
    pub fn date_not_understood(self) -> &'static str {
        self.pick("Date not understood, e.g. 24.10. 23:59", "Datum nicht verstanden, z. B. 24.10. 23:59")
    }
    pub fn new_this_week(self, files: usize) -> String {
        self.fmt(format!("New this week ({files})"), format!("Neu diese Woche ({files})"))
    }
    pub fn nothing_new_this_week(self) -> &'static str {
        self.pick("No files were sorted in the last 7 days.", "In den letzten 7 Tagen wurden keine Dateien sortiert.")
    }

    // ---------- semester archive ----------
    pub fn semester(self) -> &'static str {
        self.pick("Semester", "Semester")
    }
    pub fn finish_semester(self) -> &'static str {
        self.pick("Finish semester", "Semester abschliessen")
    }
    pub fn finish_semester_hint(self) -> &'static str {
        self.pick(
            "Move finished modules with their folders into «Archiv», so only the current ones remain.",
            "Abgeschlossene Module samt Ordner ins «Archiv» verschieben, damit nur die aktuellen bleiben.",
        )
    }
    pub fn finish_semester_button(self) -> &'static str {
        self.pick("Finish semester…", "Semester abschliessen…")
    }
    pub fn archive_explained(self) -> &'static str {
        self.pick(
            "The chosen modules leave the list, and their folders move to Archiv/<semester>/ in the study folder. Nothing is deleted, and each module can be brought back in the settings.",
            "Die gewählten Module verschwinden aus der Liste, ihre Ordner kommen nach Archiv/<Semester>/ im Studienordner. Es wird nichts gelöscht, und jedes Modul lässt sich in den Einstellungen wiederherstellen.",
        )
    }
    pub fn semester_name(self) -> &'static str {
        self.pick("Semester", "Semester")
    }
    pub fn modules_to_archive(self) -> &'static str {
        self.pick("Modules that are over", "Module, die vorbei sind")
    }
    pub fn archive_count(self, count: usize) -> String {
        match count {
            0 | 1 => self.pick("Archive", "Archivieren").into(),
            n => self.fmt(format!("Archive {n} modules"), format!("{n} Module archivieren")),
        }
    }
    pub fn archived_note(self, count: usize) -> String {
        match count {
            1 => self.pick("1 module archived", "1 Modul archiviert").into(),
            n => self.fmt(format!("{n} modules archived"), format!("{n} Module archiviert")),
        }
    }
    pub fn gdrive_title(self) -> &'static str {
        self.pick("Google Drive (without the Google app)", "Google Drive (ohne Google-App)")
    }
    pub fn gdrive_hint(self) -> &'static str {
        self.pick(
            "Keeps the study folder in sync with a folder in your Google Drive, both ways. Sign in once in the browser. Nothing is lost: deleted or overwritten files are kept in a backup for 30 days, and a file changed on both sides is kept twice.",
            "Hält den Studienordner mit einem Ordner in deinem Google Drive synchron, in beide Richtungen. Einmal im Browser anmelden. Es geht nichts verloren: Gelöschte oder überschriebene Dateien bleiben 30 Tage in einer Sicherung, und eine auf beiden Seiten geänderte Datei wird doppelt behalten.",
        )
    }
    pub fn gdrive_connect(self) -> &'static str {
        self.pick("Connect Google Drive", "Mit Google Drive verbinden")
    }
    pub fn gdrive_installing(self) -> &'static str {
        self.pick("Fetching the sync tool (rclone)…", "Synchronisier-Werkzeug (rclone) wird geladen…")
    }
    pub fn gdrive_signing_in(self) -> &'static str {
        self.pick(
            "Sign in with Google in the browser and allow access…",
            "Melde dich im Browser bei Google an und erlaube den Zugriff…",
        )
    }
    pub fn gdrive_open_sign_in(self) -> &'static str {
        self.pick("Open sign-in page", "Anmeldeseite öffnen")
    }
    pub fn gdrive_folder(self) -> &'static str {
        self.pick("Folder in Google Drive", "Ordner in Google Drive")
    }
    pub fn gdrive_sync_on(self) -> &'static str {
        self.pick("Sync automatically", "Automatisch synchronisieren")
    }
    pub fn gdrive_sync_on_hint(self) -> &'static str {
        self.pick(
            "Right after sorting, every 15 minutes and at the start.",
            "Gleich nach dem Sortieren, alle 15 Minuten und beim Start.",
        )
    }
    pub fn gdrive_sync_now(self) -> &'static str {
        self.pick("Sync now", "Jetzt synchronisieren")
    }
    pub fn gdrive_syncing(self) -> &'static str {
        self.pick("Syncing…", "Wird synchronisiert…")
    }
    pub fn gdrive_backups(self) -> &'static str {
        self.pick("Backups", "Sicherungen")
    }
    pub fn gdrive_disconnect(self) -> &'static str {
        self.pick("Disconnect", "Trennen")
    }
    pub fn gdrive_connected(self) -> &'static str {
        self.pick("Connected with Google Drive", "Mit Google Drive verbunden")
    }
    pub fn gdrive_never(self) -> &'static str {
        self.pick("Not synced yet", "Noch nicht synchronisiert")
    }
    pub fn gdrive_last(self, time: &str, down: usize, up: usize, conflicts: usize) -> String {
        let mut text = self.fmt(format!("Synced at {time}"), format!("Synchronisiert um {time}"));
        if down + up > 0 {
            text.push_str(
                &self.fmt(format!(" · {down} down, {up} up"), format!(" · {down} heruntergeladen, {up} hochgeladen")),
            );
        }
        if conflicts > 0 {
            text.push_str(&self.fmt(
                format!(" · {conflicts} changed on both sides (both kept)"),
                format!(" · {conflicts} auf beiden Seiten geändert (beide behalten)"),
            ));
        }
        text
    }
    pub fn gdrive_failed(self, why: &str) -> String {
        let why = match why {
            "too many deletes" => self.pick(
                "more than half of the files would be deleted – not synced, to be safe. Check the study folder, then «Sync now».",
                "mehr als die Hälfte der Dateien würde gelöscht – zur Sicherheit nicht synchronisiert. Prüfe den Studienordner, dann «Jetzt synchronisieren».",
            ),
            "sign-in expired" => self.pick("the Google sign-in expired – please connect again.", "die Google-Anmeldung ist abgelaufen – bitte neu verbinden."),
            other => return self.fmt(format!("Google Drive: not synced ({other})"), format!("Google Drive: nicht synchronisiert ({other})")),
        };
        self.fmt(format!("Google Drive: {why}"), format!("Google Drive: {why}"))
    }
    pub fn gdrive_synced_note(self, down: usize, up: usize) -> String {
        self.fmt(
            format!("Google Drive: {down} file(s) down, {up} up"),
            format!("Google Drive: {down} Datei(en) heruntergeladen, {up} hochgeladen"),
        )
    }
    pub fn gdrive_conflict_note(self, count: usize) -> String {
        self.fmt(
            format!("Google Drive: {count} file(s) changed on both sides – both versions kept («….conflict1»)"),
            format!(
                "Google Drive: {count} Datei(en) auf beiden Seiten geändert – beide Versionen behalten («….conflict1»)"
            ),
        )
    }
    pub fn gdrive_app_folder(self) -> &'static str {
        self.pick(
            "The study folder is in the folder of the Google Drive app already, which syncs it.",
            "Der Studienordner liegt schon im Ordner der Google-Drive-App, die ihn synchronisiert.",
        )
    }
    pub fn auto_results_window(self) -> &'static str {
        self.pick("Results window for automatic sorting", "Ergebnisfenster beim automatischen Sortieren")
    }
    pub fn auto_results_window_hint(self) -> &'static str {
        self.pick(
            "When files are sorted while File Sorter is minimised, in the notification area or behind other windows: show them bottom right, with «Undo».",
            "Wenn Dateien sortiert werden, während File Sorter minimiert, im Infobereich oder hinter anderen Fenstern ist: unten rechts anzeigen, mit «Rückgängig».",
        )
    }
    pub fn flat_modules(self) -> &'static str {
        self.pick("Modules directly in the study folder", "Module direkt im Studienordner")
    }
    pub fn flat_modules_hint(self) -> &'static str {
        self.pick(
            "Studies/Statistics instead of Studies/Computer Science/Statistics – no folder per study program (major, minor).",
            "Studium/Statistik statt Studium/Informatik/Statistik – kein Ordner pro Studiengang (Haupt-, Nebenfach).",
        )
    }
    pub fn layout_label(self) -> &'static str {
        self.pick("How is your study folder organised?", "Wie ist dein Studienordner aufgebaut?")
    }
    pub fn layout_programs(self) -> &'static str {
        self.pick("Program › Module", "Studiengang › Modul")
    }
    pub fn layout_flat(self) -> &'static str {
        self.pick("Modules directly", "Module direkt")
    }
    pub fn flatten_title(self) -> &'static str {
        self.pick("Move the module folders up?", "Modulordner nach oben verschieben?")
    }
    pub fn flatten_text(self, count: usize) -> String {
        match count {
            1 => self
                .pick(
                    "1 module is in a study program's folder. Move it directly into the study folder? Nothing is overwritten, and «Undo» puts it back.",
                    "1 Modul liegt im Ordner eines Studiengangs. Direkt in den Studienordner verschieben? Es wird nichts überschrieben, und «Rückgängig» stellt es zurück.",
                )
                .into(),
            n => self.fmt(
                format!("{n} modules are in study programs' folders. Move them directly into the study folder? Nothing is overwritten, and «Undo» puts them back."),
                format!("{n} Module liegen in Ordnern von Studiengängen. Direkt in den Studienordner verschieben? Es wird nichts überschrieben, und «Rückgängig» stellt sie zurück."),
            ),
        }
    }
    pub fn flatten_kept(self) -> &'static str {
        self.pick("These stay where they are:", "Diese bleiben, wo sie sind:")
    }
    pub fn flatten_own_folder(self) -> &'static str {
        self.pick("its folder was chosen by hand", "eigener Ordner gewählt")
    }
    pub fn flatten_taken(self, path: &str) -> String {
        self.fmt(format!("{path} exists already"), format!("{path} gibt es schon"))
    }
    pub fn flatten_move(self, count: usize) -> String {
        match count {
            1 => self.pick("Move 1 module", "1 Modul verschieben").into(),
            n => self.fmt(format!("Move {n} modules"), format!("{n} Module verschieben")),
        }
    }
    pub fn flatten_skip(self) -> &'static str {
        self.pick("Don't move", "Nicht verschieben")
    }
    pub fn flatten_done(self, count: usize) -> String {
        match count {
            1 => self
                .pick("1 module is now directly in the study folder", "1 Modul liegt jetzt direkt im Studienordner")
                .into(),
            n => self.fmt(
                format!("{n} modules are now directly in the study folder"),
                format!("{n} Module liegen jetzt direkt im Studienordner"),
            ),
        }
    }
    pub fn flatten_undone(self) -> &'static str {
        self.pick("Modules put back into their study programs", "Module zurück in ihre Studiengänge verschoben")
    }
    pub fn restore(self) -> &'static str {
        self.pick("Restore", "Wiederherstellen")
    }
    pub fn restored_note(self, module: &str) -> String {
        self.fmt(format!("{module} is back"), format!("{module} ist zurück"))
    }
    pub fn restore_failed(self, module: &str) -> String {
        self.fmt(format!("{module} could not be restored"), format!("{module} konnte nicht wiederhergestellt werden"))
    }
    pub fn stayed_behind(self, file: &str, why: &str) -> String {
        self.fmt(format!("Stayed: {file} ({why})"), format!("Geblieben: {file} ({why})"))
    }

    // ---------- updates ----------
    pub fn update_available(self, version: &str) -> String {
        self.fmt(format!("File Sorter {version} is available."), format!("File Sorter {version} ist da."))
    }
    pub fn update_installing(self) -> String {
        self.pick("Downloading and checking the new version…", "Neue Version wird heruntergeladen und geprüft…").into()
    }
    pub fn update_installed(self, version: &str) -> String {
        self.fmt(
            format!("File Sorter {version} is installed and runs from the next start."),
            format!("File Sorter {version} ist installiert und läuft ab dem nächsten Start."),
        )
    }
    pub fn update_failed(self, error: &str) -> String {
        self.fmt(
            format!("The update didn't work ({error}). The current version keeps running."),
            format!("Die Aktualisierung hat nicht geklappt ({error}). Die jetzige Version läuft weiter."),
        )
    }
    pub fn update_now(self) -> &'static str {
        self.pick("Update now", "Jetzt aktualisieren")
    }
    pub fn later(self) -> &'static str {
        self.pick("Later", "Später")
    }
    pub fn whats_new(self) -> &'static str {
        self.pick("What's new", "Was ist neu")
    }
    pub fn restart_now(self) -> &'static str {
        self.pick("Restart now", "Jetzt neu starten")
    }

    // ---------- setup ----------
    pub fn setup_step(self, step: usize, of: usize) -> String {
        self.fmt(format!("Setup · step {step} of {of}"), format!("Einrichtung · Schritt {step} von {of}"))
    }
    pub fn setup_welcome(self) -> &'static str {
        self.pick("Welcome to File Sorter", "Willkommen bei File Sorter")
    }
    pub fn setup_downloads_text(self) -> &'static str {
        self.pick(
            "File Sorter watches the folder your browser saves downloads to and puts each course file into its module's folder. Which folder is it?",
            "File Sorter beobachtet den Ordner, in den dein Browser Downloads speichert, und legt jede Kursdatei in den Ordner ihres Moduls. Welcher Ordner ist es?",
        )
    }
    pub fn setup_study_title(self) -> &'static str {
        self.pick("Your study folder", "Dein Studienordner")
    }
    pub fn setup_study_text(self) -> &'static str {
        self.pick(
            "Here each study program gets a folder and in it one folder per module (e.g. Studium/Informatik/Programmieren).",
            "Hier bekommt jeder Studiengang einen Ordner und darin jedes Modul einen eigenen (z. B. Studium/Informatik/Programmieren).",
        )
    }
    pub fn setup_in_cloud(self) -> &'static str {
        self.pick(
            "Or in a cloud folder, to have it on every device:",
            "Oder in einem Cloud-Ordner, um ihn auf allen Geräten zu haben:",
        )
    }
    pub fn setup_modules_title(self) -> &'static str {
        self.pick("Your modules", "Deine Module")
    }
    pub fn setup_modules_text(self) -> &'static str {
        self.pick(
            "If the study folder already has a folder per module, File Sorter can read them in. Otherwise add modules in the overview with «+».",
            "Hat der Studienordner schon einen Ordner pro Modul, kann File Sorter sie einlesen. Sonst fügst du Module in der Übersicht mit «+» hinzu.",
        )
    }
    pub fn setup_found(self, count: usize) -> String {
        match count {
            0 => self.pick("No module folders found.", "Keine Modulordner gefunden.").into(),
            n => self.fmt(format!("{n} module folders found."), format!("{n} Modulordner gefunden.")),
        }
    }
    pub fn setup_modules_count(self, count: usize) -> String {
        self.fmt(format!("Modules now: {count}"), format!("Module jetzt: {count}"))
    }
    pub fn setup_start_watching(self) -> &'static str {
        self.pick("Start watching the downloads folder now", "Download-Ordner jetzt überwachen")
    }
    pub fn setup_finish(self) -> &'static str {
        self.pick("Done", "Fertig")
    }
    pub fn setup_skip(self) -> &'static str {
        self.pick("Skip", "Überspringen")
    }
    pub fn setup_again(self) -> &'static str {
        self.pick("Run setup", "Einrichtung starten")
    }
    pub fn next(self) -> &'static str {
        self.pick("Next", "Weiter")
    }
    pub fn back(self) -> &'static str {
        self.pick("Back", "Zurück")
    }
    pub fn folder_missing(self) -> &'static str {
        self.pick("This folder doesn't exist.", "Diesen Ordner gibt es nicht.")
    }

    // ---------- tabs and settings ----------
    pub fn tab_overview(self) -> &'static str {
        self.pick("Overview", "Übersicht")
    }
    pub fn tab_settings(self) -> &'static str {
        self.pick("Settings", "Einstellungen")
    }
    pub fn automatic(self) -> &'static str {
        self.pick("Automatic", "Automatisch")
    }
    pub fn saved(self) -> &'static str {
        self.pick("Saved", "Gespeichert")
    }

    pub fn general(self) -> &'static str {
        self.pick("General", "Allgemein")
    }
    pub fn language_hint(self) -> &'static str {
        self.pick("Automatic follows the language of your computer.", "Automatisch folgt der Sprache deines Computers.")
    }
    pub fn appearance(self) -> &'static str {
        self.pick("Appearance", "Erscheinungsbild")
    }
    pub fn appearance_hint(self) -> &'static str {
        self.pick("Light, dark, or like your computer.", "Hell, dunkel oder wie dein Computer.")
    }
    pub fn light(self) -> &'static str {
        self.pick("Light", "Hell")
    }
    pub fn dark(self) -> &'static str {
        self.pick("Dark", "Dunkel")
    }
    pub fn animations(self) -> &'static str {
        self.pick("Animations", "Animationen")
    }
    pub fn animations_hint(self) -> &'static str {
        self.pick(
            "Flying files, sliding cards and the pulsing status dot.",
            "Fliegende Dateien, einblendende Karten und der pulsierende Status.",
        )
    }

    pub fn sorting(self) -> &'static str {
        self.pick("Sorting", "Sortieren")
    }
    pub fn file_action(self) -> &'static str {
        self.pick("What happens to a file", "Was mit einer Datei passiert")
    }
    pub fn action_move(self) -> &'static str {
        self.pick("Move", "Verschieben")
    }
    pub fn action_move_hint(self) -> &'static str {
        self.pick(
            "The file ends up only in its module folder; Downloads stays tidy. (Recommended)",
            "Die Datei liegt danach nur noch im Modulordner, der Download-Ordner bleibt aufgeräumt. (Empfohlen)",
        )
    }
    pub fn action_copy(self) -> &'static str {
        self.pick("Copy", "Kopieren")
    }
    pub fn action_copy_hint(self) -> &'static str {
        self.pick(
            "A copy goes to the module folder, the original stays where it was. Each file is copied only once.",
            "Eine Kopie kommt in den Modulordner, das Original bleibt, wo es war. Jede Datei wird nur einmal kopiert.",
        )
    }
    pub fn read_contents(self) -> &'static str {
        self.pick("Read file contents", "Dateiinhalt lesen")
    }
    pub fn read_contents_hint(self) -> &'static str {
        self.pick(
            "If the file name isn't enough, read the text of PDFs, slides and Word files.",
            "Wenn der Dateiname nicht reicht, den Text von PDFs, Folien und Word-Dateien lesen.",
        )
    }
    pub fn sort_existing_on_start(self) -> &'static str {
        self.pick("Include files already there", "Vorhandene Dateien mitsortieren")
    }
    // ---------- features ----------
    pub fn features(self) -> &'static str {
        self.pick("Features", "Funktionen")
    }
    pub fn feature_learn(self) -> &'static str {
        self.pick("Learn from assigning", "Aus Zuordnungen lernen")
    }
    pub fn feature_learn_hint(self) -> &'static str {
        self.pick(
            "When you assign a file to a module by hand, words from its name become keywords of that module.",
            "Ordnest du eine Datei von Hand einem Modul zu, werden Wörter aus ihrem Namen zu Stichwörtern des Moduls.",
        )
    }
    pub fn feature_origin(self) -> &'static str {
        self.pick("Download source", "Download-Herkunft")
    }
    pub fn feature_origin_hint(self) -> &'static str {
        self.pick(
            "Remember which Moodle, ILIAS or OLAT course a file came from and sort the next one from there the same way.",
            "Merken, aus welchem Moodle-, ILIAS- oder OLAT-Kurs eine Datei kam, und weitere von dort gleich einsortieren.",
        )
    }
    pub fn feature_classifier(self) -> &'static str {
        self.pick("Learn the modules from their files", "Module aus ihren Dateien lernen")
    }
    pub fn feature_classifier_hint(self) -> &'static str {
        self.pick(
            "When the keywords don't decide, compare a file with the files already in each module and sort it only if one is clearly more alike.",
            "Wenn die Stichwörter nicht entscheiden, eine Datei mit den Dateien vergleichen, die schon in den Modulen liegen, und nur einsortieren, wenn eines klar ähnlicher ist.",
        )
    }
    pub fn feature_ask_unsure(self) -> &'static str {
        self.pick("Ask when unsure", "Bei Unsicherheit nachfragen")
    }
    pub fn feature_ask_unsure_hint(self) -> &'static str {
        self.pick(
            "When two modules fit about equally well, the file stays and Activity asks which one is right.",
            "Passen zwei Module etwa gleich gut, bleibt die Datei liegen und die Aktivität fragt, welches stimmt.",
        )
    }
    pub fn feature_ocr(self) -> &'static str {
        self.pick("Recognise text in scans", "Texterkennung für Scans")
    }
    pub fn feature_ocr_hint(self) -> &'static str {
        if cfg!(windows) {
            self.pick(
                "Scanned PDFs and photos of pages are read with the text recognition built into Windows, so they can be sorted, searched and their deadlines found.",
                "Eingescannte PDFs und Fotos von Seiten werden mit der in Windows eingebauten Texterkennung gelesen – so werden sie sortiert, gefunden und ihre Termine erkannt.",
            )
        } else {
            self.pick(
                "Scanned PDFs and photos of pages are read with tesseract, so they can be sorted, searched and their deadlines found.",
                "Eingescannte PDFs und Fotos von Seiten werden mit tesseract gelesen – so werden sie sortiert, gefunden und ihre Termine erkannt.",
            )
        }
    }
    pub fn feature_ocr_missing(self) -> &'static str {
        self.pick(
            "Not installed: sudo apt install tesseract-ocr tesseract-ocr-deu poppler-utils (then restart File Sorter).",
            "Nicht installiert: sudo apt install tesseract-ocr tesseract-ocr-deu poppler-utils (danach File Sorter neu starten).",
        )
    }
    pub fn feature_reminders(self) -> &'static str {
        self.pick("Reminders", "Erinnerungen")
    }
    pub fn feature_reminders_hint(self) -> &'static str {
        self.pick(
            "A notification before each deadline and exam – also while File Sorter runs in the background.",
            "Eine Benachrichtigung vor jeder Abgabe und Prüfung – auch wenn File Sorter im Hintergrund läuft.",
        )
    }
    pub fn lead(self, lead: crate::reminders::Lead) -> &'static str {
        use crate::reminders::Lead;
        match lead {
            Lead::Day => self.pick("1 day before", "1 Tag vorher"),
            Lead::Morning => self.pick("Morning of the day (8:00)", "Am Morgen des Tages (8:00)"),
            Lead::Hours3 => self.pick("3 hours before", "3 Stunden vorher"),
            Lead::Hour => self.pick("1 hour before", "1 Stunde vorher"),
        }
    }
    pub fn feature_timetable_hint(self) -> &'static str {
        self.pick("Use the timetable for ties", "Stundenplan bei Gleichstand beachten")
    }
    pub fn feature_subfolders(self) -> &'static str {
        self.pick("Subfolders", "Unterordner")
    }
    pub fn feature_subfolders_hint(self) -> &'static str {
        self.pick(
            "Put lecture slides, exercises, solutions and exams into their own folder inside the module.",
            "Vorlesungsfolien, Übungen, Lösungen und Prüfungen im Modul in eigene Ordner legen.",
        )
    }
    pub fn subfolder_list_hint(self) -> &'static str {
        self.pick(
            "Every module gets these folders. A file goes into the one whose words are in its name (or at the start of the text). Folders you make inside a module yourself are used too when their name is in the file name.",
            "Jedes Modul bekommt diese Ordner. Eine Datei kommt in den, dessen Wörter in ihrem Namen (oder am Anfang des Textes) stehen. Ordner, die du selbst in einem Modul anlegst, werden auch genutzt, wenn ihr Name im Dateinamen vorkommt.",
        )
    }
    pub fn subfolder_name_hint(self) -> &'static str {
        self.pick("Folder name", "Ordnername")
    }
    pub fn subfolder_words_hint(self) -> &'static str {
        self.pick("Words, separated by commas", "Wörter, mit Komma getrennt")
    }
    pub fn subfolder_builtin_words(self, kind: crate::categories::Category) -> &'static str {
        use crate::categories::Category;
        match kind {
            Category::Lecture => self.pick(
                "knows VL, slides, script, chapter … – more words?",
                "kennt VL, Folien, Skript, Kapitel … – weitere Wörter?",
            ),
            Category::Exercise => self.pick(
                "knows exercise, sheet, assignment … – more words?",
                "kennt Übung, Blatt, Serie, Aufgabe … – weitere Wörter?",
            ),
            Category::Solution => self.pick(
                "knows solution, Lösung, Musterlösung … – more words?",
                "kennt Lösung, Musterlösung, Solution … – weitere Wörter?",
            ),
            Category::Exam => self.pick(
                "knows exam, Klausur, midterm … – more words?",
                "kennt Klausur, Prüfung, Exam … – weitere Wörter?",
            ),
        }
    }
    pub fn add_subfolder(self) -> &'static str {
        self.pick("Subfolder", "Unterordner")
    }
    pub fn create_in_all_modules(self) -> &'static str {
        self.pick("Create in all modules", "In allen Modulen anlegen")
    }
    pub fn create_subfolders(self) -> &'static str {
        self.pick("Create subfolders", "Unterordner anlegen")
    }
    pub fn resort_existing(self) -> &'static str {
        self.pick("Sort existing files into subfolders", "Vorhandene Dateien einsortieren")
    }
    pub fn resort_existing_hint(self) -> &'static str {
        self.pick(
            "Move the files lying directly in the module folders into their subfolders (you see a preview first, and can undo it)",
            "Die Dateien, die direkt in den Modulordnern liegen, in ihre Unterordner verschieben (zuerst mit Vorschau, rückgängig machbar)",
        )
    }
    pub fn folders_created(self, count: usize) -> String {
        match count {
            0 => self.pick("All subfolders were already there", "Alle Unterordner waren schon da").into(),
            1 => self.pick("1 folder created", "1 Ordner angelegt").into(),
            n => self.fmt(format!("{n} folders created"), format!("{n} Ordner angelegt")),
        }
    }
    pub fn nothing_to_resort(self) -> &'static str {
        self.pick(
            "No file in the module folders needs to go into a subfolder",
            "Keine Datei in den Modulordnern gehört in einen Unterordner",
        )
    }
    pub fn resort_explained(self, count: usize) -> String {
        self.fmt(
            format!("{count} files go into subfolders. Nothing is overwritten (a name that is taken gets «(1)»), files open in another program stay, and «Undo» puts everything back."),
            format!("{count} Dateien kommen in Unterordner. Nichts wird überschrieben (ein besetzter Name bekommt «(1)»), geöffnete Dateien bleiben, und «Rückgängig» legt alles zurück."),
        )
    }
    pub fn file_count(self, count: usize) -> String {
        match count {
            1 => self.pick("1 file", "1 Datei").into(),
            n => self.fmt(format!("{n} files"), format!("{n} Dateien")),
        }
    }
    pub fn resort_start(self) -> &'static str {
        self.pick("Sort in", "Einsortieren")
    }
    pub fn resorted(self, count: usize) -> String {
        match count {
            1 => self.pick("1 file moved into its subfolder", "1 Datei in ihren Unterordner verschoben").into(),
            n => self.fmt(format!("{n} files moved into subfolders"), format!("{n} Dateien in Unterordner verschoben")),
        }
    }
    pub fn resort_undone(self) -> &'static str {
        self.pick("Files put back into the module folders", "Dateien in die Modulordner zurückgelegt")
    }
    pub fn resort_open(self, count: usize) -> String {
        self.fmt(
            format!("{count} open in another program – they stay where they are"),
            format!("{count} in einem anderen Programm geöffnet – sie bleiben, wo sie sind"),
        )
    }
    pub fn resort_not_back(self, count: usize) -> String {
        self.fmt(
            format!("{count} files could not be put back"),
            format!("{count} Dateien konnten nicht zurückgelegt werden"),
        )
    }
    pub fn subfolders_off(self) -> &'static str {
        self.pick(
            "Subfolders are off: files go straight into the module folder.",
            "Unterordner sind aus: Dateien kommen direkt in den Modulordner.",
        )
    }
    pub fn use_subfolders(self) -> &'static str {
        self.pick("Use subfolders", "Unterordner verwenden")
    }
    pub fn subfolder_not_created(self) -> &'static str {
        self.pick("not created yet", "noch nicht angelegt")
    }
    pub fn loose_files(self, count: usize) -> String {
        match count {
            0 => self
                .pick("No file lies directly in the module folder.", "Keine Datei liegt direkt im Modulordner.")
                .into(),
            1 => self.pick("1 file lies directly in the module folder.", "1 Datei liegt direkt im Modulordner.").into(),
            n => self.fmt(
                format!("{n} files lie directly in the module folder."),
                format!("{n} Dateien liegen direkt im Modulordner."),
            ),
        }
    }
    pub fn create_here(self) -> &'static str {
        self.pick("Create", "Anlegen")
    }
    pub fn create_here_hint(self) -> &'static str {
        self.pick(
            "Create the subfolders of the list in this module",
            "Die Unterordner der Liste in diesem Modul anlegen",
        )
    }
    pub fn resort_here(self) -> &'static str {
        self.pick("Sort in", "Einsortieren")
    }
    pub fn resort_here_hint(self) -> &'static str {
        self.pick(
            "Move the files lying directly in this module folder into their subfolders (preview first, can be undone)",
            "Die Dateien, die direkt in diesem Modulordner liegen, in ihre Unterordner verschieben (zuerst Vorschau, rückgängig machbar)",
        )
    }
    pub fn learn_folders(self) -> &'static str {
        self.pick("Learn from files", "Aus Dateien lernen")
    }
    pub fn learn_folders_hint(self) -> &'static str {
        self.pick(
            "Look at the files already in the subfolders of all modules and learn which words in a name mean which subfolder",
            "Die Dateien, die schon in den Unterordnern aller Module liegen, ansehen und lernen, welche Wörter im Namen zu welchem Unterordner gehören",
        )
    }
    pub fn edit_list(self) -> &'static str {
        self.pick("Edit list…", "Liste bearbeiten…")
    }
    pub fn learned_words(self) -> &'static str {
        self.pick("Learned:", "Gelernt:")
    }
    pub fn remove_word_hint(self) -> &'static str {
        self.pick("Remove – it isn't learned again", "Entfernen – wird nicht wieder gelernt")
    }
    pub fn folder_words_learned(self, words: usize, folders: usize) -> String {
        match (words, folders) {
            (0, _) => self.pick("No new subfolder words learned", "Keine neuen Wörter für Unterordner gelernt").into(),
            (1, _) => self.pick("1 word learned for a subfolder", "1 Wort für einen Unterordner gelernt").into(),
            (w, 1) => self
                .fmt(format!("{w} words learned for a subfolder"), format!("{w} Wörter für einen Unterordner gelernt")),
            (w, f) => self.fmt(
                format!("{w} words learned for {f} subfolders"),
                format!("{w} Wörter für {f} Unterordner gelernt"),
            ),
        }
    }
    pub fn folder_words_undone(self) -> &'static str {
        self.pick("Learned subfolder words taken back", "Gelernte Unterordner-Wörter zurückgenommen")
    }
    pub fn subfolder_list_title(self) -> &'static str {
        self.pick("Subfolders of all modules", "Unterordner aller Module")
    }
    pub fn feature_auto_learn(self) -> &'static str {
        self.pick("Learn from existing files by itself", "Automatisch aus vorhandenen Dateien lernen")
    }
    pub fn feature_auto_learn_hint(self) -> &'static str {
        self.pick(
            "Every 30 minutes and after sorting in, learn from the files in the subfolders which words belong where. Otherwise only on «Learn from files».",
            "Alle 30 Minuten und nach dem Einsortieren aus den Dateien in den Unterordnern lernen, welche Wörter wohin gehören. Sonst nur mit «Aus Dateien lernen».",
        )
    }
    pub fn subfolders_managed_in_overview(self) -> &'static str {
        self.pick(
            "Create, sort in and edit the list: Overview › select a module › «Subfolders».",
            "Anlegen, einsortieren und Liste bearbeiten: Übersicht › Modul wählen › «Unterordner».",
        )
    }
    pub fn open_this_folder(self) -> &'static str {
        self.pick("Open folder", "Ordner öffnen")
    }
    pub fn open_module_folder(self) -> &'static str {
        self.pick("Open module folder", "Modulordner öffnen")
    }
    pub fn not_chosen(self) -> &'static str {
        self.pick("not chosen", "nicht gewählt")
    }
    pub fn subfolder_defaults(self) -> [&'static str; 4] {
        match self {
            Lang::En => ["Lectures", "Exercises", "Solutions", "Exams"],
            Lang::De => ["Vorlesung", "Übungen", "Lösungen", "Prüfungen"],
        }
    }
    pub fn feature_unzip(self) -> &'static str {
        self.pick("Unpack ZIP files", "ZIP-Dateien entpacken")
    }
    pub fn feature_unzip_hint(self) -> &'static str {
        self.pick(
            "Sort ZIP files by their content and unpack them in the module folder (the ZIP is kept).",
            "ZIP-Dateien nach ihrem Inhalt einsortieren und im Modulordner entpacken (die ZIP-Datei bleibt).",
        )
    }
    pub fn feature_versions(self) -> &'static str {
        self.pick("Keep older versions apart", "Ältere Versionen ablegen")
    }
    pub fn feature_versions_hint(self) -> &'static str {
        self.pick(
            "When a new version of a file arrives (\"Script v2\"), move the old one into \"Older versions\".",
            "Kommt eine neue Version einer Datei («Skript v2»), die alte in «Ältere Versionen» verschieben.",
        )
    }
    pub fn feature_search(self) -> &'static str {
        self.pick("Search in files", "Suche in Dateien")
    }
    pub fn feature_search_hint(self) -> &'static str {
        self.pick(
            "Search the text of all PDFs, slides and documents in the study folder (Ctrl+F).",
            "Den Text aller PDFs, Folien und Dokumente im Studienordner durchsuchen (Ctrl+F).",
        )
    }
    pub fn feature_deadlines(self) -> &'static str {
        self.pick("Deadlines", "Abgabetermine")
    }
    pub fn feature_deadlines_hint(self) -> &'static str {
        self.pick(
            "Find due dates in exercise sheets and show what is due soon.",
            "Abgabetermine in Übungsblättern erkennen und anzeigen, was bald fällig ist.",
        )
    }
    pub fn feature_weekly(self) -> &'static str {
        self.pick("New this week", "Neu diese Woche")
    }
    pub fn feature_weekly_hint(self) -> &'static str {
        self.pick(
            "Show which files arrived in each module in the last 7 days.",
            "Anzeigen, welche Dateien in den letzten 7 Tagen in welchem Modul dazugekommen sind.",
        )
    }
    pub fn feature_update(self) -> &'static str {
        self.pick("Updates", "Aktualisierungen")
    }
    pub fn feature_update_hint(self) -> &'static str {
        self.pick(
            "Check for a new version of File Sorter once a day.",
            "Einmal am Tag nach einer neuen Version von File Sorter suchen.",
        )
    }
    pub fn update_ask(self) -> &'static str {
        self.pick("Ask first", "Vorher fragen")
    }
    pub fn update_auto(self) -> &'static str {
        self.pick("Install automatically", "Automatisch installieren")
    }
    pub fn none(self) -> &'static str {
        self.pick("None", "Keine")
    }
    pub fn all(self) -> &'static str {
        self.pick("All", "Alle")
    }
    pub fn sort_existing_on_start_hint(self) -> &'static str {
        self.pick(
            "When watching starts, also sort the files already in the folder: none, only recent ones, or all.",
            "Beim Start der Überwachung auch Dateien sortieren, die schon im Ordner liegen: keine, nur neue oder alle.",
        )
    }
    pub fn file_types(self) -> &'static str {
        self.pick("File types", "Dateitypen")
    }
    pub fn file_types_hint(self) -> &'static str {
        self.pick(
            "Only these types are sorted from the watched folder. (The right-click menu sorts any file.)",
            "Nur diese Typen werden aus dem überwachten Ordner sortiert. (Das Rechtsklick-Menü sortiert jede Datei.)",
        )
    }
    pub fn more_types(self) -> &'static str {
        self.pick("More:", "Weitere:")
    }
    pub fn defaults(self) -> &'static str {
        self.pick("Defaults", "Standard")
    }
    pub fn no_types(self) -> &'static str {
        self.pick(
            "No file type selected – nothing will be sorted.",
            "Kein Dateityp ausgewählt – es wird nichts sortiert.",
        )
    }

    pub fn startup(self) -> &'static str {
        self.pick("Startup", "Starten")
    }
    pub fn autostart(self) -> &'static str {
        self.pick("Start when I sign in", "Beim Anmelden starten")
    }
    pub fn autostart_hint(self) -> &'static str {
        self.pick(
            "File Sorter opens minimised in the taskbar and starts watching right away.",
            "File Sorter öffnet sich minimiert in der Taskleiste und überwacht sofort.",
        )
    }
    pub fn watch_on_open(self) -> &'static str {
        self.pick("Start watching right away", "Überwachung sofort starten")
    }
    pub fn watch_on_open_hint(self) -> &'static str {
        self.pick(
            "Begin watching as soon as the window opens.",
            "Gleich beim Öffnen des Fensters mit dem Überwachen beginnen.",
        )
    }

    pub fn right_click(self) -> &'static str {
        self.pick("Right-click menu", "Rechtsklick-Menü")
    }
    pub fn context_menu_setting(self) -> &'static str {
        self.pick("Show «Sort with File Sorter»", "«Mit File Sorter sortieren» anzeigen")
    }
    pub fn context_menu_hint(self) -> &'static str {
        if cfg!(windows) {
            self.pick(
                "Right-click files or a folder to sort them right away. On Windows 11 it's under «Show more options».",
                "Rechtsklick auf Dateien oder einen Ordner sortiert sie sofort. Unter Windows 11 unter «Weitere Optionen anzeigen».",
            )
        } else {
            self.pick(
                "Right-click files or a folder to sort them right away: in Files (Nautilus) and Caja under «Scripts», in Nemo, Dolphin and Thunar directly, everywhere else under «Open with».",
                "Rechtsklick auf Dateien oder einen Ordner sortiert sie sofort: in Dateien (Nautilus) und Caja unter «Skripte», in Nemo, Dolphin und Thunar direkt, sonst überall unter «Öffnen mit».",
            )
        }
    }
    pub fn results_window(self) -> &'static str {
        self.pick("Show the results window", "Ergebnisfenster anzeigen")
    }
    pub fn results_window_hint(self) -> &'static str {
        self.pick(
            "After sorting from the right-click menu, briefly show where the files went.",
            "Nach dem Sortieren per Rechtsklick kurz zeigen, wohin die Dateien gekommen sind.",
        )
    }

    pub fn about(self) -> &'static str {
        self.pick("About", "Info")
    }
    pub fn about_text(self) -> &'static str {
        self.pick(
            "Settings are saved automatically. The log file lists everything File Sorter did.",
            "Einstellungen werden automatisch gespeichert. Die Log-Datei listet alles auf, was File Sorter gemacht hat.",
        )
    }
    pub fn open_log(self) -> &'static str {
        self.pick("Open log file", "Log-Datei öffnen")
    }
    pub fn open_settings_folder(self) -> &'static str {
        self.pick("Open settings folder", "Einstellungsordner öffnen")
    }

    // ---------- dialogs ----------
    pub fn add_module(self) -> &'static str {
        self.pick("Add module", "Modul hinzufügen")
    }
    pub fn edit_module(self) -> &'static str {
        self.pick("Edit module", "Modul bearbeiten")
    }
    pub fn study_program(self) -> &'static str {
        self.pick("Study program:", "Studienfach:")
    }
    pub fn module(self) -> &'static str {
        self.pick("Module:", "Modul:")
    }
    pub fn ok(self) -> &'static str {
        "OK"
    }
    pub fn cancel(self) -> &'static str {
        self.pick("Cancel", "Abbrechen")
    }
    pub fn delete_module(self) -> &'static str {
        self.pick("Delete module", "Modul löschen")
    }
    pub fn delete_confirm(self, label: &str) -> String {
        self.fmt(
            format!("Remove «{label}»? Its folder and files stay where they are."),
            format!("«{label}» entfernen? Ordner und Dateien bleiben erhalten."),
        )
    }

    // ---------- errors ----------
    pub fn folder_not_found(self) -> &'static str {
        self.pick("Folder not found", "Ordner nicht gefunden")
    }
    pub fn study_folder_missing(self, path: &str) -> String {
        self.fmt(format!("Study folder does not exist:\n{path}"), format!("Der Studienordner existiert nicht:\n{path}"))
    }
    pub fn watch_folder_missing(self, path: &str) -> String {
        self.fmt(
            format!("Watch folder does not exist:\n{path}"),
            format!("Der überwachte Ordner existiert nicht:\n{path}"),
        )
    }
    pub fn no_modules_found(self) -> &'static str {
        self.pick("No modules found", "Keine Module gefunden")
    }
    pub fn no_modules_found_text(self) -> &'static str {
        self.pick(
            "No module folders found.\nExpected: Study folder \\ Study program \\ Module",
            "Keine Modulordner gefunden.\nErwartet: Studienordner \\ Studienfach \\ Modul",
        )
    }
    pub fn no_modules(self) -> &'static str {
        self.pick("No modules", "Keine Module")
    }
    pub fn no_modules_text(self) -> &'static str {
        self.pick(
            "Add or import at least one module first.",
            "Füge zuerst mindestens ein Modul hinzu oder lies die Modulordner ein.",
        )
    }
    pub fn could_not_start(self) -> &'static str {
        self.pick("Could not start", "Konnte nicht starten")
    }
    pub fn already_running(self) -> &'static str {
        self.pick(
            "File Sorter is already running and watching (another window or in the background). \
             Close the other one first.",
            "File Sorter läuft bereits und überwacht schon (anderes Fenster oder im Hintergrund). \
             Schliesse zuerst das andere.",
        )
    }
    pub fn could_not_save(self) -> &'static str {
        self.pick("Could not save settings", "Einstellungen konnten nicht gespeichert werden")
    }

    // ---------- cloud ----------
    pub fn cloud(self) -> &'static str {
        "Cloud"
    }
    pub fn cloud_title(self) -> &'static str {
        self.pick("Study folder in the cloud", "Studienordner in der Cloud")
    }
    pub fn cloud_hint(self) -> &'static str {
        self.pick(
            "So your files are on your phone, tablet and other computers too.",
            "Damit deine Dateien auch auf dem Handy, Tablet und anderen Computern da sind.",
        )
    }
    pub fn cloud_off(self) -> &'static str {
        self.pick("Off – only on this computer", "Aus – nur auf diesem Computer")
    }
    pub fn cloud_off_hint(self) -> &'static str {
        self.pick("Nothing goes to the cloud.", "Nichts kommt in die Cloud.")
    }
    pub fn cloud_move(self) -> &'static str {
        self.pick("Move into the cloud", "In die Cloud verlegen")
    }
    pub fn cloud_move_hint(self) -> &'static str {
        self.pick(
            "The study folder lives in the cloud folder and is the same everywhere. Notes you add on \
             another device come back here too.",
            "Der Studienordner liegt im Cloud-Ordner und ist überall gleich. Notizen, die du auf einem \
             anderen Gerät machst, kommen auch hierher zurück.",
        )
    }
    pub fn cloud_mirror(self) -> &'static str {
        self.pick("Copy in the cloud", "Kopie in der Cloud")
    }
    pub fn cloud_mirror_hint(self) -> &'static str {
        self.pick(
            "The study folder stays here; File Sorter keeps a copy in the cloud folder up to date. One way \
             only: changes made on other devices don't come back.",
            "Der Studienordner bleibt hier; File Sorter hält eine Kopie im Cloud-Ordner aktuell. Nur in eine \
             Richtung: Änderungen auf anderen Geräten kommen nicht zurück.",
        )
    }
    pub fn cloud_folder(self) -> &'static str {
        self.pick("Cloud folder", "Cloud-Ordner")
    }
    pub fn cloud_searching(self) -> &'static str {
        self.pick("Looking for OneDrive, Dropbox and Google Drive…", "Suche OneDrive, Dropbox und Google Drive…")
    }
    pub fn cloud_none_found(self) -> &'static str {
        self.pick(
            "No cloud folder found. Is OneDrive, Dropbox or Google Drive installed and signed in? You can \
             also choose a folder yourself.",
            "Kein Cloud-Ordner gefunden. Ist OneDrive, Dropbox oder Google Drive installiert und angemeldet? \
             Du kannst auch selbst einen Ordner wählen.",
        )
    }
    pub fn other_folder(self) -> &'static str {
        self.pick("Other folder…", "Anderer Ordner…")
    }
    pub fn provider(self, provider: Provider) -> &'static str {
        match provider {
            Provider::OneDrive => "OneDrive",
            Provider::OneDriveBusiness => self.pick("OneDrive (work or school)", "OneDrive (Arbeit/Schule)"),
            Provider::Dropbox => "Dropbox",
            Provider::GoogleDrive => "Google Drive",
            Provider::ICloud => "iCloud Drive",
            Provider::Other => self.pick("the cloud folder", "dem Cloud-Ordner"),
        }
    }
    /// The service's name with its account, for the folder choice ("OneDrive – Contoso").
    pub fn provider_label(self, provider: Provider, account: Option<&str>, folder: &str) -> String {
        match (provider, account) {
            (Provider::Other, _) => folder.to_string(),
            (Provider::OneDriveBusiness, Some(org)) => format!("OneDrive – {org}"),
            (_, Some(account)) => format!("{} – {account}", self.provider(provider)),
            (_, None) => self.provider(provider).to_string(),
        }
    }
    /// Where the files can be seen in a browser.
    pub fn provider_web(self, provider: Provider) -> Option<&'static str> {
        match provider {
            Provider::OneDrive => Some("onedrive.live.com"),
            Provider::OneDriveBusiness => Some("office.com"),
            Provider::Dropbox => Some("dropbox.com"),
            Provider::GoogleDrive => Some("drive.google.com"),
            Provider::ICloud => Some("icloud.com"),
            Provider::Other => None,
        }
    }
    pub fn cloud_target(self, path: &str) -> String {
        self.fmt(format!("Goes to {path}"), format!("Kommt nach {path}"))
    }
    pub fn cloud_apply_move(self) -> &'static str {
        self.pick("Move now", "Jetzt verlegen")
    }
    pub fn cloud_apply_mirror(self) -> &'static str {
        self.pick("Set up the copy", "Kopie einrichten")
    }
    pub fn cloud_apply_off(self) -> &'static str {
        self.pick("Turn off", "Ausschalten")
    }
    pub fn cloud_already_there(self, provider: &str) -> String {
        self.fmt(
            format!("Your study folder is already in {provider} – nothing needs to be moved."),
            format!("Dein Studienordner liegt schon in {provider} – es muss nichts verschoben werden."),
        )
    }
    pub fn cloud_status_moved(self, provider: &str, path: &str) -> String {
        self.fmt(
            format!("Your study folder is in {provider}: {path}"),
            format!("Dein Studienordner liegt in {provider}: {path}"),
        )
    }
    pub fn cloud_status_mirror(self, provider: &str, path: &str) -> String {
        self.fmt(format!("A copy is kept in {provider}: {path}"), format!("Eine Kopie liegt in {provider}: {path}"))
    }
    pub fn cloud_other_devices(self, provider: Provider) -> String {
        let name = self.provider(provider);
        match self.provider_web(provider) {
            Some(web) => self.fmt(
                format!("On your other devices: in the {name} app or on {web}."),
                format!("Auf deinen anderen Geräten: in der {name}-App oder auf {web}."),
            ),
            None => self.fmt(
                "On your other devices: wherever this folder is synced to.".into(),
                "Auf deinen anderen Geräten: überall, wohin dieser Ordner abgeglichen wird.".into(),
            ),
        }
    }
    pub fn ago(self, seconds: u64) -> String {
        match seconds {
            0..60 => self.pick("just now", "gerade eben").into(),
            60..3600 => self.fmt(format!("{} min ago", seconds / 60), format!("vor {} Min.", seconds / 60)),
            _ => self.fmt(format!("{} h ago", seconds / 3600), format!("vor {} Std.", seconds / 3600)),
        }
    }
    pub fn last_sync(self, ago: &str, copied: u64, updated: u64) -> String {
        match (copied, updated) {
            (0, 0) => self.fmt(format!("Up to date ({ago})"), format!("Auf dem neuesten Stand ({ago})")),
            _ => self.fmt(
                format!("Updated {ago}: {copied} new, {updated} changed"),
                format!("Abgeglichen {ago}: {copied} neu, {updated} geändert"),
            ),
        }
    }
    pub fn sync_now(self) -> &'static str {
        self.pick("Update now", "Jetzt abgleichen")
    }
    pub fn syncing(self) -> &'static str {
        self.pick("Updating the copy…", "Kopie wird abgeglichen…")
    }
    pub fn cloud_unreachable(self, provider: &str) -> String {
        self.fmt(
            format!("{provider} can't be reached right now; the copy is updated later."),
            format!("{provider} ist gerade nicht erreichbar – die Kopie wird später abgeglichen."),
        )
    }
    pub fn mirror_conflicts(self, count: usize) -> String {
        self.fmt(
            format!(
                "{count} file(s) were changed both in the cloud and here: your version is next to it, with this \
                 computer's name in brackets."
            ),
            format!(
                "{count} Datei(en) wurden in der Cloud und hier geändert: deine Version liegt daneben, mit dem \
                 Namen dieses Computers in Klammern."
            ),
        )
    }
    pub fn changed_in_cloud(self, count: u64) -> String {
        self.fmt(
            format!("{count} file(s) were changed on another device; those changes are only in the cloud copy."),
            format!(
                "{count} Datei(en) wurden auf einem anderen Gerät geändert; diese Änderungen sind nur in der Kopie \
                 in der Cloud."
            ),
        )
    }
    pub fn moving_progress(self, done: u64, total: u64) -> String {
        self.fmt(format!("Moving file {done} of {total}…"), format!("Verschiebe Datei {done} von {total}…"))
    }
    pub fn share_settings(self) -> &'static str {
        self.pick("Share modules with my other computers", "Module mit meinen anderen Computern teilen")
    }
    pub fn share_hint(self) -> &'static str {
        self.pick(
            "Modules, keywords and file types are kept in the study folder in the cloud, so every computer \
             sorts the same way.",
            "Module, Schlüsselwörter und Dateitypen liegen im Studienordner in der Cloud, damit jeder Computer \
             gleich sortiert.",
        )
    }
    pub fn share_needs_cloud(self) -> &'static str {
        self.pick(
            "Needs the study folder in the cloud (moved there or as a copy).",
            "Dafür muss der Studienordner in der Cloud liegen (verlegt oder als Kopie).",
        )
    }
    pub fn snack_shared_from(self, computer: &str) -> String {
        self.fmt(format!("Modules taken over from {computer}"), format!("Module von {computer} übernommen"))
    }
    pub fn snack_mirror_off(self, provider: &str) -> String {
        self.fmt(
            format!("The copy in {provider} stays, but is no longer updated"),
            format!("Die Kopie in {provider} bleibt, wird aber nicht mehr aktualisiert"),
        )
    }
    pub fn cloud_chip(self, provider: &str, copy: bool) -> String {
        if copy { self.fmt(format!("Copy in {provider}"), format!("Kopie in {provider}")) } else { provider.into() }
    }

    // cloud dialogs
    pub fn confirm_move_title(self, provider: &str) -> String {
        self.fmt(format!("Move the study folder into {provider}?"), format!("Studienordner nach {provider} verlegen?"))
    }
    pub fn confirm_move_text(self, files: u64, size: &str, from: &str, to: &str) -> String {
        self.fmt(
            format!(
                "{files} files ({size}) are moved from {from} to {to}. Nothing is overwritten: if a different \
                 file with the same name is already there, the moved one gets a number."
            ),
            format!(
                "{files} Dateien ({size}) werden von {from} nach {to} verschoben. Nichts wird überschrieben: \
                 Liegt dort schon eine andere Datei mit dem gleichen Namen, bekommt die verschobene eine Nummer."
            ),
        )
    }
    pub fn confirm_existing(self, files: u64) -> String {
        self.fmt(
            format!("There are already {files} files in that folder in the cloud; both are merged."),
            format!("In diesem Ordner in der Cloud liegen schon {files} Dateien; beides wird zusammengeführt."),
        )
    }
    pub fn confirm_close_documents(self) -> &'static str {
        self.pick(
            "Close open documents first: files that are still open stay where they are.",
            "Schliesse offene Dokumente vorher: Dateien, die noch geöffnet sind, bleiben, wo sie sind.",
        )
    }
    pub fn move_rest(self) -> &'static str {
        self.pick("Move them now", "Jetzt nachholen")
    }
    pub fn move_button(self) -> &'static str {
        self.pick("Move", "Verlegen")
    }
    pub fn confirm_mirror_title(self, provider: &str) -> String {
        self.fmt(format!("Keep a copy in {provider}?"), format!("Kopie in {provider} anlegen?"))
    }
    pub fn confirm_mirror_text(self, files: u64, size: &str, to: &str) -> String {
        self.fmt(
            format!("{files} files ({size}) are copied to {to}. The study folder here stays as it is."),
            format!("{files} Dateien ({size}) werden nach {to} kopiert. Der Studienordner hier bleibt, wie er ist."),
        )
    }
    pub fn mirror_button(self) -> &'static str {
        self.pick("Copy", "Kopieren")
    }
    pub fn back_title(self) -> &'static str {
        self.pick("Bring the study folder back to this computer?", "Studienordner zurück auf diesen Computer holen?")
    }
    pub fn back_text(self, from: &str, to: &str) -> String {
        self.fmt(
            format!("The files are moved from {from} back to {to}. Then they are no longer on your other devices."),
            format!(
                "Die Dateien werden von {from} zurück nach {to} verschoben. Danach sind sie nicht mehr auf deinen \
                 anderen Geräten."
            ),
        )
    }
    pub fn back_button(self) -> &'static str {
        self.pick("Bring back", "Zurückholen")
    }
    pub fn leave_button(self, provider: &str) -> String {
        self.fmt(format!("Leave it in {provider}"), format!("In {provider} lassen"))
    }
    pub fn shared_conflict_title(self) -> &'static str {
        self.pick("Other modules in the cloud", "Andere Module in der Cloud")
    }
    pub fn shared_conflict_text(self, computer: &str, theirs: usize, ours: usize) -> String {
        self.fmt(
            format!(
                "{computer} saved {theirs} module(s) in the cloud; this computer has {ours} that differ. Which \
                 should all your computers use?"
            ),
            format!(
                "{computer} hat {theirs} Modul(e) in der Cloud gespeichert; dieser Computer hat {ours} andere. \
                 Welche sollen alle deine Computer verwenden?"
            ),
        )
    }
    pub fn take_cloud(self) -> &'static str {
        self.pick("Use the ones in the cloud", "Die aus der Cloud")
    }
    pub fn keep_mine(self) -> &'static str {
        self.pick("Keep mine", "Meine behalten")
    }

    // cloud activity entries
    pub fn relocated(self, provider: &str, files: u64) -> String {
        self.fmt(
            format!("Study folder moved to {provider}: {files} files"),
            format!("Studienordner nach {provider} verlegt: {files} Dateien"),
        )
    }
    pub fn relocated_back(self, files: u64) -> String {
        self.fmt(
            format!("Study folder back on this computer: {files} files"),
            format!("Studienordner zurück auf diesem Computer: {files} Dateien"),
        )
    }
    pub fn relocate_details(self, duplicates: u64, renamed: usize, left: usize) -> String {
        let mut parts = Vec::new();
        if duplicates > 0 {
            parts.push(self.fmt(
                format!("{duplicates} already there (identical)"),
                format!("{duplicates} schon vorhanden (identisch)"),
            ));
        }
        if renamed > 0 {
            parts.push(self.fmt(format!("{renamed} saved with a number"), format!("{renamed} mit Nummer gespeichert")));
        }
        if left > 0 {
            parts.push(self.fmt(
                format!("{left} still open – they stay at the old place"),
                format!("{left} noch geöffnet – sie bleiben am alten Ort"),
            ));
        }
        parts.join(" · ")
    }
    pub fn relocate_failed(self, error: &str) -> String {
        self.fmt(
            format!("Could not move the study folder: {error}"),
            format!("Studienordner konnte nicht verlegt werden: {error}"),
        )
    }
    pub fn relocate_cancelled(self, moved: u64) -> String {
        self.fmt(
            format!("Moving stopped after {moved} files; the rest is still at the old place."),
            format!("Verlegen nach {moved} Dateien gestoppt; der Rest liegt noch am alten Ort."),
        )
    }
    pub fn cloud_same_folder(self) -> &'static str {
        self.pick(
            "The study folder can't be moved into itself. Choose another cloud folder.",
            "Der Studienordner kann nicht in sich selbst verschoben werden. Wähle einen anderen Cloud-Ordner.",
        )
    }
    pub fn size(self, bytes: u64) -> String {
        let (value, unit) = match bytes {
            0 => return "0 KB".into(),
            // Tiny files would read "0.0 KB".
            1..100 => (0.1, "KB"),
            100..1_000_000 => (bytes as f64 / 1e3, "KB"),
            1_000_000..1_000_000_000 => (bytes as f64 / 1e6, "MB"),
            _ => (bytes as f64 / 1e9, "GB"),
        };
        let text = if value < 10.0 { format!("{value:.1}") } else { format!("{value:.0}") };
        let text = if self == Lang::De { text.replace('.', ",") } else { text };
        format!("{text} {unit}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locale_detection() {
        assert_eq!(Lang::from_locale("de-CH"), Lang::De);
        assert_eq!(Lang::from_locale("de_DE.UTF-8"), Lang::De);
        assert_eq!(Lang::from_locale("en-US"), Lang::En);
        assert_eq!(Lang::from_locale("fr-CH"), Lang::En);
        assert_eq!(Lang::from_setting(Some("de")), Lang::De);
        assert_eq!(Lang::from_setting(Some("en")), Lang::En);
        assert_eq!(Lang::from_setting(Some(Lang::De.code())), Lang::De);
    }

    #[test]
    fn texts_differ_and_are_swiss() {
        assert_eq!(Lang::En.start(), "Start watching");
        assert_eq!(Lang::De.start(), "Überwachung starten");
        assert_eq!(Lang::De.reason(&Reason::FileName(vec!["prog1".into()])), "Dateiname enthält «prog1»");
        assert!(Lang::De.already_running().contains("Schliesse"));
        assert!(!Lang::De.already_running().contains('ß'));
    }

    #[test]
    fn sizes_and_times() {
        assert_eq!(Lang::En.size(340_000_000), "340 MB");
        assert_eq!(Lang::De.size(1_234_000_000), "1,2 GB");
        assert_eq!(Lang::De.size(2_500), "2,5 KB");
        assert_eq!(Lang::De.size(25), "0,1 KB");
        assert_eq!(Lang::De.ago(5), "gerade eben");
        assert_eq!(Lang::De.ago(150), "vor 2 Min.");
        assert_eq!(Lang::En.provider_label(Provider::OneDriveBusiness, Some("Contoso"), ""), "OneDrive – Contoso");
        assert_eq!(Lang::De.provider_label(Provider::Dropbox, None, ""), "Dropbox");
    }
}
