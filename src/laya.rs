//! Laya: an optional AI model (ConvAI Innovations, Apache 2.0, huggingface.co/convaiinnovations/laya)
//! that reads text in 100+ languages and answers typed questions with probabilities. File
//! Sorter asks it which module a document belongs to when the keywords don't decide, and
//! which dates in a document are deadlines or exams.
//!
//! Laya runs in Python (PyTorch). Setting it up (optional, a few GB) fetches `uv` (Astral's
//! Python installer: a fixed release, checked against its SHA-256), which installs a private
//! Python with the `laya` package (the CPU build of PyTorch) and Laya's multilingual
//! checkpoint – all in File Sorter's own folder, apart from anything else on the computer.
//! While File Sorter needs it, a small helper (`laya_helper.py`) keeps the model loaded and
//! answers over its standard input and output; it ends after a while without questions,
//! while saving resources in the background, and when File Sorter ends.

use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use sha2::{Digest, Sha256};

/// The uv release fetched, with the SHA-256 of its downloads (from the release's `.sha256`).
const UV_VERSION: &str = "0.12.20";
const UV_SHA256_WINDOWS: &str = "95f9bc30fbb3574d276e28ac4a6de932d25153645853d13da8c21eec3bc88d06";
const UV_SHA256_LINUX: &str = "6590717592ace991ff83a63fef799e3ad9d33ecc8f96c5d6bdd732496e79337f";
/// The `laya` package and the Python it runs on.
const LAYA_PACKAGE: &str = "laya==0.4.12";
const PYTHON_VERSION: &str = "3.12";
const HELPER: &str = include_str!("../resources/laya_helper.py");

/// How long the model may take to load (the first time it reads a GB from disk).
const LOAD_TIME: Duration = Duration::from_secs(240);
/// How long one answer may take.
const ANSWER_TIME: Duration = Duration::from_secs(120);
/// Without questions this long, the helper ends (its memory goes back to the system).
const IDLE: Duration = Duration::from_secs(15 * 60);
/// After the helper failed, it isn't tried again for this long.
const RETRY_AFTER: Duration = Duration::from_secs(10 * 60);

/// Where Laya lives: `<state>/laya`.
#[derive(Debug, Clone)]
pub struct Place {
    pub dir: PathBuf,
}

impl Place {
    pub fn new(state_dir: &Path) -> Place {
        Place { dir: state_dir.join("laya") }
    }

    fn uv(&self) -> PathBuf {
        self.dir.join(if cfg!(windows) { "uv.exe" } else { "uv" })
    }

    fn venv(&self) -> PathBuf {
        self.dir.join("venv")
    }

    pub fn python(&self) -> PathBuf {
        if cfg!(windows) {
            self.venv().join("Scripts").join("python.exe")
        } else {
            self.venv().join("bin").join("python")
        }
    }

    fn helper(&self) -> PathBuf {
        self.dir.join("laya_helper.py")
    }

    fn marker(&self) -> PathBuf {
        self.dir.join("installed.json")
    }

    fn models(&self) -> PathBuf {
        self.dir.join("models")
    }

    /// Set up completely (the last step writes the marker).
    pub fn installed(&self) -> bool {
        self.marker().is_file() && self.python().is_file()
    }

    /// How much room it takes on disk.
    pub fn size(&self) -> u64 {
        fn walk(dir: &Path) -> u64 {
            let Ok(entries) = fs::read_dir(dir) else { return 0 };
            entries
                .flatten()
                .map(|e| match e.file_type() {
                    Ok(t) if t.is_dir() => walk(&e.path()),
                    Ok(t) if t.is_file() => e.metadata().map_or(0, |m| m.len()),
                    _ => 0,
                })
                .sum()
        }
        walk(&self.dir)
    }

