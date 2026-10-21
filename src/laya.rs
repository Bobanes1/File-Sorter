//! The AI model Laya, optional: it reads documents in many languages and answers questions
//! about them – which module a file belongs to when the keywords don't decide, which subfolder
//! when the rules don't, and what a date in a sentence is. It runs on this computer, natively
//! (no Python); only the model (about 680 MB) is downloaded once, into the state folder.
//!
//! The model is laya-uni by Nils Briggen: Laya (convaiinnovations/laya, multilingual
//! checkpoint) fine-tuned on course files of every faculty (`training/laya-uni`). It is a
//! ModernBERT encoder plus a small decision head: each answer option sits behind a `[MASK]`
//! marker, and the head scores the markers – a port of Laya's own `rl_common.py` /
//! `rl_agent_api.py` (choice questions only).
//!
//! The model is pinned: its files' URLs and SHA-256 are in this file, and a newer model only
//! comes with a newer File Sorter (tested with the real model in CI first).

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use candle_core::{D, DType, Device, IndexOp, Module, Tensor};
use candle_nn::{Embedding, LayerNorm, Linear, VarBuilder};
use candle_transformers::models::modernbert::{self, ModernBert};
use serde::Deserialize;

use crate::config::{Config, Course};

/// laya-uni-1: its files (and the confidence its training chose for sorting on its own,
/// 70 % / 2× – File Sorter sorts with less, see `ALONE`).
const MODEL: &str = r#"{"name": "laya-uni-1", "min_probability": 0.7, "min_lead": 2.0, "files": [
 {"name": "rl_agent_config.json", "size": 4987, "sha256": "c06f9dff24ed9f1132e5e44020381d380d3b5ea30e650f90a82ff6378215c6ff"},
 {"name": "encoder-config.json", "size": 1939, "sha256": "36b7d4898274ed6f06ce9c1f0e34034d8bd39295ab232d7cabc41a3eac821e5f"},
 {"name": "tokenizer.json", "size": 34363188, "sha256": "609d8f4c067cd3950f88594c5a802616cea245823836ef5848ee4fc40aab5b6f"},
 {"name": "model.safetensors", "size": 643835524, "sha256": "2a73a52757ff1557d59abcdb43db469327df077146ce68140292a943572925bb"}]}"#;
/// Where the model's files are published (the release laya-uni-1 of NilsBriggen/laya-uni).
const MODEL_URL: &str = "https://github.com/NilsBriggen/laya-uni/releases/download/laya-uni-1/";
/// Where File Sorter 0.15–0.16 kept Laya with its own Python (removed once laya-uni is there).
const OLD_DIR: &str = "laya";
pub const INSTRUCTIONS: &str = "Which university module does this file belong to?";
/// Enough text for the model's 1024 tokens; tokenizing a whole book would only cost time.
const MAX_TEXT_CHARS: usize = 6000;
/// Option texts are cut to this many tokens (as in training).
const MAX_OPTION_TOKENS: usize = 48;
/// The model leaves memory after this long without questions.
const IDLE: Duration = Duration::from_secs(15 * 60);
/// After the model failed to load, it is tried again after this long.
#[cfg_attr(test, allow(dead_code))]
const RETRY_AFTER: Duration = Duration::from_secs(10 * 60);

/// The model's files and the confidence it needs to choose a module.
#[derive(Debug, Clone, PartialEq, Deserialize)]
struct Manifest {
    name: String,
    files: Vec<Asset>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
struct Asset {
    name: String,
    size: u64,
    sha256: String,
}

impl Manifest {
    fn get() -> Manifest {
        serde_json::from_str(MODEL).expect("the built-in manifest is valid")
    }

    fn size(&self) -> u64 {
        self.files.iter().map(|f| f.size).sum()
    }
}

/// Where Laya is kept: `<state>/laya-uni/<model>/`.
#[derive(Debug, Clone)]
pub struct Place {
    root: PathBuf,
    /// The folder of File Sorter 0.15–0.16's Laya (with Python).
    old: PathBuf,
}

impl Place {
    pub fn new(state_dir: &Path) -> Place {
        Place { root: state_dir.join("laya-uni"), old: state_dir.join(OLD_DIR) }
    }

    fn model_dir(&self) -> PathBuf {
        self.root.join(Manifest::get().name)
    }

    /// All files there (each is only put in place after its checksum was right).
    pub fn installed(&self) -> bool {
        let dir = self.model_dir();
        Manifest::get().files.iter().all(|f| dir.join(&f.name).is_file())
    }

    /// The bytes downloaded so far, and the whole size.
    pub fn progress(&self) -> (u64, u64) {
        let manifest = Manifest::get();
        let dir = self.model_dir();
        let size = |p: PathBuf| fs::metadata(p).map(|m| m.len()).unwrap_or(0);
        let done = manifest
            .files
            .iter()
            .map(|f| {
                let file = dir.join(&f.name);
                if file.is_file() { f.size } else { size(partial(&file)) }
            })
            .sum();
        (done, manifest.size())
    }

    /// File Sorter 0.15–0.16's Laya (with its own Python) is there.
    pub fn had_older(&self) -> bool {
        self.old.join("installed.json").is_file() || self.old.join("venv").is_dir()
    }

    /// How much room it takes.
    pub fn size(&self) -> u64 {
        folder_size(&self.root)
    }

    /// Take Laya away again.
    pub fn remove(&self) -> std::io::Result<()> {
        stop();
        match fs::remove_dir_all(&self.root) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        }
    }

