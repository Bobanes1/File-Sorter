//! Optional: let a decision model choose the module from a file's name and text, instead of
//! counting keyword hits. The model is laya-uni: Laya (convaiinnovations/laya, multilingual
//! checkpoint) fine-tuned on course files of every faculty (`training/laya-uni`). It runs on
//! this computer; only the model (about 680 MB) is downloaded, into the state folder, and
//! again when a newer one is published (`keep_up`).
//! When the model isn't there, isn't sure, or the modules don't fit into its question,
//! the keywords decide as before.
//!
//! The model is a ModernBERT encoder plus a small decision head: each module is one option
//! behind a `[MASK]` marker, and the head scores the markers. This is a port of the model's
//! own `rl_common.py` / `rl_agent_api.py` (choice questions only).

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use candle_core::{D, DType, Device, IndexOp, Module, Tensor};
use candle_nn::{Embedding, LayerNorm, Linear, VarBuilder};
use candle_transformers::models::modernbert::{self, ModernBert};
use serde::{Deserialize, Serialize};

use crate::config::{Config, Course};

/// The newest model's manifest: GitHub sends this to the latest release of NilsBriggen/laya-uni.
/// Each model is its own release (laya-uni-1, -2, …) whose files are never replaced.
const MANIFEST_URL: &str = "https://github.com/NilsBriggen/laya-uni/releases/latest/download/manifest.json";
/// The manifest of laya-uni-1, for when the newest one can't be read on the first download.
const BUILT_IN: &str = r#"{"name": "laya-uni-1", "version": 1, "min_probability": 0.7, "min_lead": 2.0, "files": [
 {"name": "rl_agent_config.json", "size": 4987, "sha256": "c06f9dff24ed9f1132e5e44020381d380d3b5ea30e650f90a82ff6378215c6ff",
  "url": "https://github.com/NilsBriggen/laya-uni/releases/download/laya-uni-1/rl_agent_config.json"},
 {"name": "encoder-config.json", "size": 1939, "sha256": "36b7d4898274ed6f06ce9c1f0e34034d8bd39295ab232d7cabc41a3eac821e5f",
  "url": "https://github.com/NilsBriggen/laya-uni/releases/download/laya-uni-1/encoder-config.json"},
 {"name": "tokenizer.json", "size": 34363188, "sha256": "609d8f4c067cd3950f88594c5a802616cea245823836ef5848ee4fc40aab5b6f",
  "url": "https://github.com/NilsBriggen/laya-uni/releases/download/laya-uni-1/tokenizer.json"},
 {"name": "model.safetensors", "size": 643835524, "sha256": "2a73a52757ff1557d59abcdb43db469327df077146ce68140292a943572925bb",
  "url": "https://github.com/NilsBriggen/laya-uni/releases/download/laya-uni-1/model.safetensors"}]}"#;
/// The files a model consists of (each manifest lists them with its own URLs and checksums).
const FILE_NAMES: [&str; 4] = ["rl_agent_config.json", "encoder-config.json", "tokenizer.json", "model.safetensors"];
/// Which model is in use, next to the models' folders.
const INSTALLED: &str = "installed.json";
/// Where the first, not fine-tuned model was kept (removed once laya-uni is there).
const OLD_DIR: &str = "laya";
/// How often to look for a newer model.
const CHECK_EVERY: std::time::Duration = std::time::Duration::from_secs(24 * 60 * 60);
pub const INSTRUCTIONS: &str = "Which university module does this file belong to?";
/// Enough text for the model's 1024 tokens; tokenizing a whole book would only cost time.
const MAX_TEXT_CHARS: usize = 6000;
/// Option texts are cut to this many tokens (as in training).
const MAX_OPTION_TOKENS: usize = 48;

