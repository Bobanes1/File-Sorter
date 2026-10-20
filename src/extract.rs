//! Pull readable text out of PDFs, slides and Word documents.
//!
//! Only the first few pages/slides are read: course names and codes are almost
//! always on the title page, and it keeps sorting fast for large lecture files.

use std::fs;
use std::io::{Read, Seek};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;

use quick_xml::Reader;
use quick_xml::events::Event;

const MAX_PAGES: u32 = 5;
const MAX_SLIDES: usize = 5;
const MAX_PARAGRAPHS: usize = 60;
const MAX_CHARS: usize = 20_000;

/// Text from the start of the file, or "" if it can't be read.
pub fn extract_text(path: &Path) -> String {
    let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    // PDF parsers can panic on unusual files; a bad file must never take the sorter down.
    let result = catch_unwind(AssertUnwindSafe(|| match ext.as_str() {
        "pdf" => pdf_text(&fs::read(path).ok()?),
        "pptx" => pptx_text(fs::File::open(path).ok()?),
        "docx" => docx_text(fs::File::open(path).ok()?),
        "txt" | "md" => fs::read(path).ok().map(|b| String::from_utf8_lossy(&b).into_owned()),
        "zip" => Some(zip_text(path)),
        _ => None,
    }));
    let text = finish(result, path);
    // A scan (pages that are only pictures) or a photo: recognise the text.
    if ext == "pdf" && crate::ocr::letters(&text) < crate::ocr::EMPTY_PAGE {
        let pages: Vec<u32> = (1..=OCR_SORT_PAGES).collect();
        let found = crate::ocr::pdf_pages(path, &pages);
        if !found.is_empty() {
            return pages.iter().filter_map(|p| found.get(p).cloned()).collect::<Vec<_>>().join("\n");
        }
    }
    if crate::ocr::is_image(path) {
        return crate::ocr::image(path).unwrap_or_default();
    }
    text
}

/// Pages of a scan that are recognised for sorting, and for the search and deadlines.
const OCR_SORT_PAGES: u32 = 3;
const OCR_INDEX_PAGES: usize = 12;

/// Text from the start of a document held in memory (`name` gives its type).
fn extract_bytes(name: &str, data: &[u8]) -> String {
    let ext = Path::new(name).extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    let result = catch_unwind(AssertUnwindSafe(|| match ext.as_str() {
        "pdf" => pdf_text(data),
        "pptx" => pptx_text(std::io::Cursor::new(data)),
        "docx" => docx_text(std::io::Cursor::new(data)),
        "txt" | "md" => Some(String::from_utf8_lossy(data).into_owned()),
        _ => None,
    }));
    finish(result, Path::new(name))
}

/// Documents inside a ZIP file that are read for its text (the first few, not too big).
const ZIP_DOCUMENTS: usize = 3;
const ZIP_DOCUMENT_BYTES: u64 = 20 * 1024 * 1024;
const ZIP_NAMES: usize = 200;

/// The names of the files in a ZIP file.
pub fn zip_names(path: &Path) -> String {
    let Ok(file) = fs::File::open(path) else { return String::new() };
    let Ok(zip) = zip::ZipArchive::new(file) else { return String::new() };
    zip.file_names().take(ZIP_NAMES).filter_map(|n| n.ok()).collect::<Vec<_>>().join("\n")
}

/// What a ZIP file says about itself: the names of the files in it, then the text of the
/// first few documents.
pub fn zip_text(path: &Path) -> String {
    let Ok(file) = fs::File::open(path) else { return String::new() };
    let Ok(mut zip) = zip::ZipArchive::new(file) else { return String::new() };
    let names: Vec<String> = zip.file_names().take(ZIP_NAMES).filter_map(|n| n.ok()).map(|n| n.into_owned()).collect();
    let mut text = names.join("\n");
    text.push('\n');
    let documents = names.iter().filter(|n| {
        let lower = n.to_lowercase();
        !lower.starts_with("__macosx/") && [".pdf", ".pptx", ".docx", ".txt", ".md"].iter().any(|e| lower.ends_with(e))
    });
    for name in documents.take(ZIP_DOCUMENTS) {
        let Ok(mut entry) = zip.by_name(name) else { continue };
        if entry.encrypted() || entry.size() > ZIP_DOCUMENT_BYTES {
            continue;
        }
        let mut data = Vec::new();
        if entry.by_ref().take(ZIP_DOCUMENT_BYTES).read_to_end(&mut data).is_ok() {
            text.push_str(&extract_bytes(name, &data));
            text.push('\n');
        }
        if text.len() > MAX_CHARS * 4 {
            break;
        }
    }
    text
}

