//! Text recognition (OCR) for scanned PDFs and pictures: pages without a text layer are
//! read from their image.
//!
//! On Windows this uses the recognition built into Windows (`Windows.Media.Ocr`, with the
//! languages installed there), on Linux `tesseract` and `pdftoppm` if they are installed.
//! What was recognised is kept in `ocr-cache.json` by the file's content, so a file is never
//! recognised twice, also after it moved.

use std::collections::HashMap;
use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

use sha2::{Digest, Sha256};

pub const CACHE_FILE: &str = "ocr-cache.json";
/// A page with fewer letters than this has no (real) text layer.
pub const EMPTY_PAGE: usize = 25;
/// Cached files, at most (the oldest go first).
const MAX_CACHED: usize = 3000;

static ENABLED: AtomicBool = AtomicBool::new(true);
static CACHE_DIR: OnceLock<PathBuf> = OnceLock::new();
static CACHE: Mutex<Option<Cache>> = Mutex::new(None);

/// Switch recognition on or off (the setting, and saving resources in the background).
pub fn set_enabled(on: bool) {
    ENABLED.store(on, Ordering::Relaxed);
}

pub fn enabled() -> bool {
    ENABLED.load(Ordering::Relaxed) && available() != Engine::Missing
}

/// Where the cache is kept (the state folder).
pub fn set_cache_dir(dir: &Path) {
    let _ = CACHE_DIR.set(dir.to_path_buf());
}

/// What recognises text on this computer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Engine {
    Windows,
    Tesseract,
    Missing,
}

pub fn available() -> Engine {
    static FOUND: OnceLock<Engine> = OnceLock::new();
    *FOUND.get_or_init(|| {
        if cfg!(windows) {
            Engine::Windows
        } else if has_program("tesseract") && has_program("pdftoppm") {
            Engine::Tesseract
        } else {
            Engine::Missing
        }
    })
}

fn has_program(name: &str) -> bool {
    std::process::Command::new(name)
        .arg("-v")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok()
}

pub fn letters(text: &str) -> usize {
    text.chars().filter(|c| c.is_alphabetic()).count()
}

/// Recognised pages of files, by a fingerprint of their content.
#[derive(Default, serde::Serialize, serde::Deserialize)]
struct Cache {
    /// Fingerprint → (page number → text), in the order they were added.
    files: Vec<(String, HashMap<u32, String>)>,
}

impl Cache {
    fn file() -> Option<PathBuf> {
        CACHE_DIR.get().map(|d| d.join(CACHE_FILE))
    }

    fn with<T>(f: impl FnOnce(&mut Cache) -> T) -> T {
        let mut guard = CACHE.lock().unwrap_or_else(|e| e.into_inner());
        let cache = guard.get_or_insert_with(|| {
            Cache::file()
                .and_then(|f| fs::read_to_string(f).ok())
                .and_then(|t| serde_json::from_str(&t).ok())
                .unwrap_or_default()
        });
        f(cache)
    }

    fn save(&self) {
        if let Some(file) = Cache::file()
            && let Err(e) = crate::cloud::save_json(self, &file)
        {
            log::info!("Could not save the recognised text: {e}");
        }
    }
}

/// Drop the cache from memory (it stays on disk).
pub fn forget_cache() {
    *CACHE.lock().unwrap_or_else(|e| e.into_inner()) = None;
}

/// A fingerprint of a file's content: its size, start and end.
fn fingerprint(path: &Path) -> Option<String> {
    let mut file = fs::File::open(path).ok()?;
    let size = file.metadata().ok()?.len();
    let mut hash = Sha256::new();
    hash.update(size.to_le_bytes());
    let mut buffer = vec![0; 256 * 1024];
    let n = file.read(&mut buffer).ok()?;
    hash.update(&buffer[..n]);
    if size > 512 * 1024 {
        file.seek(SeekFrom::End(-64 * 1024)).ok()?;
        let n = file.read(&mut buffer[..64 * 1024]).ok()?;
        hash.update(&buffer[..n]);
    }
    Some(format!("{:x}", hash.finalize()))
}

