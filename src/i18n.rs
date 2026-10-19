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
    pub fn folders(self) -> &'static str {
        self.pick("Folders", "Ordner")
    }
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
    pub fn sort_now_tooltip(self, days: Option<u32>) -> String {
        match days {
            Some(d) => self.fmt(
                format!("Sort the files in the downloads folder from the last {d} days"),
                format!("Die Dateien der letzten {d} Tage im Download-Ordner sortieren"),
            ),
            None => {
                self.pick("Sort all files in the downloads folder", "Alle Dateien im Download-Ordner sortieren").into()
            }
        }
    }
    pub fn last_days(self, days: u32) -> String {
        self.fmt(format!("Last {days} days"), format!("Letzte {days} Tage"))
    }
    pub fn all_files(self) -> &'static str {
        self.pick("All files", "Alle Dateien")
    }
    pub fn sort_recent(self, days: u32) -> String {
        self.fmt(format!("Only the last {days} days"), format!("Nur die letzten {days} Tage"))
    }
    pub fn sort_recent_hint(self, days: u32) -> String {
        self.fmt(
            format!("Files downloaded or changed in the last {days} days"),
            format!("Dateien, die in den letzten {days} Tagen heruntergeladen oder geändert wurden"),
        )
    }
    pub fn sort_all(self) -> &'static str {
        self.pick("All files", "Alle Dateien")
    }
    pub fn sort_all_hint(self) -> &'static str {
        self.pick("Everything in the downloads folder, however old", "Alles im Download-Ordner, egal wie alt")
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
    pub fn sorting_existing(self, days: Option<u32>) -> String {
        match days {
            Some(d) => self.fmt(
                format!("Sorting the files from the last {d} days…"),
                format!("Sortiere die Dateien der letzten {d} Tage…"),
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
    pub fn feature_subfolders(self) -> &'static str {
        self.pick("Subfolders", "Unterordner")
    }
    pub fn feature_subfolders_hint(self) -> &'static str {
        self.pick(
            "Put lecture slides, exercises, solutions and exams into their own folder inside the module.",
            "Vorlesungsfolien, Übungen, Lösungen und Prüfungen im Modul in eigene Ordner legen.",
        )
    }
    pub fn subfolder_names_hint(self) -> &'static str {
        self.pick(
            "Folder names for: lectures · exercises · solutions · exams. Other files stay in the module folder.",
            "Ordnernamen für: Vorlesung · Übungen · Lösungen · Prüfungen. Andere Dateien bleiben im Modulordner.",
        )
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
        self.pick(
            "Right-click files or a folder to sort them right away. On Windows 11 it's under «Show more options».",
            "Rechtsklick auf Dateien oder einen Ordner sortiert sie sofort. Unter Windows 11 unter «Weitere Optionen anzeigen».",
        )
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
