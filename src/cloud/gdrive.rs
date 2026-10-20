//! Google Drive without the Google Drive app: the study folder is kept in sync with a folder
//! in Google Drive by rclone (an open-source sync tool: fetched once from its GitHub
//! releases and checked against its published checksum, or the one installed on Linux).
//!
//! Signing in happens in the browser (`rclone authorize`); the access is kept in File
//! Sorter's own `rclone.conf`, apart from any other rclone setup.
//!
//! Syncing goes both ways (`rclone bisync`) and loses nothing: a file deleted or overwritten
//! on one side is first put into a backup folder on that side (on this computer, and in
//! Drive's «File Sorter backup» folder), and when a file changed on both sides the newer
//! one keeps the name and the other is kept beside it as `name.conflict1`. If more than half
//! of the files would be deleted, nothing is done (as when the study folder was emptied by
//! mistake).

use std::fs;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// The rclone release fetched, with the SHA-256 of its downloads (from its `SHA256SUMS`).
pub const RCLONE_VERSION: &str = "1.75.1";
const SHA256_WINDOWS: &str = "200eb602c126d82aa38b51e0f6b9ae837473ff99b51278d3f6f837574c494d6e";
const SHA256_LINUX: &str = "982b5aa772841168f8e380f139e9e787b2a105403e32b94da8676a0e1c0a13ab";

/// The name of File Sorter's Drive in its `rclone.conf`.
const REMOTE: &str = "filesorter-gdrive";
/// Where deleted and overwritten files go in Drive (beside the study folder).
pub const DRIVE_BACKUP: &str = "File Sorter backup";
/// Backups older than this are removed.
const KEEP_BACKUPS_DAYS: u32 = 30;
/// Files never synced: the system's and programs' own, and unfinished downloads.
const FILTERS: &str = "- desktop.ini\n- Thumbs.db\n- .DS_Store\n- ~$*\n- .~lock.*#\n- *.crdownload\n- *.part\n- *.partial\n- *.tmp\n- .file-sorter*\n";

/// The Google Drive settings.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct DriveSettings {
    /// Signed in and syncing.
    pub on: bool,
    /// The folder in Drive the study folder is kept in sync with («Studium»).
    pub folder: String,
    /// The study folder and Drive folder last synced as a pair (`local|folder`); another pair
    /// starts with a first sync that merges both sides.
    pub synced_pair: String,
}

/// Where File Sorter keeps rclone and its files.
#[derive(Debug, Clone)]
pub struct Place {
    dir: PathBuf,
}

impl Place {
    pub fn new(state_dir: &Path) -> Place {
        Place { dir: state_dir.join("google-drive") }
    }

    fn conf(&self) -> PathBuf {
        self.dir.join("rclone.conf")
    }

    fn workdir(&self) -> PathBuf {
        self.dir.join("sync-state")
    }

    /// Where files deleted or overwritten on this computer are kept.
    pub fn backups(&self) -> PathBuf {
        self.dir.join("backup")
    }

    fn own_rclone(&self) -> PathBuf {
        self.dir.join(if cfg!(windows) { "rclone.exe" } else { "rclone" })
    }

    /// rclone, if there is one: File Sorter's own, or (Linux) the one installed.
    pub fn rclone(&self) -> Option<PathBuf> {
        let own = self.own_rclone();
        if own.is_file() {
            return Some(own);
        }
        if cfg!(windows) {
            return None;
        }
        let ok = command(Path::new("rclone")).arg("version").stdout(Stdio::null()).stderr(Stdio::null()).status();
        ok.is_ok_and(|s| s.success()).then(|| PathBuf::from("rclone"))
    }

    /// Signed in (File Sorter's Drive is in its `rclone.conf`).
    pub fn signed_in(&self) -> bool {
        fs::read_to_string(self.conf()).is_ok_and(|t| t.contains(&format!("[{REMOTE}]")) && t.contains("token"))
    }