    /// Remove what isn't needed any more: other models' folders, and File Sorter 0.15–0.16's
    /// Laya with its own Python. Returns the bytes freed.
    fn tidy(&self) -> u64 {
        let keep = Manifest::get().name;
        let mut freed = 0;
        for entry in fs::read_dir(&self.root).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_dir() && entry.file_name() != keep.as_str() {
                freed += folder_size(&path);
                let _ = fs::remove_dir_all(&path);
            }
        }
        // Only if it is that Laya (its marker or its Python environment is there).
        if self.had_older() {
            freed += folder_size(&self.old);
            let _ = fs::remove_dir_all(&self.old);
        }
        freed
    }
}

fn folder_size(dir: &Path) -> u64 {
    let Ok(entries) = fs::read_dir(dir) else { return 0 };
    entries
        .flatten()
        .map(|e| match e.file_type() {
            Ok(t) if t.is_dir() => folder_size(&e.path()),
            Ok(t) if t.is_file() => e.metadata().map(|m| m.len()).unwrap_or(0),
            _ => 0,
        })
        .sum()
}

fn partial(file: &Path) -> PathBuf {
    file.with_extension("partial")
}

/// Download the model (the files still missing; an interrupted download goes on where it
/// stopped), each file checked against its size and SHA-256. `progress` gets the bytes done
/// and the whole size now and then.
pub fn install(place: &Place, mut progress: impl FnMut(u64, u64)) -> Result<(), String> {
    let manifest = Manifest::get();
    let dir = place.model_dir();
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    for asset in &manifest.files {
        let file = dir.join(&asset.name);
        if file.is_file() {
            continue;
        }
        let part = partial(&file);
        let mut child = crate::update::curl()
            .args(["--max-time", "7200", "--continue-at", "-", "--max-filesize"])
            .arg(asset.size.to_string())
            .arg("--output")
            .arg(&part)
            .arg(format!("{MODEL_URL}{}", asset.name))
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("curl could not be started: {e}"))?;
        loop {
            match child.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) => {
                    let (done, total) = place.progress();
                    progress(done, total);
                    std::thread::sleep(Duration::from_millis(500));
                }
                Err(e) => return Err(e.to_string()),
            }
        }
        let fetched = child.wait_with_output().map_err(|e| e.to_string())?;
        if !fetched.status.success() {
            return Err(format!("{}: {}", asset.name, String::from_utf8_lossy(&fetched.stderr).trim()));
        }
        let size = fs::metadata(&part).map(|m| m.len()).unwrap_or(0);
        let got = crate::update::sha256_of(&part).map_err(|e| e.to_string())?;
        if size != asset.size || got != asset.sha256 {
            let _ = fs::remove_file(&part);
            return Err(format!("the download of {} is not the published one (SHA-256 {got})", asset.name));
        }
        fs::rename(&part, &file).map_err(|e| e.to_string())?;
    }
    let (done, total) = place.progress();
    progress(done, total);
    let freed = place.tidy();
    if freed > 0 {
        log::info!("Removed the older Laya ({} MB)", freed / 1_000_000);
    }
    log::info!("Laya model {} is set up", manifest.name);
    Ok(())
}

// ---------- the model, while File Sorter needs it ----------

static PLACE: Mutex<Option<Place>> = Mutex::new(None);
static ON: AtomicBool = AtomicBool::new(false);
/// The model in memory, and when it was last asked.
static LOADED: Mutex<Option<(Arc<Laya>, Instant)>> = Mutex::new(None);
#[cfg_attr(test, allow(dead_code))]
static FAILED_AT: Mutex<Option<Instant>> = Mutex::new(None);

/// Where Laya is (the state folder), and whether the setting is on.
pub fn configure(state_dir: &Path, on: bool) {
    let place = Place::new(state_dir);
    if place.installed() && place.old.is_dir() {
        let place = place.clone();
        std::thread::spawn(move || {
            let freed = place.tidy();
            if freed > 0 {
                log::info!("Removed the older Laya with its own Python ({} MB)", freed / 1_000_000);
            }
        });
    }
    if let Ok(mut p) = PLACE.lock() {
        *p = Some(place);
    }
    ON.store(on, Ordering::Relaxed);
    if !on {
        stop();
    }
}

/// Set up, switched on, and not saving resources in the background.
pub fn available() -> bool {
    #[cfg(test)]
    return tests::FAKE.with(|f| f.get().is_some());
    #[cfg(not(test))]
    {
        ON.load(Ordering::Relaxed)
            && !crate::lean::active()
            && PLACE.lock().is_ok_and(|p| p.as_ref().is_some_and(Place::installed))
            && FAILED_AT.lock().map(|f| f.is_none_or(|t| t.elapsed() > RETRY_AFTER)).unwrap_or(true)
    }
}

/// Drop the model from memory (switched off, saving resources, or not asked for a while).
pub fn stop() {
    if let Ok(mut loaded) = LOADED.lock()
        && loaded.take().is_some()
    {
        log::info!("Laya left memory");
    }
}

/// The model, loaded if it isn't yet.
#[cfg_attr(test, allow(dead_code))]
fn model() -> Option<Arc<Laya>> {
    let place = PLACE.lock().ok()?.clone()?;
    let mut loaded = LOADED.lock().ok()?;
    if let Some((model, used)) = loaded.as_mut() {
        *used = Instant::now();
        return Some(model.clone());
    }
    let started = Instant::now();
    match Laya::load(&place.model_dir()) {
        Ok(model) => {
            log::info!("Laya is ready ({:.1} s)", started.elapsed().as_secs_f32());
            let model = Arc::new(model);
            *loaded = Some((model.clone(), Instant::now()));
            watch_idle();
            Some(model)
        }
        Err(e) => {
            log::info!("Laya could not be loaded: {e}");
            *FAILED_AT.lock().ok()? = Some(Instant::now());
            None
        }
    }
}

