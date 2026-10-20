//! Finding the cloud folders on this computer: OneDrive (personal and work/school),
//! Dropbox, Google Drive and iCloud Drive, so the settings can offer them by name.
//!
//! - Windows:
//!   - OneDrive: its variables `OneDriveConsumer` / `OneDrive` (personal) and
//!     `OneDriveCommercial` (work/school), and its accounts in the registry ("Personal",
//!     "Business1", … with the organisation as `DisplayName`), because the variables know only
//!     one work or school account. With only a work account signed in, `OneDrive` names the
//!     work folder too, so a folder that is `OneDriveCommercial` counts as work/school only.
//!   - Dropbox: its `info.json` in `%APPDATA%\Dropbox` or `%LOCALAPPDATA%\Dropbox`.
//!   - Google Drive for desktop: a drive of its own (`G:\`) whose "My Drive" folder is named
//!     in the account's language ("Meine Ablage", "Mon Drive", …), or `~\My Drive` when it
//!     mirrors. When it streams into a folder instead of a drive letter, the My Drive in
//!     `~\Google Drive` is found (like the old Backup and Sync's folder); a streaming folder
//!     elsewhere is not (its setting is in DriveFS's own database), so it is picked by hand.
//!   - iCloud: `~\iCloudDrive`.
//! - macOS: the folders the apps keep in `~/Library/CloudStorage` (`OneDrive-Personal`,
//!   `OneDrive-<Org>`, `Dropbox`, `GoogleDrive-<email>/<My Drive>`), iCloud Drive in
//!   `~/Library/Mobile Documents/com~apple~CloudDocs`, Dropbox's `~/.dropbox/info.json`.
//! - Linux: Dropbox's `~/.dropbox/info.json`, `~/OneDrive`, `~/Google Drive`.
//!
//! Folders known only by their usual name (`~\OneDrive`, `~\OneDrive - <Org>`, `~/Dropbox`,
//! `~/Google Drive`) count only when the app's own records name none: a signed-out account or
//! an uninstalled app leaves its folder behind, and a study folder moved there would silently
//! stop syncing. For the same reason, on Windows OneDrive's and Dropbox's folders count only
//! while the app's program is there or Windows lists the folder as a sync root: uninstalling
//! leaves the records too (OneDrive's variables and registry keys, Dropbox's `info.json`).
//!
//! Only folders that exist are returned, each once (several sources often name the same
//! folder, spelled differently), sorted by [`Provider`] and otherwise in the order found.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::cloud::is_inside;

/// A cloud storage service.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    OneDrive,
    /// OneDrive for work or school.
    OneDriveBusiness,
    Dropbox,
    GoogleDrive,
    ICloud,
    /// A folder the user chose (Nextcloud, pCloud, a network drive, …).
    Other,
}

/// A cloud folder found on this computer.
#[derive(Debug, Clone, PartialEq)]
pub struct CloudRoot {
    pub provider: Provider,
    pub path: PathBuf,
    /// Account or organisation, when a service can have several ("Contoso", "anna@gmail.com").
    pub account: Option<String>,
}

/// What detection looks at: filled from the real system by [`Probe::system`], or by tests.
#[derive(Debug, Clone, Default)]
pub struct Probe {
    pub windows: bool,
    pub macos: bool,
    pub home: PathBuf,
    /// Environment variables (`OneDrive`, `OneDriveConsumer`, `OneDriveCommercial`,
    /// `APPDATA`, `LOCALAPPDATA`, `USERPROFILE`, `ProgramFiles`, `ProgramFiles(x86)`).
    pub vars: HashMap<String, String>,
    /// OneDrive accounts from the registry (Windows): key name ("Personal", "Business1"),
    /// its `UserFolder` and its `DisplayName`.
    pub onedrive_accounts: Vec<(String, PathBuf, Option<String>)>,
    /// Roots of drives that may be Google Drive (Windows: `G:\`), already checked to respond.
    pub drive_roots: Vec<PathBuf>,
    /// The apps whose program was found (Windows: [`Provider::OneDrive`] for both kinds of
    /// OneDrive, [`Provider::Dropbox`]); see [`programs`].
    pub installed: Vec<Provider>,
    /// The folders Windows lists as sync roots (`SyncRootManager`): OneDrive, newer Dropbox and
    /// others register theirs and remove them when unlinked, so a folder listed there is
    /// synced even if its program is somewhere [`programs`] doesn't look.
    pub sync_roots: Vec<PathBuf>,
}

/// Where the programs of the apps whose records outlive them can be (Windows): per user in
/// `%LOCALAPPDATA%`, per computer in Program Files, very old Dropbox in `%APPDATA%`.
fn programs(vars: &HashMap<String, String>) -> Vec<(Provider, PathBuf)> {
    let var = |name: &str| vars.get(name).map(|v| v.trim()).filter(|v| !v.is_empty()).map(PathBuf::from);
    let dropbox = |dir: PathBuf| (Provider::Dropbox, dir.join("Dropbox").join("Client").join("Dropbox.exe"));
    let mut found = Vec::new();
    if let Some(local) = var("LOCALAPPDATA") {
        found.push((Provider::OneDrive, local.join("Microsoft").join("OneDrive").join("OneDrive.exe")));
        found.push(dropbox(local));
    }
    for dir in [var("ProgramFiles"), var("ProgramFiles(x86)")].into_iter().flatten() {
        found.push((Provider::OneDrive, dir.join("Microsoft OneDrive").join("OneDrive.exe")));
        found.push(dropbox(dir));
    }
    if let Some(roaming) = var("APPDATA") {
        found.push((Provider::Dropbox, roaming.join("Dropbox").join("bin").join("Dropbox.exe")));
    }
    found
}

/// "My Drive" in Google Drive's languages: the folder is named in the account's language.
const MY_DRIVE: &[&str] = &[
    "My Drive",
    "Meine Ablage",
    "Mon Drive",
    "Mi unidad",
    "Il mio Drive",
    "Meu Drive",
    "Mijn Drive",
    "Mój dysk",
    "Min disk",
    "Oma Drive",
    "Můj disk",
    "Мой диск",
    "マイドライブ",
    "내 드라이브",
    "我的云端硬盘",
    "我的雲端硬碟",
];

