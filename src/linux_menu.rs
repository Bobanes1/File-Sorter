//! «Sort with File Sorter» in the right-click menu of the Linux file managers: a script for
//! Nautilus (GNOME Files) and Caja, an action for Nemo, a service menu for Dolphin, a custom
//! action for Thunar – each only where that file manager is installed – and an «Open with»
//! entry, which every file manager offers.
//!
//! All of it lives in the user's own folders and is removed again when the setting is off.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Where each kind of entry goes.
#[derive(Debug, Clone)]
pub struct Places {
    pub data: PathBuf,
    pub config: PathBuf,
    /// The file managers found (`nautilus`, `nemo`, `caja`, `dolphin`, `thunar`).
    pub installed: Vec<&'static str>,
}

const MANAGERS: [&str; 5] = ["nautilus", "nemo", "caja", "dolphin", "thunar"];
const THUNAR_ID: &str = "file-sorter-sort";

impl Places {
    pub fn of_this_user() -> Option<Places> {
        let installed = MANAGERS.into_iter().filter(|m| on_path(m)).collect();
        Some(Places { data: dirs::data_dir()?, config: dirs::config_dir()?, installed })
    }

    fn nautilus_script(&self, label: &str) -> PathBuf {
        self.data.join("nautilus").join("scripts").join(label)
    }

    fn caja_script(&self, label: &str) -> PathBuf {
        self.config.join("caja").join("scripts").join(label)
    }

    fn nemo_action(&self) -> PathBuf {
        self.data.join("nemo").join("actions").join("file-sorter.nemo_action")
    }

    fn dolphin_menu(&self) -> PathBuf {
        self.data.join("kio").join("servicemenus").join("file-sorter.desktop")
    }

    fn thunar_actions(&self) -> PathBuf {
        self.config.join("Thunar").join("uca.xml")
    }

    fn open_with(&self) -> PathBuf {
        self.data.join("applications").join("file-sorter-sort.desktop")
    }

    /// The icon the entries name (`file-sorter`).
    fn icon(&self) -> PathBuf {
        self.data.join("icons").join("hicolor").join("256x256").join("apps").join("file-sorter.png")
    }
}

fn on_path(program: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|paths| std::env::split_paths(&paths).any(|dir| dir.join(program).is_file()))
}

/// A path in single quotes for `sh`.
fn shell_quoted(path: &Path) -> String {
    format!("'{}'", path.to_string_lossy().replace('\'', r"'\''"))
}

/// A path for the `Exec=` line of a desktop entry (double quotes, `"`, `` ` ``, `$` and `\`
/// escaped).
fn exec_quoted(path: &Path) -> String {
    let mut out = String::from("\"");
    for c in path.to_string_lossy().chars() {
        if matches!(c, '"' | '`' | '$' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('"');
    out
}

fn xml_escaped(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

fn write(path: &Path, text: &str, executable: bool) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    fs::write(path, text)?;
    #[cfg(unix)]
    if executable {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o755))?;
    }
    #[cfg(not(unix))]
    let _ = executable;
    Ok(())
}

