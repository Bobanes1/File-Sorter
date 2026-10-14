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
        "pdf" => pdf_text(path),
        "pptx" => pptx_text(path),
        "docx" => docx_text(path),
        "txt" | "md" => fs::read(path).ok().map(|b| String::from_utf8_lossy(&b).into_owned()),
        _ => None,
    }));
    match result {
        Ok(Some(text)) => text.chars().take(MAX_CHARS).collect(),
        Ok(None) => String::new(),
        Err(_) => {
            log::debug!("text extraction panicked for {}", path.display());
            String::new()
        }
    }
}

fn pdf_text(path: &Path) -> Option<String> {
    let mut doc = lopdf::Document::load(path).ok()?;
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
    for page in 1..=pages.min(MAX_PAGES) {
        let mut output = pdf_extract::PlainTextOutput::new(&mut text);
        // A page that fails to parse shouldn't hide the text of the others.
        let _ = pdf_extract::output_doc_page(&doc, &mut output, page);
        text.push('\n');
    }
    Some(text)
}

fn pptx_text(path: &Path) -> Option<String> {
    let mut zip = zip::ZipArchive::new(fs::File::open(path).ok()?).ok()?;
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

fn docx_text(path: &Path) -> Option<String> {
    let mut zip = zip::ZipArchive::new(fs::File::open(path).ok()?).ok()?;
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