/// How long to wait for all drives together: a network drive that isn't connected can take
/// many seconds to answer, and nobody should wait for that.
#[cfg_attr(not(windows), allow(dead_code))]
const DRIVE_WAIT: Duration = Duration::from_millis(1500);

impl Probe {
    /// The real computer. On Windows this can take up to 1.5 s (drives that are slow to
    /// answer), so better not on the UI thread.
    pub fn system() -> Probe {
        const VARS: [&str; 8] = [
            "OneDrive",
            "OneDriveConsumer",
            "OneDriveCommercial",
            "APPDATA",
            "LOCALAPPDATA",
            "USERPROFILE",
            "ProgramFiles",
            "ProgramFiles(x86)",
        ];
        let vars: HashMap<String, String> = VARS
            .iter()
            .filter_map(|&name| Some((name.to_string(), std::env::var(name).ok().filter(|v| !v.trim().is_empty())?)))
            .collect();
        let home = dirs::home_dir().unwrap_or_default();
        #[cfg(windows)]
        let (onedrive_accounts, drive_roots, sync_roots) =
            (win::onedrive_accounts(), win::drive_roots(), win::sync_roots());
        #[cfg(not(windows))]
        let (onedrive_accounts, drive_roots, sync_roots) = (Vec::new(), Vec::new(), Vec::new());
        let mut installed = Vec::new();
        if cfg!(windows) {
            for (app, program) in programs(&vars) {
                if !installed.contains(&app) && program.is_file() {
                    installed.push(app);
                }
            }
        }
        Probe {
            windows: cfg!(target_os = "windows"),
            macos: cfg!(target_os = "macos"),
            home,
            vars,
            onedrive_accounts,
            drive_roots,
            installed,
            sync_roots,
        }
    }
}

/// The cloud folders on this computer (can take up to 1.5 s, see [`Probe::system`]).
pub fn detect() -> Vec<CloudRoot> {
    let roots = detect_with(&Probe::system());
    let names: Vec<String> = roots.iter().map(|r| format!("{:?} {}", r.provider, r.path.display())).collect();
    log::info!("Cloud folders: {}", if names.is_empty() { "none".to_string() } else { names.join(", ") });
    roots
}

/// The cloud folders `probe` describes (only folders that exist).
pub fn detect_with(probe: &Probe) -> Vec<CloudRoot> {
    let mut found = Found { sync_roots: &probe.sync_roots, ..Found::default() };
    if probe.windows {
        use Provider::*;
        for (provider, app) in [(OneDrive, OneDrive), (OneDriveBusiness, OneDrive), (Dropbox, Dropbox)] {
            if !probe.installed.contains(&app) {
                found.unproven.push(provider);
            }
        }
    }
    let profile = if probe.windows { var(probe, "USERPROFILE") } else { None };
    let home = std::iter::once(probe.home.clone()).chain(profile).find(|h| h.is_absolute());
    if probe.windows {
        on_windows(probe, home.as_deref(), &mut found);
    } else if let Some(home) = &home {
        if probe.macos { on_macos(home, &mut found) } else { on_linux(home, &mut found) }
    }
    let mut roots = found.roots;
    roots.sort_by_key(|r| r.provider); // stable: the order found within a provider
    roots
}

/// The cloud folder `path` is in (or is), if any.
pub fn provider_of(path: &Path, roots: &[CloudRoot]) -> Option<CloudRoot> {
    let path = resolved(path);
    // Cloud folders can be nested (a Dropbox inside OneDrive): the innermost one counts.
    // `rev` because `max_by_key` keeps the last of equals, and the first root should win.
    roots.iter().filter(|r| is_inside(&path, &r.path)).rev().max_by_key(|r| r.path.components().count()).cloned()
}

/// `path` with its deepest existing folder resolved, so something that doesn't exist yet
/// (the study folder before it is moved) compares like the folder it will be in, also when
/// that folder is spelled differently (8.3 names like `ANNA~1`, symlinks like macOS `/var`).
fn resolved(path: &Path) -> PathBuf {
    let mut missing = Vec::new();
    let mut current = path;
    loop {
        if let Ok(real) = current.canonicalize() {
            return missing.iter().rev().fold(real, |full, name| full.join(name));
        }
        match (current.parent(), current.file_name()) {
            (Some(parent), Some(name)) => {
                missing.push(name);
                current = parent;
            }
            _ => return path.to_path_buf(),
        }
    }
}

/// The folders found so far: only existing ones, each once.
#[derive(Default)]
struct Found<'a> {
    roots: Vec<CloudRoot>,
    /// Providers whose app wasn't found: their folders count only if they are in `sync_roots`.
    unproven: Vec<Provider>,
    sync_roots: &'a [PathBuf],
}

impl Found<'_> {
    fn add(&mut self, provider: Provider, path: impl AsRef<Path>, account: Option<String>) {
        // Relative paths (an empty variable, no home folder) would depend on the current folder.
        let path: PathBuf = path.as_ref().components().collect();
        if !path.is_absolute() || !path.is_dir() {
            return;
        }
        if self.unproven.contains(&provider) && !self.sync_roots.iter().any(|r| same_folder(r, &path)) {
            log::info!("Not offering {} as {provider:?}: the app is not installed", path.display());
            return;
        }
        let account = account.map(|a| a.trim().to_string()).filter(|a| !a.is_empty());
        if let Some(known) = self.roots.iter_mut().find(|r| same_folder(&r.path, &path)) {
            // The same folder from another source: the first one stays, but may learn its account.
            if known.provider == provider && known.account.is_none() {
                known.account = account;
            }
        } else {
            self.roots.push(CloudRoot { provider, path, account });
        }
    }

    fn has(&self, provider: Provider) -> bool {
        self.roots.iter().any(|r| r.provider == provider)
    }
}

/// True if `a` and `b` are the same folder, however spelled.
fn same_folder(a: &Path, b: &Path) -> bool {
    is_inside(a, b) && is_inside(b, a)
}

fn var(probe: &Probe, name: &str) -> Option<PathBuf> {
    probe.vars.get(name).map(|v| v.trim()).filter(|v| !v.is_empty()).map(PathBuf::from)
}