/// The text of the given pages (1-based) of a PDF, recognised from their image. Pages
/// recognised before come from the cache.
pub fn pdf_pages(path: &Path, pages: &[u32]) -> HashMap<u32, String> {
    let mut out = HashMap::new();
    if pages.is_empty() || !enabled() {
        return out;
    }
    let Some(key) = fingerprint(path) else { return out };
    let known: HashMap<u32, String> =
        Cache::with(|c| c.files.iter().find(|(k, _)| *k == key).map(|(_, p)| p.clone())).unwrap_or_default();
    let missing: Vec<u32> = pages.iter().copied().filter(|p| !known.contains_key(p)).collect();
    let recognised = if missing.is_empty() { HashMap::new() } else { recognise_pdf(path, &missing) };
    for p in pages {
        if let Some(text) = known.get(p).or_else(|| recognised.get(p)) {
            out.insert(*p, text.clone());
        }
    }
    if !recognised.is_empty() || !missing.is_empty() {
        Cache::with(|c| {
            let entry = match c.files.iter().position(|(k, _)| *k == key) {
                Some(i) => &mut c.files[i].1,
                None => {
                    c.files.push((key.clone(), HashMap::new()));
                    &mut c.files.last_mut().expect("just added").1
                }
            };
            // Also pages without any text, so they aren't tried again.
            for p in &missing {
                entry.insert(*p, recognised.get(p).cloned().unwrap_or_default());
            }
            let excess = c.files.len().saturating_sub(MAX_CACHED);
            c.files.drain(..excess);
            c.save();
        });
    }
    out
}

/// The text in a picture (PNG, JPEG), recognised.
pub fn image(path: &Path) -> Option<String> {
    if !enabled() {
        return None;
    }
    let key = fingerprint(path)?;
    if let Some(text) = Cache::with(|c| c.files.iter().find(|(k, _)| *k == key).and_then(|(_, p)| p.get(&1).cloned())) {
        return Some(text);
    }
    let text = recognise_image(path).unwrap_or_default();
    Cache::with(|c| {
        c.files.push((key, HashMap::from([(1, text.clone())])));
        c.save();
    });
    Some(text)
}

pub fn is_image(path: &Path) -> bool {
    let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    ["png", "jpg", "jpeg", "tif", "tiff", "bmp"].contains(&ext.as_str())
}

// ---------- Windows: the recognition built into Windows ----------

#[cfg(windows)]
fn recognise_pdf(path: &Path, pages: &[u32]) -> HashMap<u32, String> {
    match windows_ocr::pdf(path, pages) {
        Ok(found) => found,
        Err(e) => {
            log::info!("Text recognition failed for {}: {e}", path.display());
            HashMap::new()
        }
    }
}

#[cfg(windows)]
fn recognise_image(path: &Path) -> Option<String> {
    windows_ocr::image(path).map_err(|e| log::info!("Text recognition failed for {}: {e}", path.display())).ok()
}

#[cfg(windows)]
mod windows_ocr {
    use std::collections::HashMap;
    use std::path::Path;

    use windows::Data::Pdf::{PdfDocument, PdfPageRenderOptions};
    use windows::Graphics::Imaging::{BitmapAlphaMode, BitmapDecoder, BitmapPixelFormat, SoftwareBitmap};
    use windows::Media::Ocr::OcrEngine;
    use windows::Storage::Streams::{IRandomAccessStream, InMemoryRandomAccessStream};
    use windows::Storage::{FileAccessMode, StorageFile};
    use windows::core::{HSTRING, Result};

    fn engine() -> Result<OcrEngine> {
        OcrEngine::TryCreateFromUserProfileLanguages()
    }

    fn absolute(path: &Path) -> HSTRING {
        HSTRING::from(crate::desktop::plain_path(path).as_os_str())
    }

    fn bitmap(stream: &IRandomAccessStream) -> Result<SoftwareBitmap> {
        let decoder = BitmapDecoder::CreateAsync(stream)?.join()?;
        decoder.GetSoftwareBitmapConvertedAsync(BitmapPixelFormat::Bgra8, BitmapAlphaMode::Premultiplied)?.join()
    }

    fn read(engine: &OcrEngine, bitmap: &SoftwareBitmap) -> Result<String> {
        let result = engine.RecognizeAsync(bitmap)?.join()?;
        // Line by line, so sentences and dates stay apart as on the page.
        let mut text = String::new();
        for line in result.Lines()? {
            text.push_str(&line.Text()?.to_string_lossy());
            text.push('\n');
        }
        Ok(text)
    }