    /// Take everything away again.
    pub fn remove(&self) -> std::io::Result<()> {
        stop();
        match fs::remove_dir_all(&self.dir) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        }
    }

    /// A command with File Sorter's own folders for uv, Python and the model.
    fn command(&self, program: &Path) -> Command {
        #[allow(unused_mut)]
        let mut command = Command::new(program);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            command.creation_flags(CREATE_NO_WINDOW);
        }
        command
            .env("UV_PYTHON_INSTALL_DIR", self.dir.join("python"))
            .env("UV_CACHE_DIR", self.dir.join("cache"))
            .env("UV_NO_CONFIG", "1")
            // Always its own Python, so a system update can't take it away.
            .env("UV_PYTHON_PREFERENCE", "only-managed")
            .env("HF_HOME", self.models())
            .env("HF_HUB_DISABLE_TELEMETRY", "1")
            .env("PYTHONUTF8", "1")
            .env("PYTHONIOENCODING", "utf-8")
            .env_remove("PYTHONPATH")
            .env_remove("VIRTUAL_ENV")
            .stdin(Stdio::null());
        command
    }
}

/// The steps of setting Laya up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// Fetching uv.
    Tool,
    /// A private Python.
    Python,
    /// The `laya` package and PyTorch (the biggest part).
    Packages,
    /// Laya's multilingual checkpoint.
    Model,
}

impl Step {
    pub const ALL: [Step; 4] = [Step::Tool, Step::Python, Step::Packages, Step::Model];

    pub fn number(self) -> usize {
        Step::ALL.iter().position(|s| *s == self).unwrap_or(0) + 1
    }
}

/// Set Laya up in `place`, telling `progress` each step. What was done before is kept, so
/// an interrupted setup goes on where it stopped.
pub fn install(place: &Place, mut progress: impl FnMut(Step)) -> Result<(), String> {
    fs::create_dir_all(&place.dir).map_err(|e| e.to_string())?;
    progress(Step::Tool);
    if !place.uv().is_file() {
        fetch_uv(place)?;
    }
    progress(Step::Python);
    if !place.python().is_file() {
        let mut venv = place.command(&place.uv());
        venv.args(["venv", "--python", PYTHON_VERSION]).arg(place.venv());
        run(venv, "Python")?;
    }
    progress(Step::Packages);
    let mut packages = place.command(&place.uv());
    packages.args(["pip", "install", "--python"]).arg(place.python()).arg(LAYA_PACKAGE).arg("--torch-backend=cpu");
    run(packages, "packages")?;
    // The downloaded packages are installed now; their copies only take room.
    let _ = fs::remove_dir_all(place.dir.join("cache"));
    progress(Step::Model);
    fs::write(place.helper(), HELPER).map_err(|e| e.to_string())?;
    let mut model = place.command(&place.python());
    model.arg(place.helper()).arg("--download");
    let out = run(model, "model")?;
    if !out.contains("\"ready\": true") {
        return Err(format!("the model did not answer: {}", tail(&out)));
    }
    let marker = json!({ "laya": LAYA_PACKAGE, "uv": UV_VERSION, "at": chrono::Local::now().to_rfc3339() });
    fs::write(place.marker(), marker.to_string()).map_err(|e| e.to_string())?;
    Ok(())
}

/// Run a setup command; its output goes to the log. Returns what it printed.
fn run(mut command: Command, what: &str) -> Result<String, String> {
    let out = command.output().map_err(|e| format!("{what}: {e}"))?;
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    for line in stderr.lines().chain(stdout.lines()).filter(|l| !l.trim().is_empty()).take(200) {
        log::info!("Laya setup ({what}): {line}");
    }
    if !out.status.success() {
        // uv's own «error: …» line says most; the last line says why.
        let error = stderr.lines().find(|l| l.trim_start().starts_with("error:")).map(str::trim);
        let why = tail(&stderr);
        return Err(match error {
            Some(e) if e != why => format!("{what}: {e} ({})", why.trim()),
            _ => format!("{what}: {why}"),
        });
    }
    Ok(stdout)
}

fn tail(text: &str) -> String {
    text.lines().rfind(|l| !l.trim().is_empty()).unwrap_or("").chars().take(300).collect()
}

