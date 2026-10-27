//! Word and PowerPoint through the zip container and the XML inside it.
//! Paragraphs and table cells for docx; `[Slide N]` per slide for pptx.

use std::io::Read;
use std::path::Path;

use quick_xml::Reader;
use quick_xml::events::Event;

use crate::{ExtractError, Extracted, Result, io_err};

fn open_zip(path: &Path, kind: &'static str) -> Result<zip::ZipArchive<std::fs::File>> {
    let file = std::fs::File::open(path).map_err(|e| io_err(path, e))?;
    zip::ZipArchive::new(file).map_err(|e| ExtractError::Malformed {
        path: path.to_path_buf(),
        kind,
        reason: e.to_string(),
    })
}

fn read_entry(zip: &mut zip::ZipArchive<std::fs::File>, name: &str) -> Option<String> {
    let mut entry = zip.by_name(name).ok()?;
    let mut s = String::new();
    entry.read_to_string(&mut s).ok()?;
    Some(s)
}

/// Text from WordprocessingML or PresentationML: `w:t`/`a:t` runs joined,
/// paragraphs (`w:p`/`a:p`) on their own lines, table cells tab-separated,
/// `w:tab` and `w:br` honoured.
pub fn xml_to_text(xml: &str) -> String {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(false);
    let mut out = String::new();
    let mut buf = Vec::new();
    let mut in_text = false;
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let name = e.local_name();
                match name.as_ref() {
                    "t" => in_text = true,
                    "tab" => out.push('\t'),
                    "br" => out.push('\n'),
                    _ => {}
                }
            }
            Ok(Event::Empty(e)) => match e.local_name().as_ref() {
                "tab" => out.push('\t'),
                "br" | "cr" => out.push('\n'),
                _ => {}
            },
            Ok(Event::End(e)) => match e.local_name().as_ref() {
                "t" => in_text = false,
                "p" => out.push('\n'),
                "tc" => out.push('\t'),
                "tr" => out.push('\n'),
                _ => {}
            },
            Ok(Event::Text(t)) => {
                if in_text {
                    out.push_str(&t.xml10_content());
                }
            }
            Ok(Event::GeneralRef(r)) => {
                if in_text {
                    push_reference(&mut out, &r.xml10_content());
                }
            }
            Ok(Event::Eof) => break,
            Err(e) => {
                tracing::warn!("xml: {e}");
                break;
            }
            _ => {}
        }
        buf.clear();
    }
    out
}

/// `&amp;`, `&#233;`, `&#xE9;` and the other predefined entities.
fn push_reference(out: &mut String, name: &str) {
    let ch = match name {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        n if n.starts_with("#x") || n.starts_with("#X") => u32::from_str_radix(&n[2..], 16)
            .ok()
            .and_then(char::from_u32),
        n if n.starts_with('#') => n[1..].parse::<u32>().ok().and_then(char::from_u32),
        _ => None,
    };
    match ch {
        Some(c) => out.push(c),
        None => {
            out.push('&');
            out.push_str(name);
            out.push(';');
        }
    }
}

pub fn extract_docx(path: &Path) -> Result<Extracted> {
    let mut zip = open_zip(path, "docx")?;
    let body =
        read_entry(&mut zip, "word/document.xml").ok_or_else(|| ExtractError::Malformed {
            path: path.to_path_buf(),
            kind: "docx",
            reason: "no word/document.xml".into(),
        })?;
    let mut text = xml_to_text(&body);
    for part in ["word/footnotes.xml", "word/endnotes.xml"] {
        if let Some(x) = read_entry(&mut zip, part) {
            let t = xml_to_text(&x);
            if !t.trim().is_empty() {
                text.push('\n');
                text.push_str(&t);
            }
        }
    }
    Ok(Extracted {
        text,
        units: 1,
        unit_kind: "file",
    })
}

pub fn extract_pptx(path: &Path) -> Result<Extracted> {
    let mut zip = open_zip(path, "pptx")?;
    let mut slides: Vec<(u32, String)> = Vec::new();
    for i in 0..zip.len() {
        let name = zip
            .by_index(i)
            .ok()
            .and_then(|f| f.name().ok().map(|n| n.to_string()))
            .unwrap_or_default();
        if let Some(rest) = name.strip_prefix("ppt/slides/slide")
            && let Some(num) = rest.strip_suffix(".xml")
            && let Ok(n) = num.parse::<u32>()
        {
            slides.push((n, name));
        }
    }
    slides.sort();
    if slides.is_empty() {
        return Err(ExtractError::Malformed {
            path: path.to_path_buf(),
            kind: "pptx",
            reason: "no slides".into(),
        });
    }
    let mut text = String::new();
    for (n, name) in &slides {
        let xml = read_entry(&mut zip, name).unwrap_or_default();
        text.push_str(&format!("[Slide {n}]\n"));
        text.push_str(&xml_to_text(&xml));
        let notes = format!("ppt/notesSlides/notesSlide{n}.xml");
        if let Some(x) = read_entry(&mut zip, &notes) {
            let t = xml_to_text(&x);
            if !t.trim().is_empty() {
                text.push_str("Notes: ");
                text.push_str(&t);
            }
        }
        text.push('\n');
    }
    Ok(Extracted {
        text,
        units: slides.len() as u32,
        unit_kind: "slide",
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_zip(path: &Path, entries: &[(&str, &str)]) {
        let f = std::fs::File::create(path).unwrap();
        let mut z = zip::ZipWriter::new(f);
        let opts = zip::write::SimpleFileOptions::default();
        for (name, body) in entries {
            z.start_file(*name, opts).unwrap();
            z.write_all(body.as_bytes()).unwrap();
        }
        z.finish().unwrap();
    }

    #[test]
    fn docx_paragraphs_and_tables() {
        let dir = std::env::temp_dir().join(format!("georag-docx-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("r.docx");
        let xml = r#"<?xml version="1.0"?><w:document xmlns:w="x"><w:body>
<w:p><w:r><w:t>Pile </w:t></w:r><w:r><w:t>capacity</w:t></w:r></w:p>
<w:tbl><w:tr><w:tc><w:p><w:r><w:t>BH-1</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r><w:t>80 &amp; 90</w:t></w:r></w:p></w:tc></w:tr></w:tbl>
</w:body></w:document>"#;
        write_zip(&p, &[("word/document.xml", xml)]);
        let e = extract_docx(&p).unwrap();
        assert!(e.text.starts_with("Pile capacity\n"), "{:?}", e.text);
        assert!(e.text.contains("BH-1\n\t80 & 90\n\t\n"), "{:?}", e.text);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn pptx_slides_in_order_with_markers() {
        let dir = std::env::temp_dir().join(format!("georag-pptx-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("d.pptx");
        let s = |t: &str| {
            format!(
                r#"<p:sld xmlns:a="x" xmlns:p="y"><p:cSld><a:p><a:r><a:t>{t}</a:t></a:r></a:p></p:cSld></p:sld>"#
            )
        };
        write_zip(
            &p,
            &[
                ("ppt/slides/slide10.xml", &s("Tenth")),
                ("ppt/slides/slide2.xml", &s("Second")),
                ("ppt/slides/slide1.xml", &s("First")),
            ],
        );
        let e = extract_pptx(&p).unwrap();
        assert_eq!(e.units, 3);
        let first = e.text.find("[Slide 1]").unwrap();
        let second = e.text.find("[Slide 2]").unwrap();
        let tenth = e.text.find("[Slide 10]").unwrap();
        assert!(first < second && second < tenth);
        assert!(e.text.contains("[Slide 2]\nSecond\n"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
