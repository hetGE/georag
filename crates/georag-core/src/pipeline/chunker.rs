//! Chunking (handoff 6.5): text-splitter with the chat tokenizer as the
//! sizer; page, sheet and slide markers set each chunk's locator.

use georag_engine::{InferenceEngine, Role};
use georag_store::NewChunk;
use text_splitter::{ChunkConfig, ChunkSizer, TextSplitter};

use crate::error::Result;

struct EngineSizer<'a> {
    engine: &'a dyn InferenceEngine,
}

impl ChunkSizer for EngineSizer<'_> {
    fn size(&self, chunk: &str) -> usize {
        match self.engine.tokenize(Role::Chat, chunk) {
            Ok(t) => t.len(),
            Err(_) => chunk.len().div_ceil(4),
        }
    }
}

/// A marker line such as `[Page 3]`, `[Sheet: Results]` or `[Slide 4]`.
fn marker_at(line: &str) -> Option<(Option<i64>, String)> {
    let line = line.trim();
    if !line.starts_with('[') || !line.ends_with(']') {
        return None;
    }
    let inner = &line[1..line.len() - 1];
    if let Some(n) = inner.strip_prefix("Page ") {
        let n = n.trim().parse::<i64>().ok()?;
        return Some((Some(n), format!("p.{n}")));
    }
    if let Some(name) = inner.strip_prefix("Sheet: ") {
        return Some((None, format!("sheet:{}", name.trim())));
    }
    if let Some(n) = inner.strip_prefix("Slide ") {
        let n = n.trim().parse::<i64>().ok()?;
        return Some((Some(n), format!("slide {n}")));
    }
    None
}

/// Split `text` into chunks of about `chunk_tokens` tokens with `overlap`,
/// counting with the chat tokenizer. Each chunk carries the last marker
/// seen before it starts.
pub fn chunk_text(
    engine: &dyn InferenceEngine,
    text: &str,
    chunk_tokens: usize,
    overlap: usize,
) -> Result<Vec<NewChunk>> {
    let sizer = EngineSizer { engine };
    let config = ChunkConfig::new(chunk_tokens.max(32))
        .with_sizer(&sizer)
        .with_overlap(overlap.min(chunk_tokens / 2))
        .map_err(|e| crate::CoreError::Invalid(format!("chunk settings: {e}")))?
        .with_trim(true);
    let splitter = TextSplitter::new(config);

    // Marker positions by byte offset.
    let mut markers: Vec<(usize, Option<i64>, String)> = Vec::new();
    let mut offset = 0usize;
    for line in text.split_inclusive('\n') {
        if let Some((page, locator)) = marker_at(line) {
            markers.push((offset, page, locator));
        }
        offset += line.len();
    }

    let mut chunks = Vec::new();
    for (idx, (start, piece)) in splitter.chunk_indices(text).enumerate() {
        if piece.trim().is_empty() {
            continue;
        }
        let tokens = sizer.size(piece) as i64;
        let (page, locator) = markers
            .iter()
            .rev()
            .find(|(o, _, _)| *o <= start)
            .map(|(_, p, l)| (*p, Some(l.clone())))
            .unwrap_or((None, None));
        chunks.push(NewChunk {
            idx: idx as i64,
            text: piece.to_string(),
            tokens,
            page,
            locator,
        });
    }
    // Re-number after skipping empties.
    for (i, c) in chunks.iter_mut().enumerate() {
        c.idx = i as i64;
    }
    Ok(chunks)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markers_parse() {
        assert_eq!(marker_at("[Page 12]\n"), Some((Some(12), "p.12".into())));
        assert_eq!(
            marker_at("[Sheet: Results 2024]"),
            Some((None, "sheet:Results 2024".into()))
        );
        assert_eq!(marker_at("[Slide 4]"), Some((Some(4), "slide 4".into())));
        assert_eq!(marker_at("[S3] not a marker"), None);
        assert_eq!(marker_at("plain"), None);
    }
}
