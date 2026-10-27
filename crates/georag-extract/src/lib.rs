//! Document extractors (handoff 6.3): text with page, sheet or slide
//! markers. `unsafe` is permitted only in the pdfium module and is not needed
//! yet: pdfium-render wraps the FFI.
//!
//! Each extractor is a pure function of the file; the pipeline in core runs
//! them on a CPU pool.

#![deny(unsafe_code)]

pub mod office;
pub mod pdf;
pub mod sheets;
pub mod text;

use std::path::Path;

/// Errors from extraction.
#[derive(Debug, thiserror::Error)]
pub enum ExtractError {
    #[error("no extractor handles files with extension {0:?}")]
    Unsupported(String),
    #[error("could not read {path} ({source})")]
    Io {
        path: std::path::PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("{path} is not a valid {kind} file ({reason})")]
    Malformed {
        path: std::path::PathBuf,
        kind: &'static str,
        reason: String,
    },
    #[error(
        "pdfium is missing: expected the library in {0}. Reinstall GeoRAG or run `cargo xtask fetch-pdfium --out engine`"
    )]
    PdfiumMissing(std::path::PathBuf),
    #[error("pdfium error ({0})")]
    Pdfium(String),
    #[error("{path} has no text layer ({chars_per_page:.0} characters per page): OCR is needed")]
    NoText {
        path: std::path::PathBuf,
        chars_per_page: f64,
    },
}

pub type Result<T> = std::result::Result<T, ExtractError>;

/// What the extractor found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Extracted {
    /// Full text with `[Page N]`, `[Sheet: name]` or `[Slide N]` markers on
    /// their own lines.
    pub text: String,
    /// Pages, sheets or slides seen.
    pub units: u32,
    /// `page`, `sheet` or `slide`, for locators.
    pub unit_kind: &'static str,
}

/// Settings extractors need from the app.
#[derive(Debug, Clone)]
pub struct ExtractCtx {
    /// Folder holding `libpdfium` (the home's `engine/`).
    pub engine_dir: std::path::PathBuf,
    /// Below this many characters per page a PDF counts as scanned.
    pub min_chars_per_page: f64,
}

/// Extensions handled, lowercase, without the dot.
pub const SUPPORTED: &[&str] = &[
    "pdf", "docx", "pptx", "xlsx", "xlsm", "xls", "ods", "csv", "tsv", "txt", "md", "markdown",
    "html", "htm", "rtf", "png", "jpg", "jpeg", "webp", "tiff", "tif", "gif", "bmp", "dwg", "dxf",
];

/// Extensions whose content is not read yet: images caption in P4, CAD is
/// indexed by name only.
pub const NAME_ONLY: &[&str] = &["dwg", "dxf"];
pub const IMAGE: &[&str] = &["png", "jpg", "jpeg", "webp", "tiff", "tif", "gif", "bmp"];

pub fn handles(ext: &str) -> bool {
    SUPPORTED.contains(&ext.to_ascii_lowercase().as_str())
}

/// Extract `path` by extension.
pub fn extract(path: &Path, ctx: &ExtractCtx) -> Result<Extracted> {
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "pdf" => pdf::extract(path, ctx),
        "docx" => office::extract_docx(path),
        "pptx" => office::extract_pptx(path),
        "xlsx" | "xlsm" | "xls" | "ods" => sheets::extract_workbook(path),
        "csv" | "tsv" => sheets::extract_csv(path),
        "txt" | "md" | "markdown" => text::extract_plain(path),
        "html" | "htm" => text::extract_html(path),
        "rtf" => text::extract_rtf(path),
        e if NAME_ONLY.contains(&e) => Ok(name_only(path, "drawing")),
        e if IMAGE.contains(&e) => Ok(name_only(path, "image")),
        other => Err(ExtractError::Unsupported(other.to_string())),
    }
}

/// For files whose content is not read: the name and folder become the text.
pub fn name_only(path: &Path, kind: &str) -> Extracted {
    let name = path
        .file_name()
        .map(|f| f.to_string_lossy().to_string())
        .unwrap_or_default();
    let folder = path
        .parent()
        .and_then(|p| p.file_name())
        .map(|f| f.to_string_lossy().to_string())
        .unwrap_or_default();
    let words = name
        .rsplit_once('.')
        .map(|(s, _)| s)
        .unwrap_or(&name)
        .replace(['_', '-'], " ");
    Extracted {
        text: format!("{kind} file {name} in folder {folder}: {words}\n"),
        units: 1,
        unit_kind: "file",
    }
}

pub(crate) fn io_err(path: &Path, source: std::io::Error) -> ExtractError {
    ExtractError::Io {
        path: path.to_path_buf(),
        source,
    }
}