/// For the search: at most this many pages or slides, this much text per page and per file.
const INDEX_PAGES: usize = 400;
const INDEX_PAGE_CHARS: usize = 20_000;
const INDEX_FILE_CHARS: usize = 400_000;

/// The whole text of a document for the search, one entry per page (PDF) or slide (PowerPoint);
/// Word and text files are one page. Empty if it can't be read.
pub fn pages(path: &Path) -> Vec<String> {
    pages_up_to(path, INDEX_PAGES)
}

/// The text of the first `count` pages or slides, for finding deadlines further in than the
/// title page (an exam date on page 6 of the course information).
pub fn first_pages(path: &Path, count: usize) -> String {
    match pages_up_to(path, count) {
        pages if pages.is_empty() => extract_text(path),
        pages => pages.join("\n"),
    }
}

fn pages_up_to(path: &Path, count: usize) -> Vec<String> {
    let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    let result = catch_unwind(AssertUnwindSafe(|| match ext.as_str() {
        "pdf" => pdf_pages(&fs::read(path).ok()?, count),
        "pptx" => pptx_slides(fs::File::open(path).ok()?, count),
        "docx" => {
            let mut zip = zip::ZipArchive::new(fs::File::open(path).ok()?).ok()?;
            let xml = read_entry(&mut zip, "word/document.xml")?;
            Some(vec![paragraphs(&xml, usize::MAX).join("\n")])
        }
        "txt" | "md" => fs::read(path).ok().map(|b| vec![String::from_utf8_lossy(&b).into_owned()]),
        _ => None,
    }));
    let mut read = result.ok().flatten().unwrap_or_default();
    // Pages without a text layer (a scan): recognise the first ones; a photo is one page.
    if ext == "pdf" {
        let empty: Vec<u32> = read
            .iter()
            .enumerate()
            .take(OCR_INDEX_PAGES)
            .filter(|(_, p)| crate::ocr::letters(p) < crate::ocr::EMPTY_PAGE)
            .map(|(i, _)| i as u32 + 1)
            .collect();
        for (number, text) in crate::ocr::pdf_pages(path, &empty) {
            if let Some(page) = read.get_mut(number as usize - 1) {
                *page = text;
            }
        }
    } else if crate::ocr::is_image(path) {
        read = crate::ocr::image(path).into_iter().collect();
    }
    let mut total = 0;
    let mut pages = Vec::new();
    for page in read.into_iter().take(count) {
        let page: String = page.chars().take(INDEX_PAGE_CHARS).collect();
        total += page.len();
        if total > INDEX_FILE_CHARS {
            break;
        }
        pages.push(page);
    }
    pages
}

fn pdf_pages(data: &[u8], count: usize) -> Option<Vec<String>> {
    let mut doc = load_pdf(data)?;
    if doc.is_encrypted() && doc.decrypt("").is_err() {
        return None;
    }
    let count = doc.get_pages().len().min(count) as u32;
    let mut pages = Vec::new();
    for page in 1..=count {
        let mut text = String::new();
        let mut output = pdf_extract::PlainTextOutput::new(&mut text);
        // A page that fails to parse shouldn't hide the text of the others.
        let _ = pdf_extract::output_doc_page(&doc, &mut output, page);
        pages.push(text);
    }
    Some(pages)
}

fn pptx_slides<R: Read + Seek>(reader: R, count: usize) -> Option<Vec<String>> {
    let mut zip = zip::ZipArchive::new(reader).ok()?;
    let mut slides: Vec<(u32, String)> = zip
        .file_names()
        .filter_map(|name| {
            let name = name.ok()?;
            let number = name.strip_prefix("ppt/slides/slide")?.strip_suffix(".xml")?.parse().ok()?;
            Some((number, name.to_string()))
        })
        .collect();
    slides.sort();
    let mut pages = Vec::new();
    for (_, name) in slides.into_iter().take(count) {
        let xml = read_entry(&mut zip, &name).unwrap_or_default();
        pages.push(paragraphs(&xml, usize::MAX).join("\n"));
    }
    Some(pages)
}

fn finish(result: std::thread::Result<Option<String>>, path: &Path) -> String {
    match result {
        Ok(Some(text)) => text.chars().take(MAX_CHARS).collect(),
        Ok(None) => String::new(),
        Err(_) => {
            log::debug!("text extraction panicked for {}", path.display());
            String::new()
        }
    }
}