    /// Fetch rclone (once), checking it against its published checksum.
    pub fn install(&self) -> Result<PathBuf, String> {
        if let Some(found) = self.rclone() {
            return Ok(found);
        }
        let (system, sha) = if cfg!(windows) { ("windows", SHA256_WINDOWS) } else { ("linux", SHA256_LINUX) };
        let name = format!("rclone-v{RCLONE_VERSION}-{system}-amd64");
        let url = format!("https://github.com/rclone/rclone/releases/download/v{RCLONE_VERSION}/{name}.zip");
        fs::create_dir_all(&self.dir).map_err(|e| e.to_string())?;
        let zip_path = self.dir.join("rclone-download.zip");
        let fetched = crate::update::curl()
            .args(["--max-time", "600", "--output"])
            .arg(&zip_path)
            .arg(&url)
            .output()
            .map_err(|e| format!("curl could not be started: {e}"))?;
        if !fetched.status.success() {
            let _ = fs::remove_file(&zip_path);
            return Err(String::from_utf8_lossy(&fetched.stderr).trim().to_string());
        }
        let result = (|| {
            let got = sha256_file(&zip_path).map_err(|e| e.to_string())?;
            if got != sha {
                return Err(format!("the download of rclone is not the published one (SHA-256 {got})"));
            }
            let file = fs::File::open(&zip_path).map_err(|e| e.to_string())?;
            let mut zip = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
            let exe = if cfg!(windows) { "rclone.exe" } else { "rclone" };
            let mut entry = zip.by_name(&format!("{name}/{exe}")).map_err(|e| e.to_string())?;
            let target = self.own_rclone();
            let partial = target.with_extension("partial");
            let mut out = fs::File::create(&partial).map_err(|e| e.to_string())?;
            std::io::copy(&mut entry, &mut out).map_err(|e| e.to_string())?;
            drop(out);
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(&partial, fs::Permissions::from_mode(0o755)).map_err(|e| e.to_string())?;
            }
            fs::rename(&partial, &target).map_err(|e| e.to_string())?;
            Ok(target)
        })();
        let _ = fs::remove_file(&zip_path);
        result
    }

    /// Start signing in: rclone opens the browser for the Google sign-in.
    pub fn sign_in(&self) -> Result<SignIn, String> {
        let rclone = self.rclone().ok_or("rclone is missing")?;
        fs::create_dir_all(&self.dir).map_err(|e| e.to_string())?;
        let mut child = command(&rclone)
            .args(["authorize", "drive"])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .stdin(Stdio::null())
            .spawn()
            .map_err(|e| e.to_string())?;
        // The link to open by hand (if no browser opened) is in what rclone says.
        let (link_tx, link) = channel();
        if let Some(err) = child.stderr.take() {
            std::thread::spawn(move || {
                for line in BufReader::new(err).lines().map_while(Result::ok) {
                    if let Some(at) = line.find("http://127.0.0.1:") {
                        let url = line[at..].split_whitespace().next().unwrap_or_default().to_string();
                        let _ = link_tx.send(url);
                    }
                }
            });
        }
        Ok(SignIn { child, link, place: self.clone(), started: Instant::now() })
    }

    /// Sign out: forget the access (the files stay on both sides).
    pub fn sign_out(&self) {
        let _ = fs::remove_file(self.conf());
        let _ = fs::remove_dir_all(self.workdir());
    }

    fn save_token(&self, rclone: &Path, token: &str) -> Result<(), String> {
        let out = command(rclone)
            .args(["config", "create", REMOTE, "drive", "scope=drive"])
            .arg(format!("token={token}"))
            .args(["--non-interactive", "--config"])
            .arg(self.conf())
            .output()
            .map_err(|e| e.to_string())?;
        if !out.status.success() || !self.signed_in() {
            return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
        }
        Ok(())
    }

    /// Sync `local` with `folder` in Drive, both ways. `first`: the first time for this pair,
    /// which merges both sides (the newer of two different files wins; the other is kept in
    /// the backup).
    pub fn sync(&self, local: &Path, folder: &str, first: bool) -> Result<Summary, String> {
        let rclone = self.rclone().ok_or("rclone is missing")?;
        if !local.is_dir() {
            return Err(format!("{} is not there", local.display()));
        }
        fs::create_dir_all(self.workdir()).map_err(|e| e.to_string())?;
        let filters = self.dir.join("filters.txt");
        fs::write(&filters, FILTERS).map_err(|e| e.to_string())?;
        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        let remote = format!("{REMOTE}:{}", folder.trim_matches('/'));
        let mut cmd = command(&rclone);
        cmd.arg("bisync")
            .arg(local)
            .arg(&remote)
            .arg("--config")
            .arg(self.conf())
            .arg("--workdir")
            .arg(self.workdir())
            .arg("--filters-file")
            .arg(&filters)
            .arg("--backup-dir1")
            .arg(self.backups().join(&today))
            .arg("--backup-dir2")
            .arg(format!("{REMOTE}:{DRIVE_BACKUP}/{today}"))
            .args(["--conflict-resolve", "newer", "--conflict-loser", "num"])
            .args(["--resilient", "--recover", "--max-lock", "2m", "--create-empty-src-dirs"])
            // Google Docs, Sheets and Slides have no file to sync.
            .args(["--drive-skip-gdocs", "--color", "NEVER", "-v"]);
        if first {
            cmd.args(["--resync", "--resync-mode", "newer"]);
        }
        let out = cmd.stdin(Stdio::null()).output().map_err(|e| e.to_string())?;
        let log = String::from_utf8_lossy(&out.stderr).into_owned();
        if !out.status.success() {
            return Err(explain_failure(&log));
        }
        Ok(Summary::read(&log))
    }

    /// Remove backups older than a month, here and in Drive.
    pub fn prune_backups(&self) {
        let cutoff = chrono::Local::now().date_naive() - chrono::Duration::days(KEEP_BACKUPS_DAYS as i64);
        if let Ok(entries) = fs::read_dir(self.backups()) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                if chrono::NaiveDate::parse_from_str(&name, "%Y-%m-%d").is_ok_and(|d| d < cutoff) {
                    let _ = fs::remove_dir_all(entry.path());
                }
            }
        }
        if let Some(rclone) = self.rclone() {
            let _ = command(&rclone)
                .args(["delete", "--min-age"])
                .arg(format!("{KEEP_BACKUPS_DAYS}d"))
                .arg("--rmdirs")
                .arg(format!("{REMOTE}:{DRIVE_BACKUP}"))
                .arg("--config")
                .arg(self.conf())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
    }
}