fn on_windows(probe: &Probe, home: Option<&Path>, found: &mut Found) {
    use Provider::*;
    // Work or school first: with only such an account signed in, `OneDrive` names its folder
    // too, and that is not a personal OneDrive.
    if let Some(dir) = var(probe, "OneDriveCommercial") {
        found.add(OneDriveBusiness, dir, None);
    }
    let mut business: Vec<_> = probe
        .onedrive_accounts
        .iter()
        .filter_map(|(key, dir, name)| Some((business_number(key)?, dir, name)))
        .collect();
    business.sort_by_key(|(number, ..)| *number); // "Business2" before "Business10"
    for (_, dir, name) in business {
        found.add(OneDriveBusiness, dir, name.clone());
    }
    for dir in [var(probe, "OneDriveConsumer"), var(probe, "OneDrive")].into_iter().flatten() {
        found.add(OneDrive, dir, None);
    }
    for (key, dir, _) in &probe.onedrive_accounts {
        if key.eq_ignore_ascii_case("Personal") {
            found.add(OneDrive, dir, None);
        }
    }
    let onedrive_known = ["OneDrive", "OneDriveConsumer", "OneDriveCommercial"].iter().any(|v| var(probe, v).is_some())
        || !probe.onedrive_accounts.is_empty();
    if !onedrive_known && let Some(home) = home {
        found.add(OneDrive, home.join("OneDrive"), None);
        for (name, dir) in folders_in(home) {
            match name.strip_prefix("OneDrive - ") {
                Some(org) if is_personal(org) => found.add(OneDrive, dir, None),
                Some(org) => found.add(OneDriveBusiness, dir, Some(org.to_string())),
                None => {}
            }
        }
    }
    for root in found.roots.iter_mut().filter(|r| r.provider == OneDriveBusiness && r.account.is_none()) {
        root.account = org_of(&root.path);
    }

    let infos = [var(probe, "APPDATA"), var(probe, "LOCALAPPDATA")].into_iter().flatten();
    dropbox(infos.map(|dir| dir.join("Dropbox").join("info.json")), home.map(|h| h.join("Dropbox")), found);

    // Google Drive for desktop: a drive of its own when streaming, `~\My Drive` when mirroring.
    for root in &probe.drive_roots {
        if let Some(dir) = my_drive(root) {
            found.add(GoogleDrive, dir, None);
        }
    }
    if let Some(home) = home {
        if let Some(dir) = my_drive(home) {
            found.add(GoogleDrive, dir, None);
        }
        old_google_drive(home, found);
        found.add(ICloud, home.join("iCloudDrive"), None);
    }
}

fn on_macos(home: &Path, found: &mut Found) {
    use Provider::*;
    // Synced SharePoint libraries are shared with others, not the user's own OneDrive.
    let storage = folders_in(&home.join("Library").join("CloudStorage"));
    for (name, dir) in storage.into_iter().filter(|(name, _)| !name.starts_with("OneDrive-SharedLibraries")) {
        if name == "OneDrive-Personal" {
            found.add(OneDrive, dir, None);
        } else if let Some(org) = name.strip_prefix("OneDrive-") {
            found.add(OneDriveBusiness, dir, Some(org.to_string()));
        } else if name == "Dropbox" || name == "Dropbox-Personal" {
            found.add(Dropbox, dir, None);
        } else if let Some(team) = name.strip_prefix("Dropbox-") {
            found.add(Dropbox, dir, Some(team.to_string()));
        } else if let Some(email) = name.strip_prefix("GoogleDrive-")
            && let Some(mine) = my_drive(&dir)
        {
            // Next to My Drive are "Shared drives" and "Other computers", not the user's own.
            found.add(GoogleDrive, mine, Some(email.to_string()));
        }
    }
    found.add(ICloud, home.join("Library").join("Mobile Documents").join("com~apple~CloudDocs"), None);
    dropbox([home.join(".dropbox").join("info.json")], Some(home.join("Dropbox")), found);
}

fn on_linux(home: &Path, found: &mut Found) {
    dropbox([home.join(".dropbox").join("info.json")], Some(home.join("Dropbox")), found);
    found.add(Provider::OneDrive, home.join("OneDrive"), None);
    old_google_drive(home, found);
}

/// Dropbox's folders from its `info.json` files; `usual` (`~/Dropbox`) only when there is
/// no readable info.json and no Dropbox was found otherwise.
fn dropbox(infos: impl IntoIterator<Item = PathBuf>, usual: Option<PathBuf>, found: &mut Found) {
    let mut known = false;
    for info in infos {
        let Some(folders) = dropbox_info(&info) else { continue };
        known = true;
        for (dir, account) in folders {
            found.add(Provider::Dropbox, dir, account);
        }
    }
    if !known
        && !found.has(Provider::Dropbox)
        && let Some(dir) = usual
    {
        found.add(Provider::Dropbox, dir, None);
    }
}

/// The folders in Dropbox's `info.json` (`{"personal": {"path": …}, "business": {"path": …}}`)
/// with the team's name, if any; None without a readable file.
fn dropbox_info(file: &Path) -> Option<Vec<(PathBuf, Option<String>)>> {
    let text = fs::read_to_string(file).ok()?;
    let json: serde_json::Value = match serde_json::from_str(&text) {
        Ok(json) => json,
        Err(e) => {
            log::info!("Could not read {}: {e}", file.display());
            return None;
        }
    };
    let folders = ["personal", "business"]
        .into_iter()
        .filter_map(|kind| {
            let entry = json.get(kind)?;
            let dir = entry.get("path")?.as_str()?;
            // Dropbox doesn't write the team's name so far; use it should it ever appear.
            let team = || ["team_name", "name", "display_name"].iter().find_map(|k| entry.get(k)?.as_str());
            let account = if kind == "business" { team().map(str::to_string) } else { None };
            Some((PathBuf::from(dir), account))
        })
        .collect();
    Some(folders)
}

/// `~/Google Drive` (Backup and Sync, Linux clients), or the My Drive inside it. Only when no
/// Google Drive was found otherwise: Google Drive for desktop leaves the old folder behind.
fn old_google_drive(home: &Path, found: &mut Found) {
    if found.has(Provider::GoogleDrive) {
        return;
    }
    let dir = home.join("Google Drive");
    found.add(Provider::GoogleDrive, my_drive(&dir).unwrap_or(dir), None);
}