/// Drop the model when it wasn't asked anything for a while, or while saving resources.
fn watch_idle() {
    static WATCHING: AtomicBool = AtomicBool::new(false);
    if WATCHING.swap(true, Ordering::Relaxed) {
        return;
    }
    std::thread::spawn(|| {
        loop {
            std::thread::sleep(Duration::from_secs(30));
            let idle = LOADED.lock().is_ok_and(|l| l.as_ref().is_some_and(|(_, used)| used.elapsed() > IDLE));
            if idle || crate::lean::active() || !ON.load(Ordering::Relaxed) {
                stop();
            }
        }
    });
}

/// The probability of each option (`None` when Laya isn't there, failed, or not all options
/// fit into the question – callers then do without).
fn probabilities(instructions: &str, options: &[String], state: &str) -> Option<Vec<f32>> {
    if options.is_empty() || !available() {
        return None;
    }
    #[cfg(test)]
    return tests::FAKE.with(|f| f.get()).map(|answer| answer(instructions, options, state));
    #[cfg(not(test))]
    match model()?.choose(instructions, options, state) {
        Ok(p) => p,
        Err(e) => {
            log::info!("Laya failed: {e}");
            None
        }
    }
}

// ---------- what File Sorter asks ----------

/// What the model reads about a file (training rows are made the same way: `laya_rows.rs`).
pub fn state_for(name: &str, text: &str) -> String {
    let text: String = text.chars().take(MAX_TEXT_CHARS).collect();
    format!("File name: {name}\n\n{text}")
}

/// "Programmieren: Java, OOP" – an answer and what it means, as one option. Laya's own code
/// renders a criterion `{label: text}` the same way.
pub fn option(label: &str, keywords: &[String]) -> String {
    if keywords.is_empty() { label.to_string() } else { format!("{label}: {}", keywords.join(", ")) }
}

fn option_text(course: &Course) -> String {
    option(&course.label(), &course.keywords)
}

/// The best option if the model is sure enough of it: at least `min_probability`, and
/// `min_lead` times as likely as the second.
fn confident(p: &[f32], (min_probability, min_lead): (f32, f32)) -> Option<(usize, u8)> {
    let order = ranked(p);
    let (best, second) = (p[*order.first()?], order.get(1).map_or(0.0, |i| p[*i]));
    (best >= min_probability && best >= min_lead * second).then(|| (order[0], (best * 100.0).round() as u8))
}

/// The options' places, most likely first.
fn ranked(p: &[f32]) -> Vec<usize> {
    let mut order: Vec<usize> = (0..p.len()).collect();
    order.sort_by(|a, b| p[*b].total_cmp(&p[*a]));
    order
}

/// How sure Laya must be to sort a file on its own (with Laya on, it alone decides module and
/// subfolder): lower than laya-uni's calibrated 70 % / 2×, as chosen – it sorts more files, and
/// sometimes wrongly. Checked on the real model in CI (`--laya-check`).
pub const ALONE: (f32, f32) = (0.4, 1.5);
/// How sure it must be to fill a gap the rules leave (a deadline's category).
const GAPS: (f32, f32) = (0.6, 2.0);
/// Not sure of one module, but these two together are this likely: it asks between them
/// (else nothing fits, and nothing is asked).
const BETWEEN_TWO: f32 = 0.7;

/// The option for «none of these».
const NONE: &str = "none";

/// Which of `options` (id, what it is) `state` is about, or none of them (`none`: what that
/// means here): the id and how sure Laya is, when it is sure enough (`thresholds`).
fn pick_one(
    instructions: &str,
    options: &[(String, String)],
    none: Option<&str>,
    state: &str,
    thresholds: (f32, f32),
) -> Option<(String, f64)> {
    let answers = ask_options(instructions, options, none, state)?;
    let p: Vec<f32> = answers.iter().map(|(_, p)| *p).collect();
    let (i, sure) = confident(&p, thresholds)?;
    (answers[i].0 != NONE).then(|| (answers[i].0.clone(), f64::from(sure) / 100.0))
}

/// How likely each of `options` (and `none`, if given) is.
fn ask_options(
    instructions: &str,
    options: &[(String, String)],
    none: Option<&str>,
    state: &str,
) -> Option<Vec<(String, f32)>> {
    let mut ids: Vec<String> = options.iter().map(|(id, _)| id.clone()).collect();
    let text = |(id, about): &(String, String)| {
        if about.is_empty() { id.clone() } else { option(id, std::slice::from_ref(about)) }
    };
    let mut texts: Vec<String> = options.iter().map(text).collect();
    if let Some(none) = none {
        ids.push(NONE.into());
        texts.push(option(NONE, &[none.to_string()]));
    }
    let p = probabilities(instructions, &texts, state)?;
    Some(ids.into_iter().zip(p).collect())
}

/// Laya's choice of a file's module.
#[derive(Debug, Clone, PartialEq)]
pub enum ModuleChoice {
    /// This module, this sure (0–1).
    Sure(String, f64),
    /// One of these two: the user decides.
    Between(Vec<String>),
    /// None of the modules.
    Nothing,
}