/// Signing in, waiting for the browser.
pub struct SignIn {
    child: Child,
    link: Receiver<String>,
    place: Place,
    started: Instant,
}

/// How long signing in may take before it is given up.
const SIGN_IN_TIME: Duration = Duration::from_secs(10 * 60);

impl SignIn {
    /// The sign-in page, to open by hand if the browser didn't open.
    pub fn link(&self) -> Option<String> {
        self.link.try_recv().ok()
    }

    /// `None` while waiting; then whether it worked.
    pub fn poll(&mut self) -> Option<Result<(), String>> {
        match self.child.try_wait() {
            Ok(None) if self.started.elapsed() < SIGN_IN_TIME => None,
            Ok(None) => {
                self.cancel();
                Some(Err("no sign-in within 10 minutes".into()))
            }
            Ok(Some(status)) => {
                let mut out = String::new();
                if let Some(mut stdout) = self.child.stdout.take() {
                    let _ = stdout.read_to_string(&mut out);
                }
                if !status.success() {
                    return Some(Err("signing in was not finished".into()));
                }
                let Some(token) = token_in(&out) else { return Some(Err("rclone gave no access".into())) };
                let rclone = self.place.rclone().unwrap_or_else(|| PathBuf::from("rclone"));
                Some(self.place.save_token(&rclone, &token))
            }
            Err(e) => Some(Err(e.to_string())),
        }
    }