/// Load a PDF, tolerating extra bytes after its last `%%EOF` (lopdf only looks
/// for the trailer near the very end of the file).
pub(crate) fn load_pdf(data: &[u8]) -> Option<lopdf::Document> {
    if let Ok(doc) = lopdf::Document::load_mem(data) {
        return Some(doc);
    }
    let end = data.windows(5).rposition(|w| w == b"%%EOF")? + 5;
    lopdf::Document::load_mem(&data[..end]).ok()
}

fn pdf_text(data: &[u8]) -> Option<String> {
    let mut doc = load_pdf(data)?;
    if doc.is_encrypted() && doc.decrypt("").is_err() {
        return None;
    }
    let mut text = String::new();
    if let Ok(info) = doc.trailer.get_deref(b"Info", &doc).and_then(|o| o.as_dict()) {
        for key in [&b"Title"[..], b"Subject"] {
            if let Ok(value) = info.get_deref(key, &doc).and_then(lopdf::decode_text_string) {
                text.push_str(&value);
                text.push('\n');
            }
        }
    }
    let pages = doc.get_pages().len() as u32;
    // Saving resources in the background: the first page only.
    let max = if crate::lean::active() { 1 } else { MAX_PAGES };
    for page in 1..=pages.min(max) {
        let mut output = pdf_extract::PlainTextOutput::new(&mut text);
        // A page that fails to parse shouldn't hide the text of the others.
        let _ = pdf_extract::output_doc_page(&doc, &mut output, page);
        text.push('\n');
    }
    Some(text)
}

fn pptx_text<R: Read + Seek>(reader: R) -> Option<String> {
    let mut zip = zip::ZipArchive::new(reader).ok()?;
    let mut slides: Vec<(u32, String)> = zip
        .file_names()
        .filter_map(|name| {
            let name = name.ok()?;
            let number = name.strip_prefix("ppt/slides/slide")?.strip_suffix(".xml")?.parse().ok()?;
            Some((number, name.to_string()))
        })
        .collect();
    slides.sort();
    let mut text = core_properties(&mut zip);
    for (_, name) in slides.into_iter().take(MAX_SLIDES) {
        if let Some(xml) = read_entry(&mut zip, &name) {
            text.push_str(&paragraphs(&xml, usize::MAX).join("\n"));
            text.push('\n');
        }
    }
    Some(text)
}

fn docx_text<R: Read + Seek>(reader: R) -> Option<String> {
    let mut zip = zip::ZipArchive::new(reader).ok()?;
    let mut text = core_properties(&mut zip);
    let xml = read_entry(&mut zip, "word/document.xml")?;
    text.push_str(&paragraphs(&xml, MAX_PARAGRAPHS).join("\n"));
    Some(text)
}

fn read_entry<R: Read + Seek>(zip: &mut zip::ZipArchive<R>, name: &str) -> Option<String> {
    let mut entry = zip.by_name(name).ok()?;
    let mut xml = String::new();
    entry.read_to_string(&mut xml).ok()?;
    Some(xml)
}

/// Title and subject from docProps/core.xml.
fn core_properties<R: Read + Seek>(zip: &mut zip::ZipArchive<R>) -> String {
    let Some(xml) = read_entry(zip, "docProps/core.xml") else { return String::new() };
    let mut out = String::new();
    for wanted in ["title", "subject"] {
        let value = element_text(&xml, wanted);
        if !value.trim().is_empty() {
            out.push_str(value.trim());
            out.push('\n');
        }
    }
    out
}

fn element_text(xml: &str, local_name: &str) -> String {
    let mut reader = Reader::from_str(xml);
    let mut inside = false;
    let mut out = String::new();
    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) if e.local_name().as_ref() == local_name => inside = true,
            Ok(Event::End(e)) if e.local_name().as_ref() == local_name => break,
            Ok(Event::Text(t)) if inside => out.push_str(&t.xml10_content()),
            Ok(Event::GeneralRef(r)) if inside => out.push_str(&resolve_ref(&r)),
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    out
}