/// Fetch uv from its GitHub release and check it.
fn fetch_uv(place: &Place) -> Result<(), String> {
    let (name, sha) = if cfg!(windows) {
        ("uv-x86_64-pc-windows-msvc.zip", UV_SHA256_WINDOWS)
    } else {
        ("uv-x86_64-unknown-linux-gnu.tar.gz", UV_SHA256_LINUX)
    };
    let url = format!("https://github.com/astral-sh/uv/releases/download/{UV_VERSION}/{name}");
    let archive = place.dir.join(name);
    let fetched = crate::update::curl()
        .args(["--max-time", "600", "--output"])
        .arg(&archive)
        .arg(&url)
        .output()
        .map_err(|e| format!("curl could not be started: {e}"))?;
    if !fetched.status.success() {
        let _ = fs::remove_file(&archive);
        return Err(format!("uv: {}", String::from_utf8_lossy(&fetched.stderr).trim()));
    }
    let result = (|| {
        let got = sha256_file(&archive).map_err(|e| e.to_string())?;
        if got != sha {
            return Err(format!("the download of uv is not the published one (SHA-256 {got})"));
        }
        if cfg!(windows) {
            let mut zip = zip::ZipArchive::new(fs::File::open(&archive).map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?;
            let mut entry = zip.by_name("uv.exe").map_err(|e| e.to_string())?;
            let mut out = fs::File::create(place.uv()).map_err(|e| e.to_string())?;
            std::io::copy(&mut entry, &mut out).map_err(|e| e.to_string())?;
        } else {
            let status = Command::new("tar")
                .arg("-xzf")
                .arg(&archive)
                .arg("-C")
                .arg(&place.dir)
                .arg("--strip-components=1")
                .arg("uv-x86_64-unknown-linux-gnu/uv")
                .status()
                .map_err(|e| e.to_string())?;
            if !status.success() {
                return Err("uv could not be unpacked".into());
            }
        }
        Ok(())
    })();
    let _ = fs::remove_file(&archive);
    result
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

// ---------- the helper, while File Sorter needs it ----------

static PLACE: Mutex<Option<Place>> = Mutex::new(None);
static ON: AtomicBool = AtomicBool::new(false);
/// Only the window and `--headless` start the helper (not the right-click window, which
/// shouldn't wait for a model to load).
static ALLOWED: AtomicBool = AtomicBool::new(false);
static HELPER_PROCESS: Mutex<Option<Helper>> = Mutex::new(None);
static FAILED_AT: Mutex<Option<Instant>> = Mutex::new(None);

/// Where Laya is (the state folder), and whether the setting is on.
pub fn configure(state_dir: &Path, on: bool) {
    if let Ok(mut place) = PLACE.lock() {
        *place = Some(Place::new(state_dir));
    }
    ON.store(on, Ordering::Relaxed);
    if !on {
        stop();
    }
}

/// Let this process use Laya (the window, `--headless`).
pub fn allow() {
    ALLOWED.store(true, Ordering::Relaxed);
}

/// Set up, switched on, and not saving resources in the background.
pub fn available() -> bool {
    #[cfg(test)]
    if !TEST_ON.with(|on| on.get()) {
        return false;
    }
    ALLOWED.load(Ordering::Relaxed)
        && ON.load(Ordering::Relaxed)
        && !crate::lean::active()
        && PLACE.lock().is_ok_and(|p| p.as_ref().is_some_and(Place::installed))
        && FAILED_AT.lock().map(|f| f.is_none_or(|t| t.elapsed() > RETRY_AFTER)).unwrap_or(true)
}

#[cfg(test)]
thread_local! {
    static TEST_ON: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

struct Helper {
    child: Child,
    stdin: ChildStdin,
    lines: Receiver<String>,
    used: Instant,
    next_id: u64,
}

impl Helper {
    fn start(place: &Place) -> Result<Helper, String> {
        // Written again at every start, so an update of File Sorter brings its new version.
        fs::write(place.helper(), HELPER).map_err(|e| e.to_string())?;
        let mut command = place.command(&place.python());
        command
            .arg(place.helper())
            .env("HF_HUB_OFFLINE", "1")
            .env("TRANSFORMERS_OFFLINE", "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = command.spawn().map_err(|e| format!("the helper could not be started: {e}"))?;
        let stdin = child.stdin.take().ok_or("no input")?;
        let stdout = child.stdout.take().ok_or("no output")?;
        if let Some(stderr) = child.stderr.take() {
            std::thread::spawn(move || {
                for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                    if !line.trim().is_empty() {
                        log::debug!("Laya: {line}");
                    }
                }
            });
        }
        let (tx, lines) = channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if tx.send(line).is_err() {
                    return;
                }
            }
        });
        let mut helper = Helper { child, stdin, lines, used: Instant::now(), next_id: 1 };
        let deadline = Instant::now() + LOAD_TIME;
        loop {
            let wait = deadline.saturating_duration_since(Instant::now());
            let line = helper.lines.recv_timeout(wait).map_err(|_| {
                let _ = helper.child.kill();
                "the model did not load".to_string()
            })?;
            if serde_json::from_str::<Value>(&line).is_ok_and(|v| v["ready"] == true) {
                log::info!("Laya is ready");
                return Ok(helper);
            }
        }
    }

    fn ask(&mut self, states: &[String], questions: &Value, max_len: Option<usize>) -> Result<Vec<Value>, String> {
        let id = self.next_id;
        self.next_id += 1;
        let request = json!({ "id": id, "states": states, "questions": questions, "max_len": max_len });
        writeln!(self.stdin, "{request}").map_err(|e| e.to_string())?;
        self.stdin.flush().map_err(|e| e.to_string())?;
        let deadline = Instant::now() + ANSWER_TIME;
        loop {
            let wait = deadline.saturating_duration_since(Instant::now());
            let line = self.lines.recv_timeout(wait).map_err(|_| "no answer in time".to_string())?;
            let Ok(answer) = serde_json::from_str::<Value>(&line) else { continue };
            if answer["id"] != id {
                continue;
            }
            self.used = Instant::now();
            if let Some(error) = answer["error"].as_str() {
                return Err(error.to_string());
            }
            return answer["answers"].as_array().cloned().ok_or_else(|| "no answers".to_string());
        }
    }
}

