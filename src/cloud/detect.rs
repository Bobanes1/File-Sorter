//! Finding the cloud folders on this computer: OneDrive (personal and work/school),
//! Dropbox, Google Drive and iCloud Drive.
//!
//! CONTRACT (scaffold; to be implemented):
//! - Windows:
//!   - OneDrive: env vars `OneDriveConsumer` / `OneDrive` (personal), `OneDriveCommercial`
//!     (work/school). Also the registry accounts given in `Probe::onedrive_accounts`:
//!     "Personal" → OneDrive, "Business1".. → OneDriveBusiness with their DisplayName as
//!     account. Fallback: `%USERPROFILE%\OneDrive`.
//!   - Dropbox: `%APPDATA%\Dropbox\info.json` or `%LOCALAPPDATA%\Dropbox\info.json`
//!     (`{"personal": {"path": …}, "business": {"path": …}}`). Fallback `~\Dropbox`.
//!   - Google Drive for desktop: a drive (`Probe::drive_roots`, e.g. `G:\`) with a "My Drive"
//!     folder under any of its localised names ("Meine Ablage", "Mon Drive", …), or
//!     `~\My Drive` / `~\Google Drive` (mirror mode).
//!   - iCloud: `~\iCloudDrive`.
//! - macOS: `~/Library/CloudStorage/OneDrive-Personal`, `OneDrive-<Org>`, `Dropbox`,
//!   `GoogleDrive-<email>/<My Drive>`; `~/Library/Mobile Documents/com~apple~CloudDocs`;
//!   `~/.dropbox/info.json`; `~/Dropbox`.
//! - Linux: `~/.dropbox/info.json`, `~/Dropbox`, `~/OneDrive`, `~/Google Drive`.
//! - Only existing folders are returned, each path once, in the order of [`Provider`].

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

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
    /// `APPDATA`, `LOCALAPPDATA`, `USERPROFILE`).
    pub vars: HashMap<String, String>,
    /// OneDrive accounts from the registry (Windows): key name ("Personal", "Business1"),
    /// its `UserFolder` and its `DisplayName`.
    pub onedrive_accounts: Vec<(String, PathBuf, Option<String>)>,
    /// Roots of drives that may be Google Drive (Windows: `G:\`), already checked to respond.
    pub drive_roots: Vec<PathBuf>,
}

impl Probe {
    /// The real computer.
    pub fn system() -> Probe {
        todo!("scaffold")
    }
}

/// The cloud folders on this computer.
pub fn detect() -> Vec<CloudRoot> {
    detect_with(&Probe::system())
}

/// The cloud folders `probe` describes (only folders that exist).
pub fn detect_with(probe: &Probe) -> Vec<CloudRoot> {
    let _ = probe;
    todo!("scaffold")
}

/// The cloud folder `path` is in (or is), if any.
pub fn provider_of(path: &Path, roots: &[CloudRoot]) -> Option<CloudRoot> {
    let _ = (path, roots);
    todo!("scaffold")
}
