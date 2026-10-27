//! PDF text through pdfium, loaded from the home's `engine/` folder at
//! runtime. One `[Page N]` marker per page. A document with too little text
//! per page is reported as `NoText` so OCR (P4) can be offered.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use pdfium_render::prelude::*;

use crate::{ExtractCtx, ExtractError, Extracted, Result};

/// pdfium is loaded once per process. Its bindings are not `Sync`, so the
/// handle lives behind a mutex: one PDF at a time goes through pdfium while
/// other extractors run beside it on the pool.
static PDFIUM: OnceLock<std::result::Result<Mutex<Pdfium>, String>> = OnceLock::new();

fn pdfium(engine_dir: &Path) -> Result<&'static Mutex<Pdfium>> {
    let state = PDFIUM.get_or_init(|| {
        let lib = Pdfium::pdfium_platform_library_name_at_path(engine_dir);
        if !lib.is_file() {
            return Err(format!("missing:{}", lib.display()));
        }
        Pdfium::bind_to_library(&lib)
            .map(|b| Mutex::new(Pdfium::new(b)))
            .map_err(|e| e.to_string())
    });
    match state {
        Ok(p) => Ok(p),
        Err(m) if m.starts_with("missing:") => Err(ExtractError::PdfiumMissing(PathBuf::from(
            &m["missing:".len()..],
        ))),
        Err(m) => Err(ExtractError::Pdfium(m.clone())),
    }
}

pub fn extract(path: &Path, ctx: &ExtractCtx) -> Result<Extracted> {
    let pdfium = pdfium(&ctx.engine_dir)?;
    let guard = pdfium.lock().unwrap_or_else(|e| e.into_inner());
    let doc = guard
        .load_pdf_from_file(path, None)
        .map_err(|e| ExtractError::Malformed {
            path: path.to_path_buf(),
            kind: "PDF",
            reason: e.to_string(),
        })?;
    let mut text = String::new();
    let mut chars = 0usize;
    let mut pages = 0u32;
    for (i, page) in doc.pages().iter().enumerate() {
        pages += 1;
        let page_text = page.text().map(|t| t.all()).unwrap_or_default();
        let cleaned = normalise(&page_text);
        chars += cleaned.chars().filter(|c| !c.is_whitespace()).count();
        text.push_str(&format!("[Page {}]\n", i + 1));
        text.push_str(&cleaned);
        text.push('\n');
    }
    let per_page = if pages > 0 {
        chars as f64 / f64::from(pages)
    } else {
        0.0
    };
    if per_page < ctx.min_chars_per_page {
        return Err(ExtractError::NoText {
            path: path.to_path_buf(),
            chars_per_page: per_page,
        });
    }
    Ok(Extracted {
        text,
        units: pages,
        unit_kind: "page",
    })
}

/// Collapse runs of spaces and join hyphenated line breaks.
fn normalise(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut last_space = false;
    for c in s.replace("\r\n", "\n").replace('\r', "\n").chars() {
        if c == ' ' || c == '\t' {
            if !last_space {
                out.push(' ');
            }
            last_space = true;
        } else {
            out.push(c);
            last_space = c == '\n';
        }
    }
    out.replace("-\n", "")
}
