//! Look for a new version of File Sorter on GitHub and, on Windows, install it.
//!
//! The release is read with the system's `curl` (part of Windows 10 and 11), so File
//! Sorter needs no network code of its own. A new exe is only used if its size and its
//! SHA-256 checksum are exactly what GitHub lists for the release. Installing renames the
//! running exe to `FileSorter.old.exe` (Windows allows that), moves the new one into its
//! place and, if anything fails, puts the old one back; the old file is removed at the
//! next start.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Deserialize;
use sha2::{Digest, Sha256};

const LATEST: &str = "https://api.github.com/repos/Bobanes1/File-Sorter/releases/latest";
const ASSET: &str = "FileSorter.exe";
/// Name of the previous exe after an update, removed at the next start.
const OLD_EXE: &str = "FileSorter.old.exe";
/// The new exe while it is put next to the running one.
const NEW_EXE: &str = "FileSorter.new.exe";

#[derive(Debug, Clone, PartialEq)]
pub struct Release {
    /// "0.10" (without the "v").
    pub version: String,
    /// The release's page, for reading what is new.
    pub page: String,
    pub download: Option<Download>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Download {
    pub url: String,
    pub size: u64,
    /// Lower-case hex.
    pub sha256: String,
}

#[derive(Deserialize)]
struct ApiRelease {
    tag_name: String,
    html_url: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    assets: Vec<ApiAsset>,
}

#[derive(Deserialize)]
struct ApiAsset {
    name: String,
    browser_download_url: String,
    size: u64,
    #[serde(default)]
    digest: Option<String>,
}

/// Version numbers of "v0.10.1" → [0, 10, 1]; `None` if it isn't one.
fn numbers(version: &str) -> Option<Vec<u32>> {
    let version = version.trim().trim_start_matches(['v', 'V']);
    version.split('.').map(|part| part.parse().ok()).collect()
}

/// True if `candidate` is a later version than `current` ("0.10" > "0.9", "0.9.1" > "0.9").
pub fn is_newer(candidate: &str, current: &str) -> bool {
    match (numbers(candidate), numbers(current)) {
        (Some(mut a), Some(mut b)) => {
            let len = a.len().max(b.len());
            a.resize(len, 0);
            b.resize(len, 0);
            a > b
        }
        _ => false,
    }
}

fn parse_release(json: &str) -> Result<Release, String> {
    let api: ApiRelease = serde_json::from_str(json).map_err(|e| format!("unexpected answer from GitHub: {e}"))?;
    if api.draft || api.prerelease {
        return Err("no published release".into());
    }
    let download = api.assets.into_iter().find(|a| a.name.eq_ignore_ascii_case(ASSET)).and_then(|a| {
        let sha256 = a.digest?.strip_prefix("sha256:")?.to_lowercase();
        (sha256.len() == 64).then_some(Download { url: a.browser_download_url, size: a.size, sha256 })
    });
    Ok(Release { version: api.tag_name.trim_start_matches(['v', 'V']).to_string(), page: api.html_url, download })
}

/// `curl` without a console window popping up.
pub(crate) fn curl() -> Command {
    #[allow(unused_mut)]
    let mut command = Command::new("curl");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command.args(["--silent", "--show-error", "--fail", "--location", "--proto", "=https"]);
    command
}

/// The latest release, if it is newer than this version.
pub fn check() -> Result<Option<Release>, String> {
    let output = curl()
        .args(["--max-time", "20", "-H", "Accept: application/vnd.github+json", "-H", "User-Agent: File-Sorter"])
        .arg(LATEST)
        .output()
        .map_err(|e| format!("curl could not be started: {e}"))?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    let release = parse_release(&String::from_utf8_lossy(&output.stdout))?;
    Ok(is_newer(&release.version, env!("CARGO_PKG_VERSION")).then_some(release))
}

fn sha256_of(path: &Path) -> std::io::Result<String> {
    let mut file = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 256 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// True if `file` is exactly the download GitHub lists (size and checksum).
fn verified(file: &Path, download: &Download) -> Result<(), String> {
    let size = fs::metadata(file).map_err(|e| e.to_string())?.len();
    if size != download.size {
        return Err(format!("the download has {size} bytes instead of {}", download.size));
    }
    let hash = sha256_of(file).map_err(|e| e.to_string())?;
    if hash != download.sha256 {
        return Err("the download's checksum doesn't match".into());
    }
    Ok(())
}

/// Download the new exe into `dir` and check it. Returns where it is.
pub fn download(release: &Release, dir: &Path) -> Result<PathBuf, String> {
    let download = release.download.as_ref().ok_or("this release has no FileSorter.exe with a checksum")?;
    fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let file = dir.join(format!("FileSorter-{}.exe.part", release.version));
    let _ = fs::remove_file(&file);
    let status = curl()
        .args(["--max-time", "600", "-H", "User-Agent: File-Sorter", "--output"])
        .arg(&file)
        .arg(&download.url)
        .status()
        .map_err(|e| format!("curl could not be started: {e}"))?;
    if !status.success() {
        let _ = fs::remove_file(&file);
        return Err("the download failed".into());
    }
    if let Err(e) = verified(&file, download) {
        let _ = fs::remove_file(&file);
        return Err(e);
    }
    Ok(file)
}

/// Put the downloaded exe in place of `current` (the running one), keeping the old one as
/// `FileSorter.old.exe` until the next start. On failure everything stays as it was.
/// `new` must be in the same folder as `current`, so both steps are plain renames: the
/// program file is always either the whole old or the whole new exe.
pub fn swap(new: &Path, current: &Path) -> Result<(), String> {
    let old = current.with_file_name(OLD_EXE);
    if old.exists() {
        fs::remove_file(&old).map_err(|e| format!("{} is in the way: {e}", old.display()))?;
    }
    fs::rename(current, &old).map_err(|e| format!("the program file can't be replaced ({e})"))?;
    if let Err(e) = fs::rename(new, current) {
        // Put the old exe back; nothing is lost.
        return match fs::rename(&old, current) {
            Ok(()) => Err(format!("the new version could not be put in place ({e})")),
            Err(back) => Err(format!(
                "the new version could not be put in place ({e}), and the old one could not be renamed back ({back}): \
                 rename {} to {} by hand",
                old.display(),
                current.display()
            )),
        };
    }
    Ok(())
}

/// Download, check and install `release` (Windows). Afterwards the next start runs it.
pub fn install(release: &Release, state_dir: &Path) -> Result<(), String> {
    if !cfg!(windows) {
        return Err("installing is only done on Windows".into());
    }
    let current = std::env::current_exe().map_err(|e| e.to_string())?;
    let downloaded = download(release, state_dir)?;
    // Next to the program (maybe on another drive), checked again, then swapped by renaming.
    let new = current.with_file_name(NEW_EXE);
    let placed = fs::copy(&downloaded, &new).map_err(|e| format!("can't write next to the program ({e})"));
    let _ = fs::remove_file(&downloaded);
    let result = placed
        .and_then(|_| verified(&new, release.download.as_ref().ok_or("no download")?))
        .and_then(|_| swap(&new, &current));
    let _ = fs::remove_file(&new);
    result
}

/// Start the (new) exe again with the same arguments; it waits until this process is gone.
pub fn restart() -> std::io::Result<()> {
    let exe = std::env::current_exe()?;
    let args: Vec<String> = std::env::args().skip(1).filter(|a| a != "--autostart").collect();
    Command::new(exe).args(args).arg("--wait-for").arg(std::process::id().to_string()).spawn().map(drop)
}

/// Started by `restart`: wait (at most 20 s) until the previous process has ended.
pub fn wait_for(pid: u32) {
    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::CloseHandle;
        use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject};
        // SAFETY: plain Win32 calls; the handle is closed again.
        unsafe {
            let handle = OpenProcess(PROCESS_SYNCHRONIZE, 0, pid);
            if !handle.is_null() {
                WaitForSingleObject(handle, 20_000);
                CloseHandle(handle);
            }
        }
    }
    #[cfg(not(windows))]
    {
        let _ = pid;
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
}

/// Remove the exe left over from the last update.
pub fn clean_up() {
    if let Ok(current) = std::env::current_exe() {
        let old = current.with_file_name(OLD_EXE);
        if old.exists() && old != current {
            let _ = fs::remove_file(old);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_as_numbers() {
        assert!(is_newer("v0.10", "0.9.0"));
        assert!(is_newer("0.9.1", "0.9.0"));
        assert!(is_newer("1.0", "0.9.0"));
        assert!(!is_newer("v0.9", "0.9.0"));
        assert!(!is_newer("v0.8", "0.9.0"));
        assert!(!is_newer("nightly", "0.9.0"));
    }

    #[test]
    fn reads_the_release_and_its_checksum() {
        let json = r#"{"tag_name":"v0.10","html_url":"https://github.com/Bobanes1/File-Sorter/releases/tag/v0.10",
            "assets":[{"name":"FileSorter.exe","browser_download_url":"https://x/FileSorter.exe","size":5,
            "digest":"sha256:2CF24DBA5FB0A30E26E83B2AC5B9E29E1B161E5C1FA7425E73043362938B9824"}]}"#;
        let release = parse_release(json).unwrap();
        assert_eq!(release.version, "0.10");
        let download = release.download.unwrap();
        assert_eq!(download.sha256, "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824");
        let without = r#"{"tag_name":"v0.10","html_url":"u","assets":[{"name":"FileSorter.exe","browser_download_url":"x","size":5}]}"#;
        assert_eq!(parse_release(without).unwrap().download, None, "no checksum, no install");
        assert!(parse_release(r#"{"tag_name":"v1","html_url":"u","prerelease":true}"#).is_err());

        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("new.exe");
        fs::write(&file, "hello").unwrap();
        assert_eq!(verified(&file, &download), Ok(()));
        fs::write(&file, "hellO").unwrap();
        assert!(verified(&file, &download).is_err());
    }

    #[test]
    fn swapping_keeps_the_old_exe_until_it_works() {
        let tmp = tempfile::tempdir().unwrap();
        let current = tmp.path().join("FileSorter.exe");
        let new = tmp.path().join("new.exe");
        fs::write(&current, "old").unwrap();
        fs::write(&new, "new").unwrap();
        swap(&new, &current).unwrap();
        assert_eq!(fs::read_to_string(&current).unwrap(), "new");
        assert_eq!(fs::read_to_string(tmp.path().join(OLD_EXE)).unwrap(), "old");
        // A missing download changes nothing.
        let err = swap(&tmp.path().join("missing.exe"), &current);
        assert!(err.is_err());
        assert_eq!(fs::read_to_string(&current).unwrap(), "new");
        assert!(!tmp.path().join(OLD_EXE).exists() || fs::read_to_string(tmp.path().join(OLD_EXE)).unwrap() != "new");
    }
}