/// The "My Drive" folder in `dir`, under whichever language's name it has.
fn my_drive(dir: &Path) -> Option<PathBuf> {
    MY_DRIVE.iter().map(|name| dir.join(name)).find(|d| d.is_dir())
}

/// The folders in `dir` with their names, sorted (the system lists them in any order).
fn folders_in(dir: &Path) -> Vec<(String, PathBuf)> {
    let Ok(entries) = fs::read_dir(dir) else { return Vec::new() };
    let mut folders: Vec<_> = entries
        .flatten()
        .map(|e| (e.file_name().to_string_lossy().into_owned(), e.path()))
        .filter(|(_, path)| path.is_dir())
        .collect();
    folders.sort();
    folders
}

/// True for the suffix of the personal OneDrive's folder ("OneDrive - Personal") when a work or
/// school account is signed in too. It is in Windows' language; one not listed here makes the
/// folder show as work or school.
fn is_personal(suffix: &str) -> bool {
    const PERSONAL: &[&str] = &[
        "Personal",
        "Persönlich",
        "Personnel",
        "Personale",
        "Pessoal",
        "Persoonlijk",
        "Personlig",
        "Henkilökohtainen",
        "Osobiste",
        "Osobní",
        "Личный",
        "個人用",
        "개인",
        "个人",
        "個人",
    ];
    let suffix = suffix.trim();
    PERSONAL.iter().any(|p| p.to_lowercase() == suffix.to_lowercase())
}

/// 1 for OneDrive's registry account "Business1", …; None for "Personal" and others.
fn business_number(key: &str) -> Option<u32> {
    let (kind, number) = (key.get(..8)?, key.get(8..)?);
    if kind.eq_ignore_ascii_case("Business") { number.parse().ok() } else { None }
}

/// The organisation in a work or school OneDrive's folder name ("OneDrive - Contoso").
fn org_of(dir: &Path) -> Option<String> {
    let org = dir.file_name()?.to_str()?.strip_prefix("OneDrive - ")?.trim();
    (!org.is_empty()).then(|| org.to_string())
}

/// The drives in `roots` that `check` approves within `wait`, in their order. Each one is
/// asked on a thread of its own, because a network drive that isn't connected can take many
/// seconds to answer; such a thread is left to finish by itself.
#[cfg_attr(not(windows), allow(dead_code))]
fn answering_drives(roots: Vec<PathBuf>, wait: Duration, check: fn(&Path) -> bool) -> Vec<PathBuf> {
    let deadline = Instant::now() + wait;
    let (tx, rx) = mpsc::channel();
    for (i, root) in roots.iter().enumerate() {
        let (tx, root) = (tx.clone(), root.clone());
        let _ = thread::Builder::new().name("drive check".into()).spawn(move || {
            let _ = tx.send((i, check(&root)));
        });
    }
    drop(tx);
    let mut answers = vec![None; roots.len()];
    // Ends when every drive answered (all senders are gone) or at the deadline.
    while let Ok((i, ok)) = rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
        answers[i] = Some(ok);
    }
    roots
        .into_iter()
        .zip(answers)
        .filter_map(|(root, answer)| {
            if answer.is_none() {
                log::info!("{} did not answer within {wait:?}; not looking for Google Drive there", root.display());
            }
            (answer == Some(true)).then_some(root)
        })
        .collect()
}

#[cfg(windows)]
mod win {
    use std::os::windows::ffi::OsStrExt;
    use std::path::{Path, PathBuf};

    use windows_sys::Win32::Storage::FileSystem::{GetDriveTypeW, GetLogicalDrives};
    use winreg::RegKey;
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
    use winreg::types::FromRegValue;

    use super::{DRIVE_WAIT, answering_drives, my_drive};

    // GetDriveTypeW's answers (windows-sys keeps them in Win32::System::WindowsProgramming,
    // a feature this crate doesn't enable).
    const DRIVE_NO_ROOT_DIR: u32 = 1;
    const DRIVE_REMOVABLE: u32 = 2;
    const DRIVE_CDROM: u32 = 5;

    /// OneDrive's accounts: key name ("Personal", "Business1"), `UserFolder`, `DisplayName`.
    pub fn onedrive_accounts() -> Vec<(String, PathBuf, Option<String>)> {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let Ok(accounts) = hkcu.open_subkey(r"Software\Microsoft\OneDrive\Accounts") else {
            return Vec::new();
        };
        accounts
            .enum_keys()
            .flatten()
            .filter_map(|name| {
                let key = accounts.open_subkey(&name).ok()?;
                let folder: String = key.get_value("UserFolder").ok()?;
                let display: Option<String> = key.get_value("DisplayName").ok();
                let folder = folder.trim();
                (!folder.is_empty()).then(|| (name.clone(), PathBuf::from(folder), display))
            })
            .collect()
    }

    /// The folders registered as sync roots, of every user (the value's name is the user's
    /// SID): comparing paths is enough to tell whether a folder found is one of them.
    pub fn sync_roots() -> Vec<PathBuf> {
        let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
        let Ok(manager) = hklm.open_subkey(r"SOFTWARE\Microsoft\Windows\CurrentVersion\Explorer\SyncRootManager")
        else {
            return Vec::new();
        };
        let mut roots = Vec::new();
        for name in manager.enum_keys().flatten() {
            let Ok(users) = manager.open_subkey(format!(r"{name}\UserSyncRoots")) else { continue };
            for (_, value) in users.enum_values().flatten() {
                if let Ok(path) = String::from_reg_value(&value)
                    && !path.trim().is_empty()
                {
                    roots.push(PathBuf::from(path.trim()));
                }
            }
        }
        roots
    }

