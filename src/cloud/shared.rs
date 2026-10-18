//! Modules, keywords and file types shared between computers through a small file in the
//! study folder in the cloud.
//!
//! CONTRACT (scaffold; to be implemented):
//! - [`SHARED_FILE`] in the cloud study folder holds a [`Shared`] as JSON (pretty printed).
//! - Machine-specific settings (folders, language, startup, …) are never shared; a module's
//!   custom `folder` neither (written empty; kept per computer when applying).
//! - [`decide`] picks the step: the newest change wins.

use std::io;
use std::path::Path;
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

use crate::config::{Config, Course};

/// The file in the cloud study folder.
pub const SHARED_FILE: &str = ".file-sorter.json";
/// Version of the format written; files from newer versions are not touched.
pub const VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Shared {
    pub version: u32,
    /// Milliseconds since 1970.
    pub updated_at: i64,
    /// The computer that wrote it.
    pub updated_by: String,
    pub courses: Vec<Course>,
    pub extensions: Vec<String>,
    pub read_file_contents: bool,
}

impl Shared {
    pub fn from_config(config: &Config, updated_at: i64, updated_by: &str) -> Shared {
        let _ = (config, updated_at, updated_by);
        todo!("scaffold")
    }

    /// Same modules (in any order), keywords, file types and content reading as `config`.
    pub fn matches(&self, config: &Config) -> bool {
        let _ = config;
        todo!("scaffold")
    }

    /// Take over modules, keywords and file types; a module keeps its custom folder on this
    /// computer. Returns whether anything changed.
    pub fn apply_to(&self, config: &mut Config) -> bool {
        let _ = config;
        todo!("scaffold")
    }
}

/// The shared settings in `file`; `None` if missing, unreadable or from a newer version.
pub fn read(file: &Path) -> Option<Shared> {
    let _ = file;
    todo!("scaffold")
}

/// Write `shared` to `file` (through a temporary file).
pub fn write(file: &Path, shared: &Shared) -> io::Result<()> {
    let _ = (file, shared);
    todo!("scaffold")
}

/// When `file` last changed, to check for news cheaply.
pub fn modified(file: &Path) -> Option<SystemTime> {
    let _ = file;
    todo!("scaffold")
}

/// What to do to keep this computer and the shared file in step.
#[derive(Debug, Clone, PartialEq)]
pub enum Step {
    Nothing,
    /// Another computer changed them more recently: take theirs.
    Apply(Shared),
    /// This computer has changes (or there is no shared file yet): write ours.
    Write,
}

/// - `seen`: `updated_at` of the shared file this computer last read or wrote.
/// - `local_changed_at`: when the modules here were last changed (ms), if not shared yet.
/// - No shared file → `Write` if there are modules, else `Nothing`.
/// - Shared file equal to ours → `Nothing`.
/// - Written elsewhere since `seen` → `Apply`, unless our unshared change is newer → `Write`.
/// - Otherwise (ours differ, theirs not newer) → `Write`.
pub fn decide(config: &Config, seen: i64, local_changed_at: Option<i64>, remote: Option<&Shared>) -> Step {
    let _ = (config, seen, local_changed_at, remote);
    todo!("scaffold")
}