impl Drop for Helper {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// End the helper (its memory goes back to the system).
pub fn stop() {
    if let Ok(mut helper) = HELPER_PROCESS.lock()
        && helper.take().is_some()
    {
        log::info!("Laya ended");
    }
}

/// End the helper when it wasn't asked anything for a while, or while saving resources.
fn watch_idle() {
    static WATCHING: AtomicBool = AtomicBool::new(false);
    if WATCHING.swap(true, Ordering::Relaxed) {
        return;
    }
    std::thread::spawn(|| {
        loop {
            std::thread::sleep(Duration::from_secs(30));
            let idle = HELPER_PROCESS.lock().is_ok_and(|h| h.as_ref().is_some_and(|h| h.used.elapsed() > IDLE));
            if idle || crate::lean::active() || !ON.load(Ordering::Relaxed) {
                stop();
            }
        }
    });
}

/// Ask Laya `questions` about each of `states`; the answers per state (`None` when Laya isn't
/// there or failed – callers then do without).
pub fn ask(states: &[String], questions: &Value, max_len: Option<usize>) -> Option<Vec<Value>> {
    if states.is_empty() || !available() {
        return None;
    }
    let place = PLACE.lock().ok()?.clone()?;
    let mut guard = HELPER_PROCESS.lock().ok()?;
    if guard.is_none() {
        match Helper::start(&place) {
            Ok(helper) => {
                *guard = Some(helper);
                watch_idle();
            }
            Err(e) => {
                log::info!("Laya could not be used: {e}");
                *FAILED_AT.lock().ok()? = Some(Instant::now());
                return None;
            }
        }
    }
    let helper = guard.as_mut()?;
    match helper.ask(states, questions, max_len) {
        Ok(answers) if answers.len() == states.len() => Some(answers),
        Ok(_) => None,
        Err(e) => {
            log::info!("Laya did not answer: {e}");
            *guard = None;
            *FAILED_AT.lock().ok()? = Some(Instant::now());
            None
        }
    }
}

/// The chosen option of a choice answer with its probability, and the probability of the
/// next most likely one.
fn choice(answer: &Value) -> Option<(String, f64, f64)> {
    let probabilities = answer["probabilities"].as_object()?;
    let mut ranked: Vec<(&String, f64)> = probabilities.iter().filter_map(|(k, v)| Some((k, v.as_f64()?))).collect();
    ranked.sort_by(|a, b| b.1.total_cmp(&a.1));
    let (best, p) = ranked.first()?;
    Some(((*best).clone(), *p, ranked.get(1).map_or(0.0, |r| r.1)))
}

// ---------- what File Sorter asks ----------

/// The option for «none of the modules».
const NONE: &str = "none";

/// Asks which of `options` (id, what it is) `text` is about, or none of them (`none`: what
/// that means here). The id and how sure Laya is, when it is clearly sure: at least `0.6`, and
/// twice as likely as the next.
fn pick_one(instructions: &str, options: &[(String, String)], none: &str, text: String) -> Option<(String, f64)> {
    if options.is_empty() || !available() {
        return None;
    }
    let mut criteria = serde_json::Map::new();
    for (id, about) in options {
        criteria.entry(id.clone()).or_insert_with(|| Value::String(about.clone()));
    }
    criteria.insert(NONE.into(), Value::String(none.into()));
    let questions = json!({"pick": {"type": "choice", "instructions": instructions, "criteria": criteria}});
    let answers = ask(&[text], &questions, None)?;
    let (id, p, second) = choice(&answers[0]["pick"])?;
    (id != NONE && p >= 0.6 && p >= 2.0 * second).then_some((id, p))
}

/// The start of a document as it is shown to Laya: its name, then its text.
fn document(name: &str, text: &str) -> String {
    let body: String = text.chars().take(3000).collect();
    format!("File name: {name}\n\n{body}")
}

/// Which module a document belongs to: its label and how sure Laya is, when it is clearly
/// sure. `allowed` limits the answer to some modules (those the keywords found about equally
/// likely).
pub fn module_for(config: &crate::config::Config, name: &str, text: &str, allowed: &[String]) -> Option<(String, f64)> {
    let options: Vec<(String, String)> = config
        .courses
        .iter()
        .filter(|c| allowed.is_empty() || allowed.contains(&c.label()))
        .map(|course| {
            let mut about = format!("the course «{}»", course.name);
            if !course.subject.is_empty() {
                about.push_str(&format!(" (study program {})", course.subject));
            }
            let words: Vec<&str> = course.keywords.iter().map(String::as_str).take(12).collect();
            if !words.is_empty() {
                about.push_str(&format!("; typical words: {}", words.join(", ")));
            }
            (course.label(), about)
        })
        .collect();
    pick_one(
        "This is a document a student downloaded. Which university course is it for?",
        &options,
        "none of these courses: other subjects, personal or administrative documents",
        document(name, text),
    )
}

/// Which subfolder of a module a document goes in (options: folder name, what goes there),
/// when Laya is clearly sure; none means the module's folder itself.
pub fn subfolder_for(name: &str, text: &str, options: &[(String, String)]) -> Option<(String, f64)> {
    pick_one(
        "This is a document from a university course. Which folder of the course does it belong in?",
        options,
        "none of these folders: general course information, or something that fits no folder",
        document(name, text),
    )
}

/// What a date in a sentence is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DateKind {
    /// Something has to be handed in (or registered for) by then.
    Due,
    /// An exam or another event takes place then.
    Event,
    Other,
}