    /// Drives that may be Google Drive's (`G:\`): with a My Drive folder, answering in time.
    pub fn drive_roots() -> Vec<PathBuf> {
        // SAFETY: takes no arguments; returns a bit mask of the drive letters in use.
        let mask = unsafe { GetLogicalDrives() };
        // From C: on: A: and B: are floppy drives.
        let roots = (2..26u8)
            .filter(|i| mask & (1u32 << i) != 0)
            .map(|i| PathBuf::from(format!("{}:\\", char::from(b'A' + i))))
            .collect();
        // Even the drive's type is asked on the drive's own thread: nothing here may hang.
        answering_drives(roots, DRIVE_WAIT, |root| may_be_google_drive(root) && my_drive(root).is_some())
    }

    /// False for USB sticks, discs and letters without a drive: Google Drive is none of these.
    fn may_be_google_drive(root: &Path) -> bool {
        let wide: Vec<u16> = root.as_os_str().encode_wide().chain(Some(0)).collect();
        // SAFETY: `wide` is a NUL-terminated UTF-16 string that lives through the call.
        let kind = unsafe { GetDriveTypeW(wide.as_ptr()) };
        !matches!(kind, DRIVE_NO_ROOT_DIR | DRIVE_REMOVABLE | DRIVE_CDROM)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mkdirs(base: &Path, dirs: &[&str]) {
        fs::create_dir_all(base).unwrap();
        for dir in dirs {
            fs::create_dir_all(base.join(dir)).unwrap();
        }
    }

    fn vars(pairs: &[(&str, PathBuf)]) -> HashMap<String, String> {
        pairs.iter().map(|(name, value)| (name.to_string(), value.to_string_lossy().into_owned())).collect()
    }

    fn found(roots: &[CloudRoot]) -> Vec<(Provider, PathBuf, Option<&str>)> {
        roots.iter().map(|r| (r.provider, r.path.clone(), r.account.as_deref())).collect()
    }

    /// Windows with OneDrive and Dropbox installed.
    fn windows_probe(home: &Path) -> Probe {
        let installed = vec![Provider::OneDrive, Provider::Dropbox];
        Probe { windows: true, home: home.to_path_buf(), installed, ..Probe::default() }
    }

    fn account(key: &str, dir: PathBuf, name: Option<&str>) -> (String, PathBuf, Option<String>) {
        (key.to_string(), dir, name.map(str::to_string))
    }

    #[test]
    fn windows_onedrive_variables_and_registry_collapse() {
        use Provider::*;
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("Anna");
        let dirs =
            ["OneDrive", "OneDrive - Contoso", "OneDrive - Fabrikam", "OneDrive - Tailspin", "OneDrive - Old School"];
        mkdirs(&home, &dirs);
        let mut probe = windows_probe(&home);
        probe.vars = vars(&[
            ("OneDriveConsumer", home.join("OneDrive")),
            ("OneDrive", home.join("OneDrive")),
            ("OneDriveCommercial", PathBuf::from(format!("{}/", home.join("OneDrive - Contoso").display()))),
        ]);
        probe.onedrive_accounts = vec![
            account("Business10", home.join("OneDrive - Tailspin"), Some("Tailspin Toys")),
            account("Business2", home.join("OneDrive - Fabrikam"), Some("Fabrikam")),
            account("Personal", home.join(".").join("OneDrive"), Some("Personal")),
            account("Business1", home.join("OneDrive - Contoso"), Some("Contoso")),
            account("Business3", home.join("OneDrive - Gone"), Some("Gone")),
        ];
        assert_eq!(
            found(&detect_with(&probe)),
            [
                (OneDrive, home.join("OneDrive"), None),
                // Found by the variable first, named by the registry.
                (OneDriveBusiness, home.join("OneDrive - Contoso"), Some("Contoso")),
                (OneDriveBusiness, home.join("OneDrive - Fabrikam"), Some("Fabrikam")),
                (OneDriveBusiness, home.join("OneDrive - Tailspin"), Some("Tailspin Toys")),
            ],
            "each folder once; \"Old School\" is a leftover the records don't name"
        );
    }

    #[test]
    fn onedrive_variable_naming_the_work_account_is_work_only() {
        use Provider::*;
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("Anna");
        let work = home.join("OneDrive - Contoso");
        mkdirs(&home, &["OneDrive - Contoso", "OneDrive"]); // "OneDrive": left from an old account
        let cases: [(&str, Vec<_>, Option<&str>); 3] = [
            ("variables only", vec![], Some("Contoso")),
            (
                "with its registry account",
                vec![account("Business1", work.clone(), Some("Contoso Ltd"))],
                Some("Contoso Ltd"),
            ),
            ("registry without a name", vec![account("Business1", work.clone(), None)], Some("Contoso")),
        ];
        for (case, accounts, name) in cases {
            let mut probe = windows_probe(&home);
            probe.vars = vars(&[("OneDrive", work.clone()), ("OneDriveCommercial", work.clone())]);
            probe.onedrive_accounts = accounts;
            assert_eq!(found(&detect_with(&probe)), [(OneDriveBusiness, work.clone(), name)], "{case}");
        }
    }

    #[test]
    fn windows_usual_folders_without_records() {
        use Provider::*;
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("Anna");
        let dirs = [
            "OneDrive - Personal",
            "OneDrive - Contoso",
            "OneDrive - Persönlich", // the personal one, in German
            "Dropbox",
            "iCloudDrive",
            "Google Drive/Mon Drive",
            "Documents",
        ];
        mkdirs(&home, &dirs);
        let expected = [
            (OneDrive, home.join("OneDrive - Personal"), None),
            (OneDrive, home.join("OneDrive - Persönlich"), None),
            (OneDriveBusiness, home.join("OneDrive - Contoso"), Some("Contoso")),
            (Dropbox, home.join("Dropbox"), None),
            // Also Google Drive for desktop streaming into a folder instead of a drive letter.
            (GoogleDrive, home.join("Google Drive").join("Mon Drive"), None),
            (ICloud, home.join("iCloudDrive"), None),
        ];
        assert_eq!(found(&detect_with(&windows_probe(&home))), expected);

        // Without a home folder from the system, USERPROFILE says where it is.
        let probe =
            Probe { home: PathBuf::new(), vars: vars(&[("USERPROFILE", home.clone())]), ..windows_probe(&home) };
        assert_eq!(found(&detect_with(&probe)), expected);

        // The plain "OneDrive" folder is the personal one.
        let other = tmp.path().join("Ben");
        mkdirs(&other, &["OneDrive", "Google Drive"]);
        assert_eq!(
            found(&detect_with(&windows_probe(&other))),
            [(OneDrive, other.join("OneDrive"), None), (GoogleDrive, other.join("Google Drive"), None)]
        );
    }

    /// Uninstalling OneDrive or Dropbox leaves their records and folders behind; a study folder
    /// moved there would no longer sync.
    #[test]
    fn records_of_an_uninstalled_app_are_not_trusted() {
        use Provider::*;
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("Anna");
        let local = tmp.path().join("Local");
        mkdirs(&home, &["OneDrive", "OneDrive - Contoso", "Dropbox", "iCloudDrive"]);
        mkdirs(&local, &["Dropbox"]);
        let info = serde_json::json!({ "personal": { "path": home.join("Dropbox") } });
        fs::write(local.join("Dropbox").join("info.json"), info.to_string()).unwrap();
        let mut probe = windows_probe(&home);
        probe.installed = Vec::new();
        probe.vars = vars(&[("OneDrive", home.join("OneDrive")), ("LOCALAPPDATA", local.clone())]);
        probe.onedrive_accounts = vec![
            account("Personal", home.join("OneDrive"), None),
            account("Business1", home.join("OneDrive - Contoso"), Some("Contoso")),
        ];
        assert_eq!(found(&detect_with(&probe)), [(ICloud, home.join("iCloudDrive"), None)]);

        // Without records either: the usual folders don't count.
        let bare = Probe { vars: HashMap::new(), onedrive_accounts: Vec::new(), ..probe.clone() };
        assert_eq!(found(&detect_with(&bare)), [(ICloud, home.join("iCloudDrive"), None)]);

        // A folder Windows lists as a sync root is synced, wherever the program is.
        probe.sync_roots = vec![PathBuf::from(format!("{}/", home.join("OneDrive - Contoso").display()))];
        assert_eq!(
            found(&detect_with(&probe)),
            [
                (OneDriveBusiness, home.join("OneDrive - Contoso"), Some("Contoso")),
                (ICloud, home.join("iCloudDrive"), None)
            ]
        );

        // Only Dropbox installed.
        probe.sync_roots = Vec::new();
        probe.installed = vec![Dropbox];
        assert_eq!(
            found(&detect_with(&probe)),
            [(Dropbox, home.join("Dropbox"), None), (ICloud, home.join("iCloudDrive"), None)]
        );

        // Elsewhere the apps' programs aren't looked for.
        let linux = Probe { windows: false, ..bare };
        assert_eq!(
            found(&detect_with(&linux)),
            [(OneDrive, home.join("OneDrive"), None), (Dropbox, home.join("Dropbox"), None)]
        );
    }

    #[test]
    fn where_the_programs_are() {
        use Provider::*;
        let probe = vars(&[
            ("LOCALAPPDATA", PathBuf::from("/u/Local")),
            ("ProgramFiles", PathBuf::from("/pf")),
            ("ProgramFiles(x86)", PathBuf::from(" ")),
            ("APPDATA", PathBuf::from("/u/Roaming")),
        ]);
        let p = |path: &str| path.split('/').fold(PathBuf::from("/"), |full, part| full.join(part));
        assert_eq!(
            programs(&probe),
            [
                (OneDrive, p("u/Local/Microsoft/OneDrive/OneDrive.exe")),
                (Dropbox, p("u/Local/Dropbox/Client/Dropbox.exe")),
                (OneDrive, p("pf/Microsoft OneDrive/OneDrive.exe")),
                (Dropbox, p("pf/Dropbox/Client/Dropbox.exe")),
                (Dropbox, p("u/Roaming/Dropbox/bin/Dropbox.exe")),
            ]
        );
        assert_eq!(programs(&HashMap::new()), []);
    }

    #[test]
    fn missing_and_relative_folders_are_skipped() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("Anna");
        let gone = tmp.path().join("gone");
        let appdata = tmp.path().join("AppData").join("Roaming");
        mkdirs(&home, &[]);
        mkdirs(tmp.path(), &["AppData/Roaming/Dropbox", "G/Shared drives"]);
        let info = serde_json::json!({ "personal": { "path": gone.join("Dropbox") } });
        fs::write(appdata.join("Dropbox").join("info.json"), info.to_string()).unwrap();
        let mut probe = windows_probe(&home);
        probe.vars = vars(&[
            ("OneDrive", gone.join("OneDrive")),
            ("OneDriveConsumer", gone.join("OneDrive")),
            // Relative: would be the current folder, which exists.
            ("OneDriveCommercial", PathBuf::from(".")),
            ("APPDATA", appdata),
            ("LOCALAPPDATA", gone.join("Local")),
        ]);
        probe.onedrive_accounts =
            vec![account("Personal", gone.join("Personal"), None), account("Business1", PathBuf::new(), None)];
        probe.drive_roots = vec![tmp.path().join("G"), gone.join("H")];
        assert_eq!(detect_with(&probe), Vec::new());

        // Nothing known at all (no home folder): nothing found, nothing relative looked at.
        for probe in
            [Probe::default(), Probe { windows: true, ..Probe::default() }, Probe { macos: true, ..Probe::default() }]
        {
            assert_eq!(detect_with(&probe), Vec::new(), "{probe:?}");
        }
    }