fn remove(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

/// The script Nautilus and Caja run with the chosen files.
fn script(program: &Path) -> String {
    format!(
        "#!/bin/sh\n\
         # Added by File Sorter («Sort with File Sorter»); switched off in its settings.\n\
         IFS='\n'\n\
         if [ -n \"$NAUTILUS_SCRIPT_SELECTED_FILE_PATHS\" ]; then set -- $NAUTILUS_SCRIPT_SELECTED_FILE_PATHS;\n\
         elif [ -n \"$CAJA_SCRIPT_SELECTED_FILE_PATHS\" ]; then set -- $CAJA_SCRIPT_SELECTED_FILE_PATHS; fi\n\
         exec {} --sort \"$@\"\n",
        shell_quoted(program)
    )
}

/// The document types offered «Open with File Sorter».
const MIME_TYPES: &str = "application/pdf;application/vnd.openxmlformats-officedocument.presentationml.presentation;\
application/vnd.ms-powerpoint;application/vnd.openxmlformats-officedocument.wordprocessingml.document;\
application/msword;application/vnd.oasis.opendocument.presentation;application/vnd.oasis.opendocument.text;\
application/vnd.openxmlformats-officedocument.spreadsheetml.sheet;application/zip;text/plain;text/markdown;\
application/x-ipynb+json;image/png;image/jpeg;inode/directory;";

/// Add (or refresh) the entries for `label`, removing ones made for an earlier label.
pub fn register(places: &Places, label: &str) -> io::Result<()> {
    unregister(places)?;
    let program = crate::desktop::program_file()?;
    let exec = format!("{} --sort %F", exec_quoted(&program));
    let has = |m: &str| places.installed.contains(&m);
    if has("nautilus") {
        write(&places.nautilus_script(label), &script(&program), true)?;
    }
    if has("caja") {
        write(&places.caja_script(label), &script(&program), true)?;
    }
    if has("nemo") {
        let action = format!(
            "[Nemo Action]\nName={label}\nComment={label}\nExec={exec}\nIcon-Name=file-sorter\n\
             Selection=notnone\nExtensions=any;dir;\nQuote=double\n"
        );
        write(&places.nemo_action(), &action, false)?;
    }
    if has("dolphin") {
        let menu = format!(
            "[Desktop Entry]\nType=Service\nMimeType=all/allfiles;inode/directory;\n\
             Actions=sortWithFileSorter;\nX-KDE-Priority=TopLevel\n\n\
             [Desktop Action sortWithFileSorter]\nName={label}\nIcon=file-sorter\nExec={exec}\n"
        );
        write(&places.dolphin_menu(), &menu, true)?;
    }
    if has("thunar") {
        let file = places.thunar_actions();
        let text = fs::read_to_string(&file)
            .unwrap_or_else(|_| "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<actions>\n</actions>\n".into());
        let action = format!(
            "<action>\n\t<icon>file-sorter</icon>\n\t<name>{}</name>\n\t<unique-id>{THUNAR_ID}</unique-id>\n\t\
             <command>{} --sort %F</command>\n\t<description>{}</description>\n\t<patterns>*</patterns>\n\t\
             <directories/>\n\t<audio-files/>\n\t<image-files/>\n\t<other-files/>\n\t<text-files/>\n\t<video-files/>\n</action>\n",
            xml_escaped(label),
            xml_escaped(&shell_quoted(&program)),
            xml_escaped(label)
        );
        if let Some(at) = text.rfind("</actions>") {
            write(&file, &format!("{}{action}{}", &text[..at], &text[at..]), false)?;
        }
    }
    if !places.icon().exists() {
        if let Some(dir) = places.icon().parent() {
            fs::create_dir_all(dir)?;
        }
        fs::write(places.icon(), include_bytes!("../assets/icon.png"))?;
    }
    let open_with = format!(
        "[Desktop Entry]\nType=Application\nName={label}\nIcon=file-sorter\nExec={exec}\n\
         MimeType={MIME_TYPES}\nNoDisplay=true\nTerminal=false\n"
    );
    write(&places.open_with(), &open_with, false)?;
    Ok(())
}

/// Remove every entry (whatever label it was made with).
pub fn unregister(places: &Places) -> io::Result<()> {
    // The scripts are named after the label: find them by what is in them.
    for dir in [places.data.join("nautilus").join("scripts"), places.config.join("caja").join("scripts")] {
        let Ok(entries) = fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            if fs::read_to_string(entry.path()).is_ok_and(|t| t.contains("Added by File Sorter")) {
                remove(&entry.path())?;
            }
        }
    }
    remove(&places.nemo_action())?;
    remove(&places.dolphin_menu())?;
    remove(&places.open_with())?;
    let thunar = places.thunar_actions();
    if let Ok(text) = fs::read_to_string(&thunar)
        && let Some(id) = text.find(&format!("<unique-id>{THUNAR_ID}</unique-id>"))
        && let Some(start) = text[..id].rfind("<action>")
        && let Some(end) = text[id..].find("</action>").map(|e| id + e + "</action>".len())
    {
        let end = if text[end..].starts_with('\n') { end + 1 } else { end };
        write(&thunar, &format!("{}{}", &text[..start], &text[end..]), false)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entries_for_every_file_manager_and_gone_again() {
        let tmp = tempfile::tempdir().unwrap();
        let places =
            Places { data: tmp.path().join("share"), config: tmp.path().join("config"), installed: MANAGERS.to_vec() };
        // Thunar has actions of its own already.
        let own = "<?xml version=\"1.0\"?>\n<actions>\n<action><name>Mine</name><unique-id>1</unique-id></action>\n</actions>\n";
        write(&places.thunar_actions(), own, false).unwrap();
        register(&places, "Sort with File Sorter").unwrap();
        let script = fs::read_to_string(places.nautilus_script("Sort with File Sorter")).unwrap();
        assert!(script.contains("--sort \"$@\"") && script.contains("NAUTILUS_SCRIPT_SELECTED_FILE_PATHS"));
        assert!(places.caja_script("Sort with File Sorter").is_file());
        assert!(fs::read_to_string(places.nemo_action()).unwrap().contains("--sort %F"));
        assert!(fs::read_to_string(places.dolphin_menu()).unwrap().contains("[Desktop Action sortWithFileSorter]"));
        let thunar = fs::read_to_string(places.thunar_actions()).unwrap();
        assert!(thunar.contains("<name>Mine</name>") && thunar.contains(THUNAR_ID));
        assert!(fs::read_to_string(places.open_with()).unwrap().contains("application/pdf"));
        // In another language: the old script goes.
        register(&places, "Mit File Sorter sortieren").unwrap();
        assert!(!places.nautilus_script("Sort with File Sorter").exists());
        assert_eq!(fs::read_to_string(places.thunar_actions()).unwrap().matches(THUNAR_ID).count(), 1);
        unregister(&places).unwrap();
        assert!(!places.nautilus_script("Mit File Sorter sortieren").exists());
        assert!(!places.nemo_action().exists() && !places.dolphin_menu().exists() && !places.open_with().exists());
        assert_eq!(fs::read_to_string(places.thunar_actions()).unwrap(), own, "Thunar's own actions stay as they were");
    }

    /// The script really passes the chosen files (with spaces in their names) on.
    #[test]
    fn the_script_hands_over_the_chosen_files() {
        let tmp = tempfile::tempdir().unwrap();
        let fake = tmp.path().join("fake sorter");
        write(&fake, "#!/bin/sh\nfor a in \"$@\"; do echo \"[$a]\"; done\n", true).unwrap();
        let script_file = tmp.path().join("script");
        write(&script_file, &script(&fake), true).unwrap();
        let out = std::process::Command::new(&script_file)
            .env("NAUTILUS_SCRIPT_SELECTED_FILE_PATHS", "/dl/Blatt 3.pdf\n/dl/VL2.pdf\n")
            .output()
            .unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout), "[--sort]\n[/dl/Blatt 3.pdf]\n[/dl/VL2.pdf]\n");
    }
}