    pub fn pdf(path: &Path, pages: &[u32]) -> Result<HashMap<u32, String>> {
        let engine = engine()?;
        let file = StorageFile::GetFileFromPathAsync(&absolute(path))?.join()?;
        let document = PdfDocument::LoadFromFileAsync(&file)?.join()?;
        let count = document.PageCount()?;
        let largest = OcrEngine::MaxImageDimension()?;
        let mut out = HashMap::new();
        for &number in pages {
            if number == 0 || number > count {
                continue;
            }
            let page = document.GetPage(number - 1)?;
            // About 200 dpi: wide enough for small print, within what the engine takes.
            let size = page.Size()?;
            let width = ((size.Width * 200.0 / 72.0) as u32).min(largest);
            let options = PdfPageRenderOptions::new()?;
            options.SetDestinationWidth(width)?;
            let stream = InMemoryRandomAccessStream::new()?;
            page.RenderWithOptionsToStreamAsync(&stream, &options)?.join()?;
            let image = bitmap(&stream.cast()?)?;
            out.insert(number, read(&engine, &image)?);
        }
        Ok(out)
    }

    pub fn image(path: &Path) -> Result<String> {
        let engine = engine()?;
        let file = StorageFile::GetFileFromPathAsync(&absolute(path))?.join()?;
        let stream = file.OpenAsync(FileAccessMode::Read)?.join()?;
        read(&engine, &bitmap(&stream)?)
    }

    use windows::core::Interface;
}

// ---------- Linux and others: tesseract ----------

#[cfg(not(windows))]
fn languages() -> String {
    static LANGS: OnceLock<String> = OnceLock::new();
    LANGS
        .get_or_init(|| {
            let out = std::process::Command::new("tesseract").arg("--list-langs").output();
            let list = out.map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default();
            let have: Vec<&str> =
                ["deu", "eng", "fra", "ita"].into_iter().filter(|l| list.lines().any(|x| x == *l)).collect();
            if have.is_empty() { "eng".into() } else { have.join("+") }
        })
        .clone()
}

#[cfg(not(windows))]
fn tesseract(image: &Path) -> Option<String> {
    let out = std::process::Command::new("tesseract")
        .arg(image)
        .arg("stdout")
        .args(["-l", &languages()])
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

#[cfg(not(windows))]
fn recognise_pdf(path: &Path, pages: &[u32]) -> HashMap<u32, String> {
    static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let mut out = HashMap::new();
    let dir = std::env::temp_dir().join(format!(
        "file-sorter-ocr-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    if fs::create_dir_all(&dir).is_err() {
        return out;
    }
    for &page in pages {
        let prefix = dir.join(format!("page{page}"));
        let rendered = std::process::Command::new("pdftoppm")
            .args(["-r", "200", "-gray", "-png", "-singlefile", "-f", &page.to_string(), "-l", &page.to_string()])
            .arg(path)
            .arg(&prefix)
            .stderr(std::process::Stdio::null())
            .status();
        let image = prefix.with_extension("png");
        if rendered.is_ok_and(|s| s.success())
            && let Some(text) = tesseract(&image)
        {
            out.insert(page, text);
        }
        let _ = fs::remove_file(&image);
    }
    let _ = fs::remove_dir_all(&dir);
    out
}

#[cfg(not(windows))]
fn recognise_image(path: &Path) -> Option<String> {
    tesseract(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprints_follow_the_content_not_the_name() {
        let tmp = tempfile::tempdir().unwrap();
        let (a, b, c) = (tmp.path().join("a.pdf"), tmp.path().join("b.pdf"), tmp.path().join("c.pdf"));
        fs::write(&a, "same content").unwrap();
        fs::write(&b, "same content").unwrap();
        fs::write(&c, "other content").unwrap();
        assert_eq!(fingerprint(&a), fingerprint(&b));
        assert_ne!(fingerprint(&a), fingerprint(&c));
    }

    /// Needs tesseract and pdftoppm (Linux) or Windows' own recognition: a page that is only
    /// a picture of text is read.
    #[test]
    #[ignore]
    fn recognises_a_scanned_page() {
        let file = PathBuf::from(std::env::var("OCR_SAMPLE").expect("OCR_SAMPLE=<scanned pdf>"));
        let text = pdf_pages(&file, &[1]).remove(&1).unwrap_or_default();
        println!("{text}");
        assert!(letters(&text) > 20, "nothing recognised");
    }
}