    #[test]
    fn dropbox_info_json() {
        use Provider::Dropbox;
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("Anna");
        let (personal, business) =
            (tmp.path().join("D").join("Dropbox"), tmp.path().join("D").join("Dropbox (Contoso)"));
        let (roaming, local) = (tmp.path().join("Roaming"), tmp.path().join("Local"));
        mkdirs(&home, &["Dropbox"]); // left behind by an earlier installation
        mkdirs(tmp.path(), &["D/Dropbox", "D/Dropbox (Contoso)", "Roaming/Dropbox", "Local/Dropbox"]);
        let mut probe = windows_probe(&home);
        probe.vars = vars(&[("APPDATA", roaming.clone()), ("LOCALAPPDATA", local.clone())]);
        let write = |dir: &Path, text: &str| fs::write(dir.join("Dropbox").join("info.json"), text).unwrap();

        // The real format: more keys than the path, no team name.
        let both = serde_json::json!({
            "personal": { "path": personal, "host": 1234567890u64, "is_team": false, "subscription_type": "Basic" },
            "business": { "path": business, "host": 987654321u64, "is_team": true, "subscription_type": "Business" },
        });
        write(&roaming, &both.to_string());
        write(&local, &serde_json::json!({ "personal": { "path": format!("{}/", personal.display()) } }).to_string());
        assert_eq!(found(&detect_with(&probe)), [(Dropbox, personal.clone(), None), (Dropbox, business.clone(), None)]);

        // A team's name, should Dropbox ever write one.
        let named = serde_json::json!({ "business": { "path": business, "team_name": "Contoso" } });
        write(&roaming, &named.to_string());
        assert_eq!(
            found(&detect_with(&probe)),
            [(Dropbox, business.clone(), Some("Contoso")), (Dropbox, personal.clone(), None)]
        );

        // Unreadable info.json files: the usual folder.
        write(&roaming, "{ not json");
        write(&local, "");
        assert_eq!(found(&detect_with(&probe)), [(Dropbox, home.join("Dropbox"), None)]);
    }

