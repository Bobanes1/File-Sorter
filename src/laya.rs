//! Optional: let the Laya decision model (convaiinnovations/laya, multilingual checkpoint)
//! choose the module from a file's name and text, instead of counting keyword hits. It runs
//! on this computer; only the model (about 680 MB) is downloaded once, into the state folder.
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
use serde::Deserialize;

use crate::config::{Config, Course};

/// The model's files are taken from this commit, so they can't change under us.
const REVISION: &str = "7b928d828b7b0e022f929d9bd2e44165aa270148";
/// (path in the model repository, local name, size, SHA-256)
const FILES: [(&str, &str, u64, &str); 4] = [
    (
        "multilingual/rl_agent_config.json",
        "rl_agent_config.json",
        472,
        "25061739243b617ad88d1219ba6f8a9c86c5881ca28df024fa2d9b3b2fcc30c6",
    ),
    (
        "multilingual/encoder/config.json",
        "encoder-config.json",
        1938,
        "83f6916d13ef0f556ac461f28308dc2bffa7ebeadee8ec9e2db5812020ea5bb4",
    ),
    (
        "multilingual/tokenizer/tokenizer.json",
        "tokenizer.json",
        34_363_188,
        "609d8f4c067cd3950f88594c5a802616cea245823836ef5848ee4fc40aab5b6f",
    ),
    (
        "multilingual/model.safetensors",
        "model.safetensors",
        643_835_514,
        "9d628fd971b700382ac6f65920a86f149777b2e748e0c955fb3b19695aa8f204",
    ),
];
const INSTRUCTIONS: &str = "Which university module does this file belong to?";
/// Enough text for the model's 1024 tokens; tokenizing a whole book would only cost time.
const MAX_TEXT_CHARS: usize = 6000;
// shortcut: untuned thresholds (the model card says it ships over-confident), tune on real files.
/// The answer must be at least this likely,
const MIN_PROBABILITY: f32 = 0.6;
/// and this many times as likely as the second.
const MIN_LEAD: f32 = 2.0;
/// Option texts are cut to this many tokens (as in training).
const MAX_OPTION_TOKENS: usize = 48;

pub fn total_size() -> u64 {
    FILES.iter().map(|f| f.2).sum()
}

/// Where the model is kept.
pub fn dir(config: &Config) -> Option<PathBuf> {
    config.state_file("laya")
}

fn ready(dir: &Path) -> bool {
    FILES.iter().all(|(_, name, _, _)| dir.join(name).is_file())
}

pub enum Status {
    /// Not downloaded (yet).
    Missing,
    /// Downloading, with the bytes done so far (of `total_size()`).
    Downloading(u64),
    Ready,
    Failed(String),
}

static DOWNLOADING: AtomicBool = AtomicBool::new(false);
/// Why the last download or loading failed (until switched off and on again).
static ERROR: Mutex<Option<String>> = Mutex::new(None);
static CACHE: Mutex<Option<Arc<Laya>>> = Mutex::new(None);

fn error() -> Option<String> {
    ERROR.lock().ok().and_then(|e| e.clone())
}

fn set_error(message: Option<String>) {
    if let Ok(mut e) = ERROR.lock() {
        *e = message;
    }
}

pub fn status(dir: Option<&Path>) -> Status {
    let Some(dir) = dir else { return Status::Missing };
    if DOWNLOADING.load(Ordering::Relaxed) {
        return Status::Downloading(progress(dir));
    }
    if let Some(e) = error() {
        return Status::Failed(e);
    }
    if ready(dir) { Status::Ready } else { Status::Missing }
}

fn progress(dir: &Path) -> u64 {
    let size = |p: PathBuf| fs::metadata(p).map(|m| m.len()).unwrap_or(0);
    FILES
        .iter()
        .map(|(_, name, total, _)| {
            let file = dir.join(name);
            if file.is_file() { *total } else { size(partial(&file)) }
        })
        .sum()
}

fn partial(file: &Path) -> PathBuf {
    file.with_extension("partial")
}

/// Fetch the model in the background (once at a time).
pub fn start_download(dir: PathBuf) {
    if DOWNLOADING.swap(true, Ordering::Relaxed) {
        return;
    }
    set_error(None);
    std::thread::spawn(move || {
        if let Err(e) = download(&dir) {
            log::info!("Could not download the Laya model: {e}");
            set_error(Some(e));
        }
        DOWNLOADING.store(false, Ordering::Relaxed);
    });
}

