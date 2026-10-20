//! Document extractors: pdf, docx, xlsx, pptx, text, html, images, OCR.
//!
//! P2 fills this crate. It is the only crate besides `georag-engine` allowed
//! to hold `unsafe` (for pdfium); that allowance is granted per module when
//! the pdfium extractor lands, with the invariant stated on each block.

/// Errors from extraction.
#[derive(Debug, thiserror::Error)]
pub enum ExtractError {
    #[error("no extractor handles files with extension {0:?}")]
    Unsupported(String),
}