/// One published model: its files and the confidence it needs before it sorts a file
/// (chosen per model on its validation rows, see training/laya-uni).
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct Manifest {
    name: String,
    version: u32,
    /// The answer must be at least this likely,
    min_probability: f32,
    /// and this many times as likely as the second.
    min_lead: f32,
    files: Vec<Asset>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
struct Asset {
    name: String,
    url: String,
    size: u64,
    sha256: String,
}

impl Manifest {
    fn built_in() -> Manifest {
        serde_json::from_str(BUILT_IN).expect("the built-in manifest is valid")
    }

    /// Only what the app can use: the four files, from GitHub, over https.
    fn checked(self) -> Result<Manifest, String> {
        let mut names: Vec<&str> = self.files.iter().map(|f| f.name.as_str()).collect();
        names.sort_unstable();
        let mut expected = FILE_NAMES.to_vec();
        expected.sort_unstable();
        let safe_name = !self.name.is_empty()
            && self.name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.')
            && !self.name.starts_with('.');
        if names != expected || !safe_name {
            return Err(format!("the manifest of {} doesn't describe a model", self.name));
        }
        if let Some(f) = self.files.iter().find(|f| !f.url.starts_with("https://github.com/")) {
            return Err(format!("{} is not on GitHub: {}", f.name, f.url));
        }
        Ok(self)
    }

    fn size(&self) -> u64 {
        self.files.iter().map(|f| f.size).sum()
    }
}

/// About how much the model download is (for the switch's hint).
pub fn total_size() -> u64 {
    Manifest::built_in().size()
}

/// Where the models are kept: one folder per model, plus `installed.json`.
pub fn dir(config: &Config) -> Option<PathBuf> {
    config.state_file("laya-uni")
}

/// The model in use, if its files are all there.
fn installed(root: &Path) -> Option<Manifest> {
    let manifest: Manifest = serde_json::from_str(&fs::read_to_string(root.join(INSTALLED)).ok()?).ok()?;
    let manifest = manifest.checked().ok()?;
    manifest.files.iter().all(|f| root.join(&manifest.name).join(&f.name).is_file()).then_some(manifest)
}

pub enum Status {
    /// Not downloaded (yet).
    Missing,
    /// Downloading, with the bytes done so far and the whole size. `Ready` still holds the
    /// model in use while a newer one comes.
    Downloading(u64, u64),
    Ready,
    Failed(String),
}

static DOWNLOADING: AtomicBool = AtomicBool::new(false);
/// The model being downloaded, and where to (for the progress bar).
static TARGET: Mutex<Option<(PathBuf, Manifest)>> = Mutex::new(None);
/// When `keep_up` looked last.
static LAST_LOOK: Mutex<Option<std::time::Instant>> = Mutex::new(None);
/// When a newer model was looked for last.
static LAST_CHECK: Mutex<Option<std::time::Instant>> = Mutex::new(None);
/// Why the first download or the loading failed (until switched off and on again). A failed
/// update is only logged: the model in use goes on working.
static ERROR: Mutex<Option<String>> = Mutex::new(None);
static CACHE: Mutex<Option<(PathBuf, Arc<Laya>)>> = Mutex::new(None);

fn error() -> Option<String> {
    ERROR.lock().ok().and_then(|e| e.clone())
}

fn set_error(message: Option<String>) {
    if let Ok(mut e) = ERROR.lock() {
        *e = message;
    }
}

pub fn status(root: Option<&Path>) -> Status {
    let Some(root) = root else { return Status::Missing };
    if DOWNLOADING.load(Ordering::Relaxed)
        && let Some((dir, manifest)) = TARGET.lock().ok().and_then(|t| t.clone())
    {
        return Status::Downloading(progress(&dir, &manifest), manifest.size());
    }
    if let Some(e) = error() {
        return Status::Failed(e);
    }
    if installed(root).is_some() { Status::Ready } else { Status::Missing }
}

fn progress(dir: &Path, manifest: &Manifest) -> u64 {
    let size = |p: PathBuf| fs::metadata(p).map(|m| m.len()).unwrap_or(0);
    manifest
        .files
        .iter()
        .map(|f| {
            let file = dir.join(&f.name);
            if file.is_file() { f.size } else { size(partial(&file)) }
        })
        .sum()
}

fn partial(file: &Path) -> PathBuf {
    file.with_extension("partial")
}

/// Called now and then while the switch is on: fetch the model if it isn't there, and once a
/// day (if updates may be looked for) a newer one, in the background. The one in use keeps
/// sorting until the newer one is complete and checked.
pub fn keep_up(root: PathBuf, look_for_updates: bool, now: bool) {
    if DOWNLOADING.load(Ordering::Relaxed) || crate::lean::active() {
        return;
    }
    // Every few seconds is plenty (it is called for every frame of the window).
    let quiet = LAST_LOOK.lock().is_ok_and(|mut last| {
        let quiet = !now && last.is_some_and(|t| t.elapsed() < std::time::Duration::from_secs(5));
        if !quiet {
            *last = Some(std::time::Instant::now());
        }
        quiet
    });
    if quiet {
        return;
    }
    let current = installed(&root);
    if current.is_none() {
        if error().is_none() {
            start(root, None);
        }
        return;
    }
    let due = LAST_CHECK.lock().is_ok_and(|mut last| {
        let due = last.is_none_or(|t| t.elapsed() >= CHECK_EVERY);
        if due {
            *last = Some(std::time::Instant::now());
        }
        due
    });
    if look_for_updates && due {
        start(root, current);
    }
}

/// Download the newest model (or, the first time and offline, laya-uni-1) and put it in use.
fn start(root: PathBuf, current: Option<Manifest>) {
    if DOWNLOADING.swap(true, Ordering::Relaxed) {
        return;
    }
    std::thread::spawn(move || {
        let first = current.is_none();
        match update(&root, current) {
            Err(e) if first => {
                log::info!("Could not download the Laya model: {e}");
                set_error(Some(e));
            }
            Err(e) => log::info!("Could not update the Laya model: {e}"),
            Ok(()) => {}
        }
        if let Ok(mut t) = TARGET.lock() {
            *t = None;
        }
        DOWNLOADING.store(false, Ordering::Relaxed);
    });
}

/// Put the newest model in use if it is newer than `current` (the first time, when the newest
/// can't be read, laya-uni-1).
fn update(root: &Path, current: Option<Manifest>) -> Result<(), String> {
    let newest = match fetch_manifest() {
        Ok(m) => m,
        Err(e) if current.is_none() => {
            log::info!("Could not read the newest Laya model's manifest ({e}); taking laya-uni-1");
            Manifest::built_in()
        }
        Err(e) => return Err(e),
    };
    if current.as_ref().is_some_and(|c| c.version >= newest.version) {
        return Ok(());
    }
    install(root, &newest)
}

fn fetch_manifest() -> Result<Manifest, String> {
    let fetched = crate::update::curl()
        .args(["--max-time", "30"])
        .arg(MANIFEST_URL)
        .output()
        .map_err(|e| format!("curl could not be started: {e}"))?;
    if !fetched.status.success() {
        return Err(String::from_utf8_lossy(&fetched.stderr).trim().to_string());
    }
    let manifest: Manifest = serde_json::from_slice(&fetched.stdout).map_err(|e| e.to_string())?;
    manifest.checked()
}

/// Download a model into its own folder, then make it the one in use and remove the others.
fn install(root: &Path, manifest: &Manifest) -> Result<(), String> {
    let dir = root.join(&manifest.name);
    if let Ok(mut t) = TARGET.lock() {
        *t = Some((dir.clone(), manifest.clone()));
    }
    download(&dir, manifest)?;
    let tmp = root.join(format!("{INSTALLED}.partial"));
    fs::write(&tmp, serde_json::to_string_pretty(manifest).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    fs::rename(&tmp, root.join(INSTALLED)).map_err(|e| e.to_string())?;
    log::info!("Laya model {} is in use", manifest.name);
    remove_other_models(root, &manifest.name);
    Ok(())
}

/// Download the files that are missing, each checked against its SHA-256. An interrupted
/// download goes on where it stopped.
fn download(dir: &Path, manifest: &Manifest) -> Result<(), String> {
    fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    for asset in &manifest.files {
        let file = dir.join(&asset.name);
        if file.is_file() {
            continue;
        }
        let part = partial(&file);
        let fetched = crate::update::curl()
            .args(["--max-time", "7200", "--continue-at", "-", "--output"])
            .arg(&part)
            .arg(&asset.url)
            .output()
            .map_err(|e| format!("curl could not be started: {e}"))?;
        if !fetched.status.success() {
            return Err(String::from_utf8_lossy(&fetched.stderr).trim().to_string());
        }
        let got = crate::update::sha256_of(&part).map_err(|e| e.to_string())?;
        if got != asset.sha256 {
            let _ = fs::remove_file(&part);
            return Err(format!("the download of {} is not the published one (SHA-256 {got})", asset.name));
        }
        fs::rename(&part, &file).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Older models (folders next to the one in use, files of the first layout directly in the
/// folder, and the not fine-tuned model): only their own files, and a folder only if nothing
/// else is in it.
fn remove_other_models(root: &Path, keep: &str) {
    let remove_files = |dir: &Path| {
        for name in FILE_NAMES {
            let _ = fs::remove_file(dir.join(name));
            let _ = fs::remove_file(partial(&dir.join(name)));
        }
    };
    remove_files(root);
    for entry in fs::read_dir(root).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.is_dir() && entry.file_name() != keep {
            remove_files(&path);
            let _ = fs::remove_dir(&path);
        }
    }
    if let Some(old) = root.parent().map(|p| p.join(OLD_DIR)) {
        remove_files(&old);
        let _ = fs::remove_dir(old);
    }
    // The model in memory is an older one now; the next file loads the new one.
    if let Ok(mut cache) = CACHE.lock()
        && cache.as_ref().is_some_and(|(dir, _)| !dir.ends_with(keep))
    {
        *cache = None;
    }
}

/// Drop the model from memory (switched off, or saving resources in the background).
pub fn forget() {
    if let Ok(mut cache) = CACHE.lock() {
        *cache = None;
    }
    set_error(None);
}

fn load(dir: &Path) -> Option<Arc<Laya>> {
    let mut cache = CACHE.lock().ok()?;
    if let Some((loaded, model)) = cache.as_ref()
        && loaded == dir
    {
        return Some(model.clone());
    }
    match Laya::load(dir) {
        Ok(model) => {
            let model = Arc::new(model);
            *cache = Some((dir.to_path_buf(), model.clone()));
            Some(model)
        }
        Err(e) => {
            log::info!("Could not load the Laya model: {e}");
            set_error(Some(e));
            None
        }
    }
}

/// Whether the model can be asked now (cheap, so the text is only read for it when it can).
pub fn usable(config: &Config) -> bool {
    !crate::lean::active()
        && config.courses.len() >= 2
        && error().is_none()
        && dir(config).is_some_and(|d| installed(&d).is_some())
}

/// The module for `path` (with the text read from it, if any) and how sure the model is (%),
/// or None when the keywords should decide.
pub fn guess(config: &Config, path: &Path, text: Option<&str>) -> Option<(usize, u8)> {
    if crate::lean::active() || config.courses.len() < 2 || error().is_some() {
        return None;
    }
    let root = dir(config)?;
    let manifest = installed(&root)?;
    let model = load(&root.join(&manifest.name))?;
    let options: Vec<String> = config.courses.iter().map(option_text).collect();
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let state = state_for(&name, text.unwrap_or(""));
    match model.choose(&options, &state) {
        Ok(Some(p)) => confident(&p, manifest.min_probability, manifest.min_lead),
        Ok(None) => None,
        Err(e) => {
            log::info!("The Laya model failed on {}: {e}", path.display());
            None
        }
    }
}

/// What the model reads about a file (training rows are made the same way: `laya_rows.rs`).
pub fn state_for(name: &str, text: &str) -> String {
    let text: String = text.chars().take(MAX_TEXT_CHARS).collect();
    format!("File name: {name}\n\n{text}")
}

fn option_text(course: &Course) -> String {
    option(&course.label(), &course.keywords)
}

/// "Programmieren: Java, OOP" – a module and its keywords, as one answer option. Laya's
/// own code renders a criterion `{label: keywords}` the same way.
pub fn option(label: &str, keywords: &[String]) -> String {
    if keywords.is_empty() { label.to_string() } else { format!("{label}: {}", keywords.join(", ")) }
}

/// The best option if the model is clearly sure of it: at least `min_probability`, and
/// `min_lead` times as likely as the second.
fn confident(p: &[f32], min_probability: f32, min_lead: f32) -> Option<(usize, u8)> {
    let mut order: Vec<usize> = (0..p.len()).collect();
    order.sort_by(|a, b| p[*b].total_cmp(&p[*a]));
    let (best, second) = (p[*order.first()?], order.get(1).map_or(0.0, |i| p[*i]));
    (best >= min_probability && best >= min_lead * second).then(|| (order[0], (best * 100.0).round() as u8))
}

/// The token ids the question is made of (as the model was trained):
/// `[CLS] choice question: … [SEP] [MASK] option … [MASK] option … [SEP] state [SEP]`.
/// Returns the ids and where each option's `[MASK]` is.
fn build_sequence(
    encode: impl Fn(&str) -> Vec<u32>,
    special: &Special,
    options: &[String],
    state: &str,
    max_len: usize,
    head_max_len: usize,
) -> (Vec<u32>, Vec<usize>) {
    let clean = |s: &str| s.replace("<mask>", " ");
    let mut head = encode(&format!("choice question: {}", clean(INSTRUCTIONS)));
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
    fn choose(&self, options: &[String], state: &str) -> candle_core::Result<Option<Vec<f32>>> {
        let (ids, markers) = build_sequence(
            |t| self.encode(t),
            &self.special,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sorter::tests::test_config;

    const SPECIAL: Special = Special { cls: 1, sep: 2, mask: 4 };

    /// One token per word, numbered by length so they're easy to tell apart.
    fn words(t: &str) -> Vec<u32> {
        t.split_whitespace().map(|w| 100 + w.len() as u32).collect()
    }

    #[test]
    fn question_has_a_marker_before_each_option() {
        let options = vec!["Analysis".to_string(), "Lineare Algebra: Matrix".to_string()];
        let (ids, markers) = build_sequence(words, &SPECIAL, &options, "File name: Blatt 3", 1024, 256);
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
        let (ids, markers) = build_sequence(words, &SPECIAL, &options, &long.repeat(100), 1024, 256);
        assert_eq!(markers.len(), 10);
        // Every option shrinks to (256 - 16) / 10 = 24 tokens, marker included.
        assert_eq!(markers[1] - markers[0], 24);
        assert_eq!(ids.len(), 1024);
        assert_eq!(*ids.last().unwrap(), SPECIAL.sep);
    }

    #[test]
    fn answers_only_when_clearly_sure() {
        assert_eq!(confident(&[0.1, 0.8, 0.1], 0.7, 2.0), Some((1, 80)));
        assert_eq!(confident(&[0.5, 0.3, 0.2], 0.7, 2.0), None);
        assert_eq!(confident(&[0.72, 0.28], 0.7, 3.0), None);
        assert_eq!(confident(&[], 0.7, 2.0), None);
        let p = softmax(&[1.0, 2.0, 3.0], 1.0);
        assert!((p.iter().sum::<f32>() - 1.0).abs() < 1e-6 && p[2] > p[1]);
    }

    #[test]
    fn without_the_model_the_keywords_decide() {
        let tmp = tempfile::tempdir().unwrap();
        let mut config = test_config(tmp.path());
        config.state_dir = Some(tmp.path().join("state"));
        config.features.laya = true;
        assert!(matches!(status(dir(&config).as_deref()), Status::Missing));
        assert_eq!(guess(&config, Path::new("Blatt 3.pdf"), None), None);
    }

    #[test]
    fn only_a_manifest_of_the_four_files_from_github_is_taken() {
        let built_in = Manifest::built_in();
        assert_eq!(built_in.clone().checked().unwrap(), built_in);
        assert_eq!(built_in.min_probability, 0.7);
        let mut elsewhere = built_in.clone();
        elsewhere.files[3].url = "https://example.com/model.safetensors".into();
        assert!(elsewhere.checked().is_err());
        let mut climbing = built_in.clone();
        climbing.name = "../outside".into();
        assert!(climbing.checked().is_err());
        let mut short = built_in;
        short.files.pop();
        assert!(short.checked().is_err());
    }

    #[test]
    fn a_model_is_in_use_once_its_files_and_installed_json_are_there() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("laya-uni");
        let manifest = Manifest::built_in();
        fs::create_dir_all(root.join(&manifest.name)).unwrap();
        fs::write(root.join(INSTALLED), serde_json::to_string(&manifest).unwrap()).unwrap();
        assert_eq!(installed(&root), None);
        for name in FILE_NAMES {
            fs::write(root.join(&manifest.name).join(name), "x").unwrap();
        }
        assert_eq!(installed(&root), Some(manifest));
        assert!(matches!(status(Some(&root)), Status::Ready));
    }

    #[test]
    fn older_models_go_but_nothing_else() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("laya-uni");
        let old = tmp.path().join(OLD_DIR);
        for dir in [root.join("laya-uni-1"), root.join("laya-uni-2"), root.clone(), old.clone()] {
            fs::create_dir_all(&dir).unwrap();
            for name in FILE_NAMES {
                fs::write(dir.join(name), "x").unwrap();
            }
        }
        fs::write(old.join("notes.txt"), "mine").unwrap();
        remove_other_models(&root, "laya-uni-2");
        assert!(FILE_NAMES.iter().all(|n| root.join("laya-uni-2").join(n).is_file()));
        assert!(!root.join("laya-uni-1").exists());
        assert!(FILE_NAMES.iter().all(|n| !root.join(n).exists() && !old.join(n).exists()));
        // A folder with something else in it stays (only our files go).
        assert!(old.join("notes.txt").exists());
    }

    /// Reads the newest manifest, downloads about 680 MB (into FILE_SORTER_LAYA_DIR, or a
    /// temporary folder) and asks the real
    /// model about files of faculties other than computer science. (The heart-cycle lecture
    /// leans to Anatomie at 39 %: not sure enough to sort, which is what this checks.)
    #[test]
    #[ignore]
    fn downloads_and_picks_obvious_modules_of_any_faculty() {
        let root = std::env::var_os("FILE_SORTER_LAYA_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| std::env::temp_dir().join("file-sorter-laya-uni"));
        let newest = fetch_manifest().unwrap();
        install(&root, &newest).unwrap();
        let manifest = installed(&root).unwrap();
        let model = Laya::load(&root.join(&manifest.name)).unwrap();
        let cases = [
            (
                ["Strafrecht AT", "Privatrecht I", "Staatsrecht", "Römisches Recht"],
                "Fall_7.pdf",
                "Fall 7: A schlägt B mit einem Stock. Prüfen Sie die Strafbarkeit des A wegen Körperverletzung (§ 223 StGB). Vorsatz, Rechtswidrigkeit, Schuld.",
            ),
            (
                ["Anatomie I", "Biochemie", "Physiologie", "Medizinische Psychologie"],
                "VL05.pdf",
                "Vorlesung 5: Der Herzzyklus. Systole und Diastole, Druck-Volumen-Diagramm, Herzminutenvolumen, Frank-Starling-Mechanismus.",
            ),
            (
                ["Ancient History", "Medieval History", "Modern European History", "Historiography"],
                "week3_reading.pdf",
                "The Investiture Controversy: Gregory VII, Henry IV and the road to Canossa (1077). Read the Dictatus Papae before the seminar.",
            ),
        ];
        for (modules, name, text) in cases {
            let options: Vec<String> = modules.iter().map(|m| m.to_string()).collect();
            let p = model.choose(&options, &state_for(name, text)).unwrap().unwrap();
            println!("{name}: {p:?}");
            let expected = match name {
                "Fall_7.pdf" => 0,
                "VL05.pdf" => 2,
                _ => 1,
            };
            // Right, or unsure (then the keywords decide) – never sure of the wrong module.
            let sure = confident(&p, manifest.min_probability, manifest.min_lead);
            assert!(sure.is_none_or(|(best, _)| best == expected), "{name}: {p:?}");
        }
    }

    /// Network: a computer with an older model (version 0) moves to the newest one and keeps
    /// nothing of the old one. Uses FILE_SORTER_LAYA_DIR like the test above (the download is
    /// skipped when the newest model's files are already there).
    #[test]
    #[ignore]
    fn an_older_model_is_replaced_by_the_newest() {
        let root = std::env::var_os("FILE_SORTER_LAYA_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| std::env::temp_dir().join("file-sorter-laya-uni"));
        let mut old = Manifest::built_in();
        old.name = "laya-uni-0".into();
        old.version = 0;
        fs::create_dir_all(root.join(&old.name)).unwrap();
        for name in FILE_NAMES {
            fs::write(root.join(&old.name).join(name), "old").unwrap();
        }
        fs::write(root.join(INSTALLED), serde_json::to_string(&old).unwrap()).unwrap();
        assert_eq!(installed(&root).unwrap().version, 0);
        update(&root, installed(&root)).unwrap();
        let now = installed(&root).unwrap();
        assert!(now.version >= 1, "{now:?}");
        assert!(!root.join("laya-uni-0").exists());
        // Nothing newer than what is in use: nothing happens.
        update(&root, Some(now.clone())).unwrap();
        assert_eq!(installed(&root), Some(now));
    }
}
