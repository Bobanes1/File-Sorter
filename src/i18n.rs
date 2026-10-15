//! All texts shown in the window, in English and German.
//!
//! Every text is one method that names both languages, so a translation can't be
//! forgotten. German uses Swiss spelling ("ss", «…»).

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
    pub fn watching(self, folder: &str) -> String {
        self.fmt(format!("Watching {folder}"), format!("Überwacht {folder}"))
    }
    pub fn stopped(self) -> &'static str {
        self.pick("Not watching", "Überwachung aus")
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
    pub fn stat_modules(self) -> &'static str {
        self.pick("Modules", "Module")
    }

    // ---------- folders ----------
    pub fn folders(self) -> &'static str {
        self.pick("Folders", "Ordner")
    }
    pub fn watch_folder(self) -> &'static str {
        self.pick("Watch (Downloads):", "Überwachen (Downloads):")
    }
    pub fn study_folder(self) -> &'static str {
        self.pick("Study folder:", "Studienordner:")
    }
    pub fn browse(self) -> &'static str {
        self.pick("Browse…", "Durchsuchen…")
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
    pub fn keywords_label(self) -> &'static str {
        self.pick(
            "Keywords (one per line, e.g. module code, short name, professor):",
            "Stichwörter (eins pro Zeile, z. B. Modulkürzel, Kurzname, Dozent/in):",
        )
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
        self.pick("Sort existing files now", "Vorhandene Dateien jetzt sortieren")
    }
    pub fn save(self) -> &'static str {
        self.pick("Save", "Speichern")
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
            format!("Already in {course} – duplicate removed from Downloads"),
            format!("Schon in {course} – Duplikat aus Downloads entfernt"),
        )
    }
    pub fn reason(self, reason: &Reason) -> String {
        let words = reason.keywords().join(", ");
        match reason {
            Reason::FileName(_) => {
                self.fmt(format!("file name contains «{words}»"), format!("Dateiname enthält «{words}»"))
            }
            Reason::Content(_) => self.fmt(format!("content mentions «{words}»"), format!("Inhalt erwähnt «{words}»")),
        }
    }
    pub fn left_reason(self, reason: &LeftReason) -> String {
        match reason {
            LeftReason::Gone => self.pick("File disappeared", "Datei ist verschwunden").into(),
            LeftReason::NotDocument => self.pick("Not a document type", "Kein Dokument-Dateityp").into(),
            LeftReason::NoMatch => self
                .pick("No matching module – stays in Downloads", "Kein passendes Modul – bleibt in Downloads")
                .into(),
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
    pub fn settings_saved(self) -> &'static str {
        self.pick("Settings saved", "Einstellungen gespeichert")
    }
    pub fn sorting_existing(self) -> &'static str {
        self.pick("Sorting files already in the folder…", "Sortiere die Dateien, die schon im Ordner liegen…")
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
    pub fn sort_existing_on_start_hint(self) -> &'static str {
        self.pick(
            "When watching starts, also sort the files already in the folder.",
            "Beim Start der Überwachung auch die Dateien sortieren, die schon im Ordner liegen.",
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
}