/// Which module a document belongs to, when Laya sorts on its own (`None` when it can't be
/// asked: the model isn't there or failed, or there are fewer than two modules).
pub fn module_of(config: &Config, name: &str, text: &str) -> Option<ModuleChoice> {
    let p: Vec<f32> = module_probabilities(config, name, text)?.into_iter().map(|(_, p)| p).collect();
    let label = |i: usize| config.courses[i].label();
    if let Some((i, sure)) = confident(&p, ALONE) {
        return Some(ModuleChoice::Sure(label(i), f64::from(sure) / 100.0));
    }
    let order = ranked(&p);
    Some(if p[order[0]] + p[order[1]] >= BETWEEN_TWO {
        ModuleChoice::Between(vec![label(order[0]), label(order[1])])
    } else {
        ModuleChoice::Nothing
    })
}

/// How likely each module is (at least two are needed).
pub fn module_probabilities(config: &Config, name: &str, text: &str) -> Option<Vec<(String, f32)>> {
    if config.courses.len() < 2 {
        return None;
    }
    let options: Vec<String> = config.courses.iter().map(option_text).collect();
    let p = probabilities(INSTRUCTIONS, &options, &state_for(name, text))?;
    Some(config.courses.iter().map(|c| c.label()).zip(p).collect())
}

/// Asked like the module question laya-uni was trained on: «Folder: the words that say it».
/// (Described in sentences, with a «none of these» option, it answered «none» to an exercise
/// sheet and a script; asked this way it got all seven test documents right.) Nothing fits:
/// about equally likely, so not sure.
const SUBFOLDER_QUESTION: &str = "Which folder of the university module does this file belong to?";

/// Which subfolder of a module a document goes in, when Laya sorts on its own (options:
/// folder name, the words that say it); not sure: the module's folder itself.
pub fn subfolder_of(name: &str, text: &str, options: &[(String, String)]) -> Option<(String, f64)> {
    if options.len() < 2 {
        return None;
    }
    pick_one(SUBFOLDER_QUESTION, options, None, &state_for(name, text), ALONE)
}

/// How likely each subfolder is.
pub fn subfolder_probabilities(name: &str, text: &str, options: &[(String, String)]) -> Option<Vec<(String, f32)>> {
    ask_options(SUBFOLDER_QUESTION, options, None, &state_for(name, text))
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
    let options = [
        option(
            "due",
            &["a deadline: something must be handed in, submitted, uploaded or registered for by then".into()],
        ),
        option("event", &["an exam, test, quiz, presentation, excursion or workshop takes place then".into()]),
        option("other", &["anything else: a lecture, a publication or version date, a holiday, a past event".into()]),
    ];
    let instructions = "What is the date in this sentence from a university course document?";
    sentences
        .iter()
        .map(|sentence| {
            let p = probabilities(instructions, &options, sentence)?;
            let (best, sure) = p.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1))?;
            let kind = [DateKind::Due, DateKind::Event, DateKind::Other][best];
            Some((kind, f64::from(*sure)))
        })
        .collect()
}

/// For each sentence with a date (known to be a hand-in, or an appointment): which of the
/// `categories` (id, what such a date is) it is, when Laya is clearly sure.
pub fn date_categories(sentences: &[String], categories: &[(String, String)]) -> Option<Vec<Option<String>>> {
    if !available() {
        return None;
    }
    let instructions = "Which kind of date is in this sentence from a university course document?";
    Some(sentences.iter().map(|s| pick_one(instructions, categories, None, s, GAPS).map(|(id, _)| id)).collect())
}

/// The token ids the question is made of (as the model was trained):
/// `[CLS] choice question: … [SEP] [MASK] option … [MASK] option … [SEP] state [SEP]`.
/// Returns the ids and where each option's `[MASK]` is.
fn build_sequence(
    encode: impl Fn(&str) -> Vec<u32>,
    special: &Special,
    instructions: &str,
    options: &[String],
    state: &str,
    max_len: usize,
    head_max_len: usize,
) -> (Vec<u32>, Vec<usize>) {
    let clean = |s: &str| s.replace("<mask>", " ");
    let mut head = encode(&format!("choice question: {}", clean(instructions)));
    let mut opts: Vec<Vec<u32>> = options
        .iter()
        .map(|o| {
            let mut ids = vec![special.mask];
            ids.extend(encode(&format!(" {}", clean(o))).into_iter().take(MAX_OPTION_TOKENS));
            ids
        })
        .collect();
    let used = |opts: &[Vec<u32>]| opts.iter().map(Vec::len).sum::<usize>() as isize;
    let mut budget = head_max_len as isize - used(&opts);
    if budget < 16 {
        // Too many or too long options: shrink every option text evenly.
        let per = 4.max((head_max_len.saturating_sub(16)) / opts.len().max(1));
        opts.iter_mut().for_each(|o| o.truncate(per));
        budget = head_max_len as isize - used(&opts);
    }
    head.truncate(budget.max(8) as usize);
    let mut ids = vec![special.cls];
    ids.extend(head);
    ids.push(special.sep);
    let mut markers = Vec::new();
    for o in opts {
        markers.push(ids.len());
        ids.extend(o);
    }
    ids.push(special.sep);
    let room = max_len.saturating_sub(ids.len() + 1);
    ids.extend(encode(&clean(state)).into_iter().take(room));
    ids.push(special.sep);
    ids.truncate(max_len);
    markers.retain(|m| *m < max_len);
    (ids, markers)
}

struct Special {
    cls: u32,
    sep: u32,
    mask: u32,
}

#[derive(Deserialize)]
struct AgentConfig {
    head_layers: usize,
    max_len: usize,
    head_max_len: usize,
    #[serde(default)]
    temperature: Vec<f32>,
    #[serde(default)]
    temperature_by_options: HashMap<String, f32>,
}