    #[test]
    fn google_drive_on_drive_letters_and_mirrored() {
        use Provider::GoogleDrive;
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("Anna");
        let (d, g, h) = (tmp.path().join("D"), tmp.path().join("G"), tmp.path().join("H"));
        mkdirs(tmp.path(), &["D/Games", "G/Meine Ablage", "G/Shared drives", "H/マイドライブ"]);
        mkdirs(&home, &["My Drive", "Google Drive"]); // mirrored, and the old Backup and Sync's
        let probe = Probe { drive_roots: vec![d, g.clone(), h.clone()], ..windows_probe(&home) };
        assert_eq!(
            found(&detect_with(&probe)),
            [
                (GoogleDrive, g.join("Meine Ablage"), None),
                (GoogleDrive, h.join("マイドライブ"), None),
                (GoogleDrive, home.join("My Drive"), None),
            ]
        );

        // Mirrored in another language.
        let other = tmp.path().join("Ben");
        mkdirs(&other, &["Mi unidad"]);
        assert_eq!(found(&detect_with(&windows_probe(&other))), [(GoogleDrive, other.join("Mi unidad"), None)]);

        // Every language's name is found.
        for (i, &name) in MY_DRIVE.iter().enumerate() {
            let root = tmp.path().join(format!("drive{i}"));
            mkdirs(&root, &[name]);
            assert_eq!(my_drive(&root), Some(root.join(name)), "{name}");
        }
        assert_eq!(my_drive(&tmp.path().join("D")), None);
    }

    #[test]
    fn macos_cloud_storage() {
        use Provider::*;
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("anna");
        let storage = home.join("Library").join("CloudStorage");
        mkdirs(
            &storage,
            &[
                "OneDrive-Personal",
                "OneDrive-Contoso",
                "OneDrive-SharedLibraries-Contoso",
                "Dropbox",
                "Dropbox-Contoso",
                "GoogleDrive-anna@gmail.com/My Drive",
                "GoogleDrive-anna@gmail.com/Other computers",
                "GoogleDrive-ben@uni.de/Meine Ablage",
                "GoogleDrive-gone@example.com",
            ],
        );
        mkdirs(&home, &["Library/Mobile Documents/com~apple~CloudDocs", ".dropbox", "Dropbox"]);
        let info = serde_json::json!({ "personal": { "path": storage.join("Dropbox") } });
        fs::write(home.join(".dropbox").join("info.json"), info.to_string()).unwrap();
        fs::write(storage.join("notes.txt"), "not a folder").unwrap();
        let probe = Probe { macos: true, home: home.clone(), ..Probe::default() };
        assert_eq!(
            found(&detect_with(&probe)),
            [
                (OneDrive, storage.join("OneDrive-Personal"), None),
                (OneDriveBusiness, storage.join("OneDrive-Contoso"), Some("Contoso")),
                (Dropbox, storage.join("Dropbox"), None),
                (Dropbox, storage.join("Dropbox-Contoso"), Some("Contoso")),
                (GoogleDrive, storage.join("GoogleDrive-anna@gmail.com").join("My Drive"), Some("anna@gmail.com")),
                (GoogleDrive, storage.join("GoogleDrive-ben@uni.de").join("Meine Ablage"), Some("ben@uni.de")),
                (ICloud, home.join("Library").join("Mobile Documents").join("com~apple~CloudDocs"), None),
            ]
        );

        // An older Dropbox: only `~/Dropbox`.
        let other = tmp.path().join("ben");
        mkdirs(&other, &["Dropbox", "Library"]);
        let probe = Probe { macos: true, home: other.clone(), ..Probe::default() };
        assert_eq!(found(&detect_with(&probe)), [(Dropbox, other.join("Dropbox"), None)]);
    }

    #[test]
    fn linux_folders() {
        use Provider::*;
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("anna");
        let elsewhere = tmp.path().join("data").join("Dropbox");
        mkdirs(&home, &[".dropbox", "Dropbox", "OneDrive", "Google Drive/My Drive", "iCloudDrive"]);
        mkdirs(tmp.path(), &["data/Dropbox"]);
        let info = serde_json::json!({ "personal": { "path": elsewhere } });
        fs::write(home.join(".dropbox").join("info.json"), info.to_string()).unwrap();
        let probe = Probe { home: home.clone(), ..Probe::default() };
        assert_eq!(
            found(&detect_with(&probe)),
            [
                (OneDrive, home.join("OneDrive"), None),
                (Dropbox, elsewhere, None),
                (GoogleDrive, home.join("Google Drive").join("My Drive"), None),
            ],
            "Dropbox where its info.json says, not the leftover ~/Dropbox; no iCloud on Linux"
        );

        let other = tmp.path().join("ben");
        mkdirs(&other, &["Dropbox", "Google Drive"]);
        let probe = Probe { home: other.clone(), ..Probe::default() };
        assert_eq!(
            found(&detect_with(&probe)),
            [(Dropbox, other.join("Dropbox"), None), (GoogleDrive, other.join("Google Drive"), None)]
        );
    }