/// Download the files that are missing, each checked against its SHA-256. An interrupted
/// download goes on where it stopped.
fn download(dir: &Path) -> Result<(), String> {
    fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    for (remote, name, _, sha) in FILES {
        let file = dir.join(name);
        if file.is_file() {
            continue;
        }
        let part = partial(&file);
        let url = format!("https://huggingface.co/convaiinnovations/laya/resolve/{REVISION}/{remote}");
        let fetched = crate::update::curl()
            .args(["--max-time", "7200", "--continue-at", "-", "--output"])
            .arg(&part)
            .arg(&url)
            .output()
            .map_err(|e| format!("curl could not be started: {e}"))?;
        if !fetched.status.success() {
            return Err(String::from_utf8_lossy(&fetched.stderr).trim().to_string());
        }
        let got = crate::update::sha256_of(&part).map_err(|e| e.to_string())?;
        if got != sha {
            let _ = fs::remove_file(&part);
            return Err(format!("the download of {name} is not the published one (SHA-256 {got})"));
        }
        fs::rename(&part, &file).map_err(|e| e.to_string())?;
    }
    Ok(())
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
    if let Some(model) = cache.as_ref() {
        return Some(model.clone());
    }
    match Laya::load(dir) {
        Ok(model) => {
            let model = Arc::new(model);
            *cache = Some(model.clone());
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
        && !DOWNLOADING.load(Ordering::Relaxed)
        && dir(config).is_some_and(|d| ready(&d))
}

/// The module for `path` (with the text read from it, if any) and how sure the model is (%),
/// or None when the keywords should decide.
pub fn guess(config: &Config, path: &Path, text: Option<&str>) -> Option<(usize, u8)> {
    if !usable(config) {
        return None;
    }
    let model = load(&dir(config)?)?;
    let options: Vec<String> = config.courses.iter().map(option_text).collect();
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let text: String = text.unwrap_or("").chars().take(MAX_TEXT_CHARS).collect();
    let state = format!("File name: {name}\n\n{text}");
    match model.choose(&options, &state) {
        Ok(Some(p)) => confident(&p),
        Ok(None) => None,
        Err(e) => {
            log::info!("The Laya model failed on {}: {e}", path.display());
            None
        }
    }
}

/// "Programmieren: Java, OOP" – the module and its keywords, as one answer option.
fn option_text(course: &Course) -> String {
    let label = course.label();
    if course.keywords.is_empty() { label } else { format!("{label}: {}", course.keywords.join(", ")) }
}

/// The best option if the model is clearly sure of it.
fn confident(p: &[f32]) -> Option<(usize, u8)> {
    let mut order: Vec<usize> = (0..p.len()).collect();
    order.sort_by(|a, b| p[*b].total_cmp(&p[*a]));
    let (best, second) = (p[*order.first()?], order.get(1).map_or(0.0, |i| p[*i]));
    (best >= MIN_PROBABILITY && best >= MIN_LEAD * second).then(|| (order[0], (best * 100.0).round() as u8))
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
        let tokenizer = tokenizers::Tokenizer::from_file(dir.join("tokenizer.json")).map_err(|e| e.to_string())?;
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
        assert_eq!(confident(&[0.1, 0.8, 0.1]), Some((1, 80)));
        assert_eq!(confident(&[0.5, 0.3, 0.2]), None);
        assert_eq!(confident(&[0.62, 0.38]), None);
        assert_eq!(confident(&[]), None);
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

    /// Downloads about 680 MB (or uses FILE_SORTER_LAYA_DIR) and asks the real model.
    #[test]
    #[ignore]
    fn downloads_and_picks_an_obvious_module() {
        let dir = std::env::var_os("FILE_SORTER_LAYA_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| std::env::temp_dir().join("file-sorter-laya"));
        download(&dir).unwrap();
        let model = Laya::load(&dir).unwrap();
        let options =
            vec!["Lineare Algebra".to_string(), "Programmieren: Java".to_string(), "Wirtschaftsrecht".to_string()];
        let state = "File name: Lineare_Algebra_Uebungsblatt_3.pdf\n\nBestimmen Sie die Eigenwerte der Matrix A.";
        let p = model.choose(&options, state).unwrap().unwrap();
        println!("{p:?}");
        assert_eq!(p.len(), 3);
        assert!(p[0] > p[1] && p[0] > p[2], "{p:?}");
    }
}