/// The parts of the encoder's `config.json` (a newer layout than candle reads).
#[derive(Deserialize)]
struct EncoderConfig {
    vocab_size: usize,
    hidden_size: usize,
    num_hidden_layers: usize,
    num_attention_heads: usize,
    intermediate_size: usize,
    max_position_embeddings: usize,
    norm_eps: f64,
    pad_token_id: u32,
    global_attn_every_n_layers: usize,
    local_attention: usize,
    rope_parameters: HashMap<String, Rope>,
}

#[derive(Deserialize)]
struct Rope {
    rope_theta: f64,
}

/// One PyTorch `TransformerEncoderLayer` (norm first, ReLU).
struct HeadLayer {
    in_proj: Linear,
    out_proj: Linear,
    linear1: Linear,
    linear2: Linear,
    norm1: LayerNorm,
    norm2: LayerNorm,
    heads: usize,
}

impl HeadLayer {
    fn load(vb: VarBuilder, d: usize) -> candle_core::Result<Self> {
        let attn = vb.pp("self_attn");
        let in_proj = Linear::new(attn.get((3 * d, d), "in_proj_weight")?, Some(attn.get(3 * d, "in_proj_bias")?));
        Ok(HeadLayer {
            in_proj,
            out_proj: candle_nn::linear(d, d, attn.pp("out_proj"))?,
            linear1: candle_nn::linear(d, 4 * d, vb.pp("linear1"))?,
            linear2: candle_nn::linear(4 * d, d, vb.pp("linear2"))?,
            norm1: candle_nn::layer_norm(d, 1e-5, vb.pp("norm1"))?,
            norm2: candle_nn::layer_norm(d, 1e-5, vb.pp("norm2"))?,
            heads: (d / 64).max(1),
        })
    }

    fn forward(&self, x: &Tensor) -> candle_core::Result<Tensor> {
        let (b, len, d) = x.dims3()?;
        let hd = d / self.heads;
        let qkv = self
            .norm1
            .forward(x)?
            .apply(&self.in_proj)?
            .reshape((b, len, 3, self.heads, hd))?
            .permute((2, 0, 3, 1, 4))?;
        let (q, k, v) = (qkv.get(0)?.contiguous()?, qkv.get(1)?.contiguous()?, qkv.get(2)?.contiguous()?);
        let att = (q * (hd as f64).powf(-0.5))?.matmul(&k.t()?)?;
        let att = candle_nn::ops::softmax_last_dim(&att)?.matmul(&v)?;
        let att = att.transpose(1, 2)?.reshape((b, len, d))?.apply(&self.out_proj)?;
        let x = (x + att)?;
        let ff = self.norm2.forward(&x)?.apply(&self.linear1)?.relu()?.apply(&self.linear2)?;
        x + ff
    }
}

struct Laya {
    encoder: ModernBert,
    head: Vec<HeadLayer>,
    type_emb: Embedding,
    scorer_norm: LayerNorm,
    scorer_in: Linear,
    scorer_out: Linear,
    tokenizer: tokenizers::Tokenizer,
    special: Special,
    config: AgentConfig,
}

impl Laya {
    fn load(dir: &Path) -> Result<Laya, String> {
        let read = |name: &str| fs::read_to_string(dir.join(name)).map_err(|e| format!("{name}: {e}"));
        let config: AgentConfig = serde_json::from_str(&read("rl_agent_config.json")?).map_err(|e| e.to_string())?;
        let enc: EncoderConfig = serde_json::from_str(&read("encoder-config.json")?).map_err(|e| e.to_string())?;
        let theta = |kind: &str| enc.rope_parameters.get(kind).map_or(160_000.0, |r| r.rope_theta);
        let enc_config = modernbert::Config {
            vocab_size: enc.vocab_size,
            hidden_size: enc.hidden_size,
            num_hidden_layers: enc.num_hidden_layers,
            num_attention_heads: enc.num_attention_heads,
            intermediate_size: enc.intermediate_size,
            max_position_embeddings: enc.max_position_embeddings,
            layer_norm_eps: enc.norm_eps,
            pad_token_id: enc.pad_token_id,
            global_attn_every_n_layers: enc.global_attn_every_n_layers,
            global_rope_theta: theta("full_attention"),
            local_attention: enc.local_attention,
            local_rope_theta: theta("sliding_attention"),
            classifier_config: None,
        };
        let mut tokenizer = tokenizers::Tokenizer::from_file(dir.join("tokenizer.json")).map_err(|e| e.to_string())?;
        // A tokenizer saved by laya's training can still carry the 48-token cut it uses for
        // option texts; the file's text must not be cut that way.
        tokenizer.with_truncation(None).map_err(|e| e.to_string())?;
        tokenizer.with_padding(None);
        let id = |t: &str| tokenizer.token_to_id(t).ok_or(format!("the tokenizer has no {t}"));
        let special = Special { cls: id("<bos>")?, sep: id("<eos>")?, mask: id("<mask>")? };
        // shortcut: F32 needs about 1.3 GB of memory (the file is F16); keep the 256k-token embedding in F16 if that matters.
        // SAFETY: the file is ours and not changed while it is mapped (only during loading).
        let vb =
            unsafe { VarBuilder::from_mmaped_safetensors(&[dir.join("model.safetensors")], DType::F32, &Device::Cpu) }
                .map_err(|e| e.to_string())?;
        let d = enc.hidden_size;
        let build = || -> candle_core::Result<Laya> {
            // candle names the encoder's weights `model.…`, Laya saved them as `encoder.…`.
            let encoder_vb = vb.clone().rename_f(|n| n.replacen("model.", "encoder.", 1));
            let head: candle_core::Result<Vec<HeadLayer>> =
                (0..config.head_layers).map(|i| HeadLayer::load(vb.pp(format!("head.layers.{i}")), d)).collect();
            Ok(Laya {
                encoder: ModernBert::load(encoder_vb, &enc_config)?,
                head: head?,
                type_emb: candle_nn::embedding(3, d, vb.pp("type_emb"))?,
                scorer_norm: candle_nn::layer_norm(d, 1e-5, vb.pp("scorer.0"))?,
                scorer_in: candle_nn::linear(d, d, vb.pp("scorer.1"))?,
                scorer_out: candle_nn::linear(d, 1, vb.pp("scorer.3"))?,
                tokenizer: tokenizer.clone(),
                special,
                config,
            })
        };
        build().map_err(|e| e.to_string())
    }