/// For each sentence with a date: what the date is, and how sure Laya is.
pub fn date_kinds(sentences: &[String]) -> Option<Vec<(DateKind, f64)>> {
    if sentences.is_empty() {
        return Some(Vec::new());
    }
    let questions = json!({
        "kind": {
            "type": "choice",
            "instructions": "What is the date in this sentence from a university course document?",
            "criteria": {
                "due": "a deadline: something must be handed in, submitted, uploaded or registered for by then",
                "event": "an exam, test, quiz, presentation, excursion or workshop takes place then",
                "other": "anything else: a lecture, a publication or version date, a holiday, a past event",
            },
        }
    });
    let answers = ask(sentences, &questions, Some(256))?;
    let kind = |a: &Value| match choice(&a["kind"]) {
        Some((k, p, _)) if k == "due" => (DateKind::Due, p),
        Some((k, p, _)) if k == "event" => (DateKind::Event, p),
        Some((_, p, _)) => (DateKind::Other, p),
        None => (DateKind::Other, 0.0),
    };
    Some(answers.iter().map(kind).collect())
}

/// For each sentence with a date (known to be a hand-in, or an appointment): which of the
/// `categories` (id, what such a date is) it is, when Laya is clearly sure.
pub fn date_categories(sentences: &[String], categories: &[(String, String)]) -> Option<Vec<Option<String>>> {
    if sentences.is_empty() || categories.is_empty() {
        return Some(Vec::new());
    }
    let mut criteria = serde_json::Map::new();
    for (id, about) in categories {
        criteria.entry(id.clone()).or_insert_with(|| Value::String(about.clone()));
    }
    let questions = json!({
        "category": {
            "type": "choice",
            "instructions": "Which kind of date is in this sentence from a university course document?",
            "criteria": criteria,
        }
    });
    let answers = ask(sentences, &questions, Some(256))?;
    let sure =
        |a: &Value| choice(&a["category"]).and_then(|(id, p, second)| (p >= 0.6 && p >= 2.0 * second).then_some(id));
    Some(answers.iter().map(sure).collect())
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// Laya on for the calling test's thread.
    pub fn enable_in_this_test() {
        TEST_ON.with(|on| on.set(true));
    }

    /// A stand-in for the `laya` package (next to the helper, so Python finds it first):
    /// documents mentioning «Varianz» are Statistik, «due»/«abgabe»/«rendre» sentences are
    /// deadlines, «Prüfung»/«exam» ones exams.
    const FAKE_LAYA: &str = r#"
class Agent:
    def system_one(self, state, questions, max_len=None):
        return self.predict_batch([state], questions)[0]
    def predict_batch(self, states, questions, batch_size=None, max_len=None):
        out = []
        for s in states:
            low = s.lower()
            answers = {}
            for qid, q in questions.items():
                criteria = q.get("criteria", {})
                keys = list(criteria.keys())
                def named(k):
                    # «Name» and typical words of an option the text mentions.
                    about = criteria[k].lower()
                    words = [w.strip(" .;,") for w in about.split("typical words:")[-1].split(",")] if "typical words:" in about else []
                    if "«" in about:
                        words.append(about.split("«")[1].split("»")[0])
                    return any(w and len(w) > 2 and w in low for w in words)
                def leading(marker):
                    # The option named so, or whose description starts so.
                    return next((k for k in keys if k == marker or criteria[k].lower().startswith(marker)), None)
                pick = None
                if qid == "pick" and "varianz" in low:
                    pick = next((k for k in keys if "statistik" in k.lower()), None)
                elif qid == "kind":
                    due = any(w in low for w in ("abgabe", "due", "rendre"))
                    event = any(w in low for w in ("prüfung", "exam", "exkursion"))
                    pick = "due" if due else ("event" if event else "other")
                elif qid != "pick" or "folder" in q.get("instructions", ""):
                    pick = next((k for k in keys if k != "none" and named(k)), None)
                    if pick is None and any(w in low for w in ("abgabe", "due", "rendre", "aufgabe 1")):
                        pick = leading("exercise") or leading("an exercise")
                    if pick is None and any(w in low for w in ("prüfung", "exam")):
                        pick = leading("exam") or leading("an exam")
                pick = pick or "none"
                probs = {k: (0.9 if k == pick else 0.1 / max(1, len(keys) - 1)) for k in keys}
                answers[qid] = {"type": "choice", "choice": pick, "probabilities": probs}
            out.append({"answers": answers})
        return out
def load(*args, **kwargs):
    return Agent()
"#;

    /// A Laya «set up» with the stand-in, using the system's Python.
    pub fn fake_place(dir: &Path) -> Option<Place> {
        let python = ["python3", "python"].into_iter().find(|p| Command::new(p).arg("--version").output().is_ok())?;
        let place = Place::new(dir);
        let bin = place.python();
        fs::create_dir_all(bin.parent()?).ok()?;
        #[cfg(unix)]
        {
            let real = String::from_utf8(
                Command::new("sh").arg("-c").arg(format!("command -v {python}")).output().ok()?.stdout,
            )
            .ok()?;
            std::os::unix::fs::symlink(real.trim(), &bin).ok()?;
        }
        #[cfg(not(unix))]
        {
            let _ = python;
            return None;
        }
        fs::create_dir_all(place.dir.join("laya")).ok()?;
        fs::write(place.dir.join("laya").join("__init__.py"), FAKE_LAYA).ok()?;
        fs::write(place.marker(), "{}").ok()?;
        Some(place)
    }

    #[test]
    fn the_helper_answers_questions_over_its_pipe() {
        let tmp = tempfile::tempdir().unwrap();
        let Some(place) = fake_place(tmp.path()) else { return };
        let mut helper = Helper::start(&place).unwrap();
        let questions = json!({"kind": {"type": "choice", "instructions": "?",
            "criteria": {"due": "a", "event": "b", "other": "c"}}});
        let answers = helper
            .ask(
                &["Abgabe bis 3. November".into(), "Prüfung am 20. Januar".into(), "Stand: 1. Oktober".into()],
                &questions,
                None,
            )
            .unwrap();
        let picks: Vec<String> = answers.iter().map(|a| choice(&a["kind"]).unwrap().0).collect();
        assert_eq!(picks, ["due", "event", "other"]);
        // A bad request gets an error, and the helper goes on.
        assert!(helper.ask(&["x".into()], &json!({"q": 1}), None).is_err());
        assert_eq!(helper.ask(&["Abgabe".into()], &questions, None).unwrap().len(), 1);
    }

    /// With the model there: a document the keywords don't decide is sorted by it, and a
    /// French deadline the rules don't know is found (one test: the place is shared).
    #[test]
    fn sorting_and_deadlines_ask_the_model() {
        use crate::config::{Config, Course};
        use crate::sorter::Outcome;
        let tmp = tempfile::tempdir().unwrap();
        if fake_place(tmp.path()).is_none() {
            return;
        }
        enable_in_this_test();
        allow();
        configure(tmp.path(), true);
        // Sorting.
        let mut cfg = crate::sorter::tests::test_config(tmp.path());
        cfg.courses.push(Course::new("Statistik", &["stat101"]));
        let file = cfg.watch_path().join("kapitel4.txt");
        fs::write(&file, "Varianzanalyse und Regression: Schätzung der Parameter.").unwrap();
        let result = crate::sorter::sort_file(&file, &Config { state_dir: None, ..cfg });
        match &result.outcome {
            Outcome::Moved { course, reason, .. } => {
                assert_eq!(course, "Statistik");
                assert_eq!(*reason, crate::matcher::Reason::Ai(90));
            }
            other => panic!("{other:?}"),
        }
        // Deadlines.
        let reference = chrono::NaiveDate::from_ymd_opt(2026, 10, 20).unwrap();
        let text = "Le devoir 3 est à rendre avant le 7 novembre 2026 à 18h.\nVersion du 3 octobre 2026.";
        assert!(crate::deadlines::find(text, reference).is_empty(), "the rules don't know it");
        let mut categories = crate::deadlines::Category::defaults();
        let found = crate::deadlines::find_in(text, reference, &categories);
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].date, chrono::NaiveDate::from_ymd_opt(2026, 11, 7).unwrap());
        assert!(found[0].ai && found[0].kind == crate::deadlines::Kind::Due);
        assert_eq!(found[0].category, "exercise");
        assert_eq!(found[0].title.as_deref(), Some("Le devoir 3 est à rendre avant le 7 novembre 2026 à 18h"));
        // A category of the user's own.
        categories.push(crate::deadlines::Category::custom("Exkursion", &categories));
        let found = crate::deadlines::find_in("Die Exkursion ins Museum ist am 12.11.2026.", reference, &categories);
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!((found[0].category.as_str(), found[0].kind), ("c1", crate::deadlines::Kind::Exam));
        // Subfolders: a name that says nothing, read by the model.
        let options = [
            ("Vorlesung".to_string(), "lecture slides or script".to_string()),
            ("Übungen".to_string(), "exercise sheets or homework to solve".to_string()),
        ];
        let picked = subfolder_for("scan_0042.pdf", "Blatt 4. Aufgabe 1: Zeigen Sie …", &options);
        assert_eq!(picked.map(|p| p.0).as_deref(), Some("Übungen"));
        assert_eq!(subfolder_for("info.pdf", "Willkommen im Kurs", &options), None);
        stop();
    }

    #[test]
    fn choices_need_a_clear_winner() {
        let answer = json!({"probabilities": {"a": 0.7, "b": 0.2, "c": 0.1}});
        assert_eq!(choice(&answer), Some(("a".into(), 0.7, 0.2)));
        assert_eq!(Step::Packages.number(), 3);
    }
}