    #[test]
    fn provider_of_picks_the_innermost() {
        use Provider::*;
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("Anna");
        mkdirs(&home, &["OneDrive/Dropbox/Studium", "OneDrive2", "G/My Drive", "Documents"]);
        fs::write(home.join("OneDrive").join("Skript.pdf"), "pdf").unwrap();
        let root = |provider, path: PathBuf| CloudRoot { provider, path, account: None };
        let roots = [
            root(Dropbox, home.join("OneDrive").join("Dropbox")),
            root(OneDrive, home.join("OneDrive")),
            root(GoogleDrive, home.join("G").join("My Drive")),
        ];
        let cases = [
            (home.join("OneDrive").join("Skript.pdf"), Some(OneDrive)),
            (home.join("OneDrive"), Some(OneDrive)),
            (home.join("OneDrive").join("Neu").join("x.pdf"), Some(OneDrive)), // doesn't exist yet
            (home.join("OneDrive").join("Dropbox").join("Studium"), Some(Dropbox)),
            (home.join("OneDrive").join("Dropbox").join("Studium").join("new.pdf"), Some(Dropbox)),
            (home.join("OneDrive").join("Dropbox").join("..").join("Skript.pdf"), Some(OneDrive)),
            (home.join("G").join("My Drive").join("Studium"), Some(GoogleDrive)),
            (home.join("OneDrive2").join("x.pdf"), None),
            (home.join("Documents"), None),
            (home.join("G"), None),
        ];
        for (path, expected) in cases {
            assert_eq!(provider_of(&path, &roots).map(|r| r.provider), expected, "{}", path.display());
        }
        assert_eq!(provider_of(&home.join("OneDrive"), &[]), None);

        // The same folder twice: the first one counts.
        let twice = [root(Other, home.join("OneDrive")), root(OneDrive, home.join("OneDrive"))];
        assert_eq!(provider_of(&home.join("OneDrive").join("Skript.pdf"), &twice).map(|r| r.provider), Some(Other));
    }

    /// Like macOS's `/var` → `/private/var` (and 8.3 names on Windows): a folder that doesn't
    /// exist yet is still in the cloud folder when that is spelled another way.
    #[cfg(unix)]
    #[test]
    fn provider_of_a_new_folder_through_a_symlink() {
        let tmp = tempfile::tempdir().unwrap();
        mkdirs(tmp.path(), &["real/OneDrive"]);
        let link = tmp.path().join("link");
        std::os::unix::fs::symlink(tmp.path().join("real"), &link).unwrap();
        let roots = [CloudRoot { provider: Provider::OneDrive, path: link.join("OneDrive"), account: None }];
        for path in [
            link.join("OneDrive").join("Studium").join("Modul"),
            tmp.path().join("real").join("OneDrive").join("Studium"),
        ] {
            assert_eq!(provider_of(&path, &roots).map(|r| r.provider), Some(Provider::OneDrive), "{}", path.display());
        }
        assert_eq!(provider_of(&link.join("Studium"), &roots), None);
    }

    #[test]
    fn registry_account_names() {
        let cases = [
            ("Business1", Some(1)),
            ("business2", Some(2)),
            ("Business10", Some(10)),
            ("Business", None),
            ("Personal", None),
            ("BusinessX", None),
            ("Busi", None),
        ];
        for (key, expected) in cases {
            assert_eq!(business_number(key), expected, "{key}");
        }
        for suffix in ["Personal", " personal ", "Persönlich", "PERSÖNLICH", "Personnel", "個人用"] {
            assert!(is_personal(suffix), "{suffix}");
        }
        for suffix in ["Contoso", "Personalabteilung", ""] {
            assert!(!is_personal(suffix), "{suffix}");
        }
        assert_eq!(org_of(Path::new("/h/OneDrive - Contoso GmbH")), Some("Contoso GmbH".to_string()));
        assert_eq!(org_of(Path::new("/h/OneDrive - ")), None);
        assert_eq!(org_of(Path::new("/h/OneDrive")), None);
    }

    #[test]
    fn slow_drives_are_left_out() {
        let check: fn(&Path) -> bool = |root| {
            let name = root.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            if name.starts_with("slow") {
                thread::sleep(Duration::from_secs(10));
            }
            !name.starts_with("empty")
        };
        let roots = ["slow", "fast", "empty", "fast2", "slow2"].map(PathBuf::from).to_vec();
        let start = Instant::now();
        let answered = answering_drives(roots, Duration::from_millis(500), check);
        assert_eq!(answered, ["fast", "fast2"].map(PathBuf::from));
        assert!(start.elapsed() < Duration::from_secs(5), "waited {:?}", start.elapsed());

        // When all answer, nobody waits for the deadline.
        let start = Instant::now();
        let answered =
            answering_drives(["a", "empty", "b"].map(PathBuf::from).to_vec(), Duration::from_secs(30), check);
        assert_eq!(answered, ["a", "b"].map(PathBuf::from));
        assert!(start.elapsed() < Duration::from_secs(5), "waited {:?}", start.elapsed());
        assert_eq!(answering_drives(Vec::new(), Duration::from_secs(30), check), Vec::<PathBuf>::new());
    }

    #[cfg(windows)]
    #[test]
    fn system_probe_and_detect_do_not_panic() {
        let start = Instant::now();
        let probe = Probe::system();
        assert!(start.elapsed() < Duration::from_secs(5), "took {:?}", start.elapsed());
        assert!(probe.windows && !probe.macos);
        assert!(probe.home.is_absolute(), "{}", probe.home.display());
        for root in &probe.drive_roots {
            assert!(my_drive(root).is_some(), "{}", root.display());
        }
        for root in detect() {
            assert!(root.path.is_dir(), "{}", root.path.display());
        }
    }

    #[cfg(windows)]
    #[test]
    fn same_folder_in_other_case_once() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("Anna");
        mkdirs(&home, &["OneDrive"]);
        let mut probe = windows_probe(&home);
        probe.vars = vars(&[("OneDrive", home.join("ONEDRIVE"))]);
        probe.onedrive_accounts = vec![account("Personal", home.join("onedrive"), None)];
        assert_eq!(found(&detect_with(&probe)), [(Provider::OneDrive, home.join("ONEDRIVE"), None)]);
    }
}