/// Text of `<a:p>`/`<w:p>` paragraphs, built from their `<a:t>`/`<w:t>` runs.
fn paragraphs(xml: &str, max: usize) -> Vec<String> {
    let mut reader = Reader::from_str(xml);
    let mut out = Vec::new();
    let mut current = String::new();
    let mut in_text = false;
    while out.len() < max {
        match reader.read_event() {
            Ok(Event::Start(e)) if e.local_name().as_ref() == "t" => in_text = true,
            Ok(Event::End(e)) => match e.local_name().as_ref() {
                "t" => in_text = false,
                "p" => {
                    if !current.trim().is_empty() {
                        out.push(std::mem::take(&mut current));
                    }
                    current.clear();
                }
                _ => {}
            },
            Ok(Event::Text(t)) if in_text => current.push_str(&t.xml10_content()),
            Ok(Event::GeneralRef(r)) if in_text => current.push_str(&resolve_ref(&r)),
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    if !current.trim().is_empty() && out.len() < max {
        out.push(current);
    }
    out
}

fn resolve_ref(r: &quick_xml::events::BytesRef) -> String {
    if let Ok(Some(c)) = r.resolve_char_ref() {
        return c.to_string();
    }
    match r.as_ref() {
        "amp" => "&",
        "lt" => "<",
        "gt" => ">",
        "quot" => "\"",
        "apos" => "'",
        _ => "",
    }
    .to_string()
}

#[cfg(test)]
mod tests {

    /// A scan (a page that is only a picture): its text is recognised, so it is sorted and
    /// its deadline found. Runs where tesseract is installed (and on Windows in CI).
    #[test]
    fn scanned_pages_are_recognised() {
        if crate::ocr::available() != crate::ocr::Engine::Tesseract {
            return;
        }
        crate::ocr::enable_in_this_test();
        let scan = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/scan_serie5.pdf");
        let text = extract_text(&scan);
        assert!(text.contains("Serie 5") && text.contains("Abgabe"), "{text}");
        let found =
            crate::deadlines::find(&first_pages(&scan, 3), chrono::NaiveDate::from_ymd_opt(2026, 10, 20).unwrap());
        assert_eq!(found[0].date, chrono::NaiveDate::from_ymd_opt(2026, 11, 3).unwrap());
    }

    use super::*;
    use std::path::PathBuf;

    fn fixture(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join("fixtures").join(name)
    }

    #[test]
    fn simple_pdf() {
        assert!(extract_text(&fixture("linear_algebra.pdf")).contains("Linear Algebra"));
    }

    #[test]
    fn real_world_pdf_with_subset_fonts_and_title() {
        let text = extract_text(&fixture("analysis_chromium.pdf"));
        assert!(text.contains("Einführung in die Analysis I – Vorlesung 4"), "metadata title missing: {text}");
        assert!(text.contains("Folgen und Grenzwerte"), "page text missing: {text}");
    }

    #[test]
    fn pptx() {
        let text = extract_text(&fixture("programming.pptx"));
        assert!(text.contains("Prog1 Lecture"), "{text}");
        assert!(text.contains("Programming 1 - Week 3"), "{text}");
        assert!(text.contains("Loops and functions in Python"), "{text}");
    }

    #[test]
    fn docx() {
        let text = extract_text(&fixture("statistics.docx"));
        assert!(text.contains("Stats Sheet"), "{text}");
        assert!(text.contains("Statistics exercise sheet"), "{text}");
        assert!(text.contains("Probability and distributions"), "{text}");
    }

    #[test]
    fn pdf_with_junk_after_end_marker() {
        // Some PDFs carry extra bytes after %%EOF; the trailer must still be found.
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("padded.pdf");
        let mut data = fs::read(fixture("linear_algebra.pdf")).unwrap();
        data.extend_from_slice(b"\n%");
        data.extend(std::iter::repeat_n(b'x', 400_000));
        fs::write(&path, data).unwrap();
        assert!(extract_text(&path).contains("Linear Algebra"));
    }

    #[test]
    fn broken_and_unknown_files_give_empty_text() {
        let tmp = tempfile::tempdir().unwrap();
        let broken = tmp.path().join("broken.pdf");
        fs::write(&broken, b"not really a pdf").unwrap();
        assert_eq!(extract_text(&broken), "");
        let broken_pptx = tmp.path().join("broken.pptx");
        fs::write(&broken_pptx, b"PK nope").unwrap();
        assert_eq!(extract_text(&broken_pptx), "");
        let old = tmp.path().join("old.ppt");
        fs::write(&old, b"\x00\x01").unwrap();
        assert_eq!(extract_text(&old), "");
    }

    #[test]
    fn xml_entities_are_decoded() {
        let xml = r#"<w:body><w:p><w:r><w:t>R&amp;D &#x2013; Q&lt;1</w:t></w:r></w:p></w:body>"#;
        assert_eq!(paragraphs(xml, 10), vec!["R&D – Q<1"]);
    }
}