    fn encode(&self, text: &str) -> Vec<u32> {
        self.tokenizer.encode(text, false).map(|e| e.get_ids().to_vec()).unwrap_or_default()
    }

    /// The probability of each option, or None if not all of them fit into the question.
    fn choose(&self, instructions: &str, options: &[String], state: &str) -> candle_core::Result<Option<Vec<f32>>> {
        let (ids, markers) = build_sequence(
            |t| self.encode(t),
            &self.special,
            instructions,
            options,
            state,
            self.config.max_len,
            self.config.head_max_len,
        );
        if markers.len() != options.len() {
            return Ok(None);
        }
        let dev = Device::Cpu;
        let input = Tensor::new(ids.as_slice(), &dev)?.unsqueeze(0)?;
        let mask = input.ones_like()?;
        let h = self.encoder.forward(&input, &mask)?;
        // Question type 0: choice.
        let mut h = h.broadcast_add(&self.type_emb.embeddings().i(0)?)?;
        for layer in &self.head {
            h = layer.forward(&h)?;
        }
        let at = Tensor::new(markers.iter().map(|m| *m as u32).collect::<Vec<_>>().as_slice(), &dev)?;
        let m = h.squeeze(0)?.index_select(&at, 0)?;
        let m = self.scorer_norm.forward(&m)?.apply(&self.scorer_in)?.gelu_erf()?.apply(&self.scorer_out)?;
        let logits: Vec<f32> = m.squeeze(D::Minus1)?.to_vec1()?;
        let k = logits.len();
        let bucket = match k {
            ..=2 => "choice:2",
            3..=5 => "choice:3-5",
            6..=10 => "choice:6-10",
            _ => "choice:11+",
        };
        let t = self
            .config
            .temperature_by_options
            .get(bucket)
            .copied()
            .unwrap_or_else(|| self.config.temperature.first().copied().unwrap_or(1.0));
        Ok(Some(softmax(&logits, t)))
    }
}

fn softmax(logits: &[f32], temperature: f32) -> Vec<f32> {
    let z: Vec<f32> = logits.iter().map(|l| l / temperature).collect();
    let max = z.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    let e: Vec<f32> = z.iter().map(|v| (v - max).exp()).collect();
    let sum: f32 = e.iter().sum();
    e.iter().map(|v| v / sum).collect()
}