    pub fn cancel(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for SignIn {
    fn drop(&mut self) {
        if matches!(self.child.try_wait(), Ok(None)) {
            self.cancel();
        }
    }
}

/// The access token rclone printed between its paste markers.
fn token_in(out: &str) -> Option<String> {
    let start = out.find("--->")? + 4;
    let end = out[start..].find("<---")? + start;
    let token = out[start..end].trim();
    (token.starts_with('{') && token.contains("access_token")).then(|| token.to_string())
}

/// What a sync did.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Summary {
    /// Files brought here from Drive, and sent there.
    pub down: usize,
    pub up: usize,
    /// Files changed on both sides (both kept).
    pub conflicts: usize,
}

impl Summary {
    fn read(log: &str) -> Summary {
        let mut summary = Summary::default();
        for line in log.lines() {
            if line.contains("Queue copy to Path2") {
                summary.up += 1;
            } else if line.contains("Queue copy to Path1") {
                summary.down += 1;
            }
            if line.contains("New or changed in both paths") {
                summary.conflicts += 1;
            }
        }
        summary
    }
}

/// A short reason from rclone's log.
fn explain_failure(log: &str) -> String {
    if log.contains("too many deletes") || log.contains("max-delete") {
        return "too many deletes".into();
    }
    if log.contains("couldn't fetch token") || log.contains("invalid_grant") || log.contains("oauth2") {
        return "sign-in expired".into();
    }
    if log.contains("must run --resync") || log.contains("--resync") {
        return "resync needed".into();
    }
    log.lines()
        .rfind(|l| l.contains("ERROR") || l.contains("Failed"))
        .map(|l| l.split(" : ").last().unwrap_or(l).trim().to_string())
        .unwrap_or_else(|| log.lines().last().unwrap_or("rclone failed").to_string())
}

fn command(program: &Path) -> Command {
    #[allow(unused_mut)]
    let mut command = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command
}

fn sha256_file(path: &Path) -> std::io::Result<String> {
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
    Ok(format!("{:x}", hasher.finalize()))
}

// ---------- the background worker ----------

/// What the worker is asked to do.
pub enum Job {
    /// Sync now (`first`: merge both sides, for a new pair).
    Sync { local: PathBuf, folder: String, first: bool },
    /// Fetch rclone.
    Install,
}

/// What it reports.
#[derive(Debug, Clone, PartialEq)]
pub enum Report {
    Installed(Result<(), String>),
    Synced { pair: String, result: Result<Summary, String> },
}

/// Syncing in the background, one job at a time.
pub struct Worker {
    tx: Sender<Job>,
    pub rx: Receiver<Report>,
    busy: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl Worker {
    pub fn start(place: Place, wake: impl Fn() + Send + 'static) -> Worker {
        let (tx, jobs) = channel::<Job>();
        let (report_tx, rx) = channel();
        let busy = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag = std::sync::Arc::clone(&busy);
        std::thread::spawn(move || {
            let mut pruned: Option<chrono::NaiveDate> = None;
            while let Ok(job) = jobs.recv() {
                flag.store(true, std::sync::atomic::Ordering::Relaxed);
                let report = match job {
                    Job::Install => Report::Installed(place.install().map(|_| ())),
                    Job::Sync { local, folder, first } => {
                        let pair = pair(&local, &folder);
                        let mut result = place.sync(&local, &folder, first);
                        // Interrupted earlier (power off, …): start over by merging both sides.
                        if matches!(&result, Err(e) if e == "resync needed") {
                            result = place.sync(&local, &folder, true);
                        }
                        let today = chrono::Local::now().date_naive();
                        if result.is_ok() && pruned != Some(today) {
                            place.prune_backups();
                            pruned = Some(today);
                        }
                        Report::Synced { pair, result }
                    }
                };
                flag.store(false, std::sync::atomic::Ordering::Relaxed);
                if report_tx.send(report).is_err() {
                    return;
                }
                wake();
            }
        });
        Worker { tx, rx, busy }
    }

    pub fn send(&self, job: Job) {
        self.busy.store(true, std::sync::atomic::Ordering::Relaxed);
        let _ = self.tx.send(job);
    }

    pub fn busy(&self) -> bool {
        self.busy.load(std::sync::atomic::Ordering::Relaxed)
    }
}

/// The study folder and Drive folder as a pair, to tell when either changed.
pub fn pair(local: &Path, folder: &str) -> String {
    format!("{}|{}", local.display(), folder.trim_matches('/'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_token_is_read_from_between_the_markers() {
        let out = "Paste the following into your remote machine --->\n{\"access_token\":\"ya29\",\"expiry\":\"x\"}\n<---End paste\n";
        assert_eq!(token_in(out).unwrap(), "{\"access_token\":\"ya29\",\"expiry\":\"x\"}");
        assert!(token_in("nothing").is_none());
    }

    #[test]
    fn failures_are_told_plainly() {
        assert_eq!(explain_failure("ERROR : too many deletes - aborting"), "too many deletes");
        assert_eq!(explain_failure("Bisync critical error: cannot find prior Path1 or Path2 listings, likely due to critical error on prior run\nMust run --resync to recover."), "resync needed");
    }

    /// Downloads rclone from its GitHub releases and checks it.
    #[test]
    #[ignore]
    fn fetches_and_checks_rclone() {
        let tmp = tempfile::tempdir().unwrap();
        let place = Place::new(tmp.path());
        let exe = place.install().unwrap();
        let out = command(&exe).arg("version").output().unwrap();
        assert!(String::from_utf8_lossy(&out.stdout).contains(RCLONE_VERSION));
    }

    /// With rclone (RCLONE=<path>, else it is fetched): two folders on this computer stand in
    /// for the computer and Drive; nothing is lost when both change, files are deleted, or a
    /// file changes on both sides.
    #[test]
    #[ignore]
    fn syncs_both_ways_without_losing_anything() {
        let tmp = tempfile::tempdir().unwrap();
        let place = Place::new(tmp.path());
        fs::create_dir_all(&place.dir).unwrap();
        match std::env::var("RCLONE") {
            Ok(rclone) => drop(fs::copy(rclone, place.own_rclone()).unwrap()),
            Err(_) => drop(place.install().unwrap()),
        }
        // A "drive" that is a folder here.
        let drive = tmp.path().join("drive");
        fs::create_dir_all(&drive).unwrap();
        fs::write(place.conf(), format!("[{REMOTE}]\ntype = alias\nremote = {}\ntoken = x\n", drive.display())).unwrap();
        let local = tmp.path().join("Studium");
        fs::create_dir_all(local.join("Statistik")).unwrap();
        fs::write(local.join("Statistik/a.pdf"), "a").unwrap();
        fs::create_dir_all(drive.join("Studium/Prog")).unwrap();
        fs::write(drive.join("Studium/Prog/b.pdf"), "b").unwrap();
        place.sync(&local, "Studium", true).unwrap();
        assert_eq!(fs::read_to_string(drive.join("Studium/Statistik/a.pdf")).unwrap(), "a");
        assert_eq!(fs::read_to_string(local.join("Prog/b.pdf")).unwrap(), "b");
        // Deleted in Drive: gone here too, but kept in the backup.
        fs::remove_file(drive.join("Studium/Prog/b.pdf")).unwrap();
        // Changed on both sides: both kept.
        fs::write(local.join("Statistik/a.pdf"), "here").unwrap();
        std::thread::sleep(Duration::from_millis(1100));
        fs::write(drive.join("Studium/Statistik/a.pdf"), "there").unwrap();
        let summary = place.sync(&local, "Studium", false).unwrap();
        assert_eq!(summary.conflicts, 1);
        assert!(!local.join("Prog/b.pdf").exists());
        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        assert_eq!(fs::read_to_string(place.backups().join(&today).join("Prog/b.pdf")).unwrap(), "b");
        assert_eq!(fs::read_to_string(local.join("Statistik/a.pdf")).unwrap(), "there", "the newer one");
        assert_eq!(fs::read_to_string(local.join("Statistik/a.pdf.conflict1")).unwrap(), "here");
        assert_eq!(fs::read_to_string(drive.join("Studium/Statistik/a.pdf.conflict1")).unwrap(), "here");
    }
}