/// Files of other faculties and the module they belong to (for checking the real model).
pub const REAL_CASES: [([&str; 4], &str, &str, usize); 3] = [
    (
        ["Strafrecht AT", "Privatrecht I", "Staatsrecht", "Römisches Recht"],
        "Fall_7.pdf",
        "Fall 7: A schlägt B mit einem Stock. Prüfen Sie die Strafbarkeit des A wegen Körperverletzung (§ 223 StGB). Vorsatz, Rechtswidrigkeit, Schuld.",
        0,
    ),
    (
        ["Anatomie I", "Biochemie", "Physiologie", "Medizinische Psychologie"],
        "VL05.pdf",
        "Vorlesung 5: Der Herzzyklus. Systole und Diastole, Druck-Volumen-Diagramm, Herzminutenvolumen, Frank-Starling-Mechanismus.",
        2,
    ),
    (
        ["Ancient History", "Medieval History", "Modern European History", "Historiography"],
        "week3_reading.pdf",
        "The Investiture Controversy: Gregory VII, Henry IV and the road to Canossa (1077). Read the Dictatus Papae before the seminar.",
        1,
    ),
];

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::sorter::tests::test_config;

    type Answer = fn(&str, &[String], &str) -> Vec<f32>;

    thread_local! {
        /// A stand-in for the model on this test's thread (none: Laya isn't there).
        pub static FAKE: std::cell::Cell<Option<Answer>> = const { std::cell::Cell::new(None) };
    }

    /// Answer with `answer` instead of the model on the calling test's thread.
    pub fn fake(answer: Answer) {
        FAKE.with(|f| f.set(Some(answer)));
    }

    /// Sure (90 %) of the option the text clearly points to: one whose «name» or typical words
    /// it mentions, else a hand-in («Abgabe», «rendre», «Aufgabe 1») or an exam/event, a
    /// module about «Varianz» for Statistik; else «none» if offered, else not sure at all.
    pub fn stand_in(instructions: &str, options: &[String], state: &str) -> Vec<f32> {
        let low = state.to_lowercase();
        let parts: Vec<(String, String)> = options
            .iter()
            .map(|o| {
                let (id, about) = o.split_once(": ").unwrap_or((o, ""));
                (id.to_lowercase(), about.to_lowercase())
            })
            .collect();
        let named = |about: &str| {
            let mut words: Vec<&str> = Vec::new();
            if let Some((_, typical)) = about.split_once("typical words: ") {
                words.extend(typical.split(", "));
            }
            if let Some(name) = about.split('«').nth(1).and_then(|n| n.split('»').next()) {
                words.push(name);
            }
            words.iter().any(|w| w.len() > 2 && low.contains(w))
        };
        let leading = |markers: &[&str]| {
            parts.iter().position(|(id, about)| markers.iter().any(|m| id == m || about.starts_with(m)))
        };
        let pick = if instructions == INSTRUCTIONS {
            low.contains("varianz").then(|| parts.iter().position(|(id, _)| id.contains("statistik"))).flatten()
        } else if instructions == SUBFOLDER_QUESTION {
            // The first folder one of whose words the text has.
            parts.iter().position(|(_, words)| words.split(", ").any(|w| w.len() > 3 && low.contains(w)))
        } else {
            parts.iter().position(|(id, about)| id != NONE && named(about)).or_else(|| {
                if ["abgabe", "due", "rendre", "aufgabe 1"].iter().any(|w| low.contains(w)) {
                    leading(&["due", "exercise", "an exercise"])
                } else if ["prüfung", "exam", "exkursion"].iter().any(|w| low.contains(w)) {
                    leading(&["event", "exam", "an exam"])
                } else {
                    leading(&[NONE, "other"])
                }
            })
        };
        let n = options.len();
        match pick {
            Some(i) => (0..n).map(|k| if k == i { 0.9 } else { 0.1 / (n - 1).max(1) as f32 }).collect(),
            None => vec![1.0 / n as f32; n],
        }
    }

    const SPECIAL: Special = Special { cls: 1, sep: 2, mask: 4 };

    /// One token per word, numbered by length so they're easy to tell apart.
    fn words(t: &str) -> Vec<u32> {
        t.split_whitespace().map(|w| 100 + w.len() as u32).collect()
    }

    #[test]
    fn question_has_a_marker_before_each_option() {
        let options = vec!["Analysis".to_string(), "Lineare Algebra: Matrix".to_string()];
        let (ids, markers) = build_sequence(words, &SPECIAL, INSTRUCTIONS, &options, "File name: Blatt 3", 1024, 256);
        assert_eq!(markers.len(), 2);
        assert!(markers.iter().all(|m| ids[*m] == SPECIAL.mask));
        assert_eq!(ids[0], SPECIAL.cls);
        // [CLS] "choice question: Which …" (10 words) [SEP] then the first option.
        assert_eq!(markers[0], 12);
        assert_eq!(markers[1], 14);
        assert_eq!(*ids.last().unwrap(), SPECIAL.sep);
        assert_eq!(ids.len(), 14 + 4 + 1 + 4 + 1);
    }

    #[test]
    fn long_options_shrink_and_long_text_is_cut() {
        let long = "word ".repeat(60);
        let options: Vec<String> = (0..10).map(|_| long.clone()).collect();
        let (ids, markers) = build_sequence(words, &SPECIAL, INSTRUCTIONS, &options, &long.repeat(100), 1024, 256);
        assert_eq!(markers.len(), 10);
        // Every option shrinks to (256 - 16) / 10 = 24 tokens, marker included.
        assert_eq!(markers[1] - markers[0], 24);
        assert_eq!(ids.len(), 1024);
        assert_eq!(*ids.last().unwrap(), SPECIAL.sep);
    }

    #[test]
    fn answers_only_when_clearly_sure() {
        assert_eq!(confident(&[0.1, 0.8, 0.1], (0.7, 2.0)), Some((1, 80)));
        assert_eq!(confident(&[0.5, 0.3, 0.2], (0.7, 2.0)), None);
        assert_eq!(confident(&[0.72, 0.28], (0.7, 3.0)), None);
        assert_eq!(confident(&[], (0.7, 2.0)), None);
        // Sorting alone: lower – Canossa (60 % / 16 %) is sorted, the heart cycle (39 % / 30 %) not.
        assert_eq!(confident(&[0.16, 0.60, 0.14, 0.09], ALONE), Some((1, 60)));
        assert_eq!(confident(&[0.39, 0.11, 0.30, 0.20], ALONE), None);
        let p = softmax(&[1.0, 2.0, 3.0], 1.0);
        assert!((p.iter().sum::<f32>() - 1.0).abs() < 1e-6 && p[2] > p[1]);
        assert_eq!(Manifest::get().files.len(), 4);
    }

    #[test]
    fn without_the_model_nothing_is_asked() {
        let tmp = tempfile::tempdir().unwrap();
        let config = test_config(tmp.path());
        assert!(!Place::new(tmp.path()).installed());
        assert_eq!(module_of(&config, "Blatt 3.pdf", ""), None);
        assert_eq!(date_kinds(&["Abgabe: 3.11.".into()]), None);
    }

    #[test]
    fn the_model_is_there_once_its_files_are_and_older_ones_go() {
        let tmp = tempfile::tempdir().unwrap();
        let place = Place::new(tmp.path());
        let dir = place.model_dir();
        fs::create_dir_all(&dir).unwrap();
        assert!(!place.installed());
        for f in Manifest::get().files {
            fs::write(dir.join(&f.name), "x").unwrap();
        }
        assert!(place.installed());
        assert_eq!(place.progress().0, Manifest::get().size());
        // An older model and the Laya of File Sorter 0.15 with its Python go; nothing else.
        fs::create_dir_all(place.root.join("laya-uni-0")).unwrap();
        fs::write(place.root.join("laya-uni-0").join("model.safetensors"), "old").unwrap();
        fs::create_dir_all(place.old.join("venv").join("bin")).unwrap();
        fs::write(place.old.join("venv").join("bin").join("python"), "py").unwrap();
        fs::create_dir_all(tmp.path().join("notes")).unwrap();
        assert_eq!(place.tidy(), 5);
        assert!(place.installed() && !place.old.exists() && !place.root.join("laya-uni-0").exists());
        assert!(tmp.path().join("notes").exists());
        place.remove().unwrap();
        assert!(!place.root.exists());
    }

    /// With the model there: a document the keywords don't decide is sorted by it, a French
    /// deadline the rules don't know is found, and a subfolder is chosen by its text.
    #[test]
    fn sorting_deadlines_and_subfolders_ask_the_model() {
        use crate::config::{Config, Course};
        use crate::sorter::Outcome;
        let tmp = tempfile::tempdir().unwrap();
        fake(stand_in);
        // Sorting.
        let mut cfg = test_config(tmp.path());
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
            ("Vorlesung".to_string(), "vorlesung, skript".to_string()),
            ("Übungen".to_string(), "übungsblatt, aufgabe".to_string()),
        ];
        let picked = subfolder_of("scan_0042.pdf", "Blatt 4. Aufgabe 1: Zeigen Sie …", &options);
        assert_eq!(picked.map(|p| p.0).as_deref(), Some("Übungen"));
        assert_eq!(subfolder_of("info.pdf", "Willkommen im Kurs", &options), None);
    }

    /// With Laya on, it alone decides module and subfolder; without it, the keywords and rules.
    #[test]
    fn with_laya_on_only_laya_sorts() {
        use crate::config::Course;
        use crate::sorter::LeftReason;
        let tmp = tempfile::tempdir().unwrap();
        fake(stand_in);
        let mut cfg = test_config(tmp.path());
        cfg.courses.push(Course::new("Statistik", &["stat101"]));
        cfg.features.subfolders = true;
        cfg.features.laya = true;
        let module = |cfg: &Config, file: &Path| {
            crate::sorter::choose_course_for_test(file, cfg).map(|m| cfg.courses[m.course].label())
        };
        // The name says «programming», the text is about statistics: Laya decides.
        let file = cfg.watch_path().join("programming_kapitel4.txt");
        fs::write(&file, "Varianzanalyse und Regression: Schätzung der Parameter.").unwrap();
        assert_eq!(module(&cfg, &file).as_deref(), Ok("Statistik"));
        // Laya not sure of any module: it stays, though its name says «programming».
        let other = cfg.watch_path().join("programming_week2.txt");
        fs::write(&other, "Loops and functions.").unwrap();
        assert_eq!(module(&cfg, &other), Err(LeftReason::NoMatch));
        // Laya off: the keywords.
        cfg.features.laya = false;
        assert_eq!(module(&cfg, &file).as_deref(), Ok("Programming Course"));
        assert_eq!(module(&cfg, &other).as_deref(), Ok("Programming Course"));
        // Subfolders: the name says «Lösung», the text is an exercise sheet.
        let sheet = cfg.watch_path().join("Loesung_Blatt3.txt");
        fs::write(&sheet, "Blatt 3. Aufgabe 1: Zeigen Sie, dass die Folge konvergiert.").unwrap();
        let folder = |cfg: &Config| crate::sorter::subfolder_for(cfg, tmp.path(), &sheet, None);
        assert_eq!(folder(&cfg).as_deref(), Some("Lösungen"));
        cfg.features.laya = true;
        assert_eq!(folder(&cfg).as_deref(), Some("Übungen"));
        // Laya not sure of a folder: the module's folder itself, whatever the name says.
        let info = cfg.watch_path().join("info_3.txt");
        fs::write(&info, "Willkommen im Kurs.").unwrap();
        assert_eq!(crate::sorter::subfolder_for(&cfg, tmp.path(), &info, None), None);
    }

    /// Between two modules Laya asks; when nothing stands out, nothing is asked.
    #[test]
    fn laya_asks_only_between_two() {
        fake(|_, options, _| match options.len() {
            3 => vec![0.45, 0.40, 0.15],
            _ => vec![1.0 / options.len() as f32; options.len()],
        });
        let tmp = tempfile::tempdir().unwrap();
        let mut cfg = test_config(tmp.path());
        let between = module_of(&cfg, "x.pdf", "");
        assert_eq!(between, Some(ModuleChoice::Between(vec![cfg.courses[0].label(), cfg.courses[1].label()])));
        cfg.courses.push(crate::config::Course::new("Statistik", &[]));
        assert_eq!(module_of(&cfg, "x.pdf", ""), Some(ModuleChoice::Nothing));
    }

    /// Downloads the model (about 680 MB, into FILE_SORTER_LAYA_DIR or a temporary folder) and
    /// asks it about files of faculties other than computer science. (The heart-cycle lecture
    /// leans to Anatomie at 39 %: not sure enough to sort, which is what this checks.)
    #[test]
    #[ignore]
    fn downloads_and_picks_obvious_modules_of_any_faculty() {
        let root = std::env::var_os("FILE_SORTER_LAYA_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| std::env::temp_dir().join("file-sorter-laya-uni"));
        let place = Place::new(&root);
        install(&place, |_, _| {}).unwrap();
        let model = Laya::load(&place.model_dir()).unwrap();
        for (modules, name, text, expected) in REAL_CASES {
            let options: Vec<String> = modules.iter().map(|m| m.to_string()).collect();
            let p = model.choose(INSTRUCTIONS, &options, &state_for(name, text)).unwrap().unwrap();
            println!("{name}: {p:?}");
            // Right, or unsure (then the keywords decide) – never sure of the wrong module.
            let sure = confident(&p, ALONE);
            assert!(sure.is_none_or(|(best, _)| best == expected), "{name}: {p:?}");
        }
    }
}
