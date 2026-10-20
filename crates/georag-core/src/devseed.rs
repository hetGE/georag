//! A synthetic geotechnical corpus for tests and demos: several site
//! reports, each a few chunks of roughly a thousand tokens, with distinct
//! numbers so answers can be checked.

use georag_engine::{InferenceEngine, Lane, Role};
use georag_store::{FileState, NewChunk, Store};

use crate::error::Result;

pub const SITES: [&str; 8] = [
    "Riverbend",
    "Harbour Quay",
    "Blackstone Ridge",
    "Millbrook",
    "Saltmarsh",
    "Ironbark",
    "Cove Point",
    "Westgate",
];

/// Chunk text for site `s`, chunk `i`. Deterministic.
pub fn chunk_text(s: usize, i: usize) -> String {
    let site = SITES[s % SITES.len()];
    let pile_len = 12 + s * 2 + i;
    let capacity = 900 + s * 150 + i * 40;
    let su = 60 + s * 10 + i * 5;
    let mut out = String::new();
    out.push_str(&format!(
        "[Page {}]\nGeotechnical interpretive report, {site} development. Section {}: pile design and ground conditions.\n\n",
        i + 1,
        i + 1
    ));
    for p in 0..6 {
        out.push_str(&format!(
            "Borehole BH-{s}{i}{p} at {site} encountered {} metres of fill over firm to stiff clay with an undrained shear strength of {} kPa, \
grading to very stiff clay below {} metres and weathered sandstone at {} metres. Groundwater was measured at {} metres below ground level. \
The adopted design uses bored piles {} metres long and {} millimetres in diameter with an ultimate geotechnical capacity of {} kN per pile; \
the serviceability settlement estimate at working load is {} millimetres. Shaft adhesion was taken as alpha {} on the undrained strength and end bearing as nine times the strength at the toe. \
Negative skin friction from the fill was included for the {site} piles at {} kN. Load testing on pile PT-{p} reached {} kN without plunging failure.\n\n",
            2 + p % 3,
            su + p * 3,
            8 + p,
            18 + p * 2 + s,
            3 + p % 2,
            pile_len,
            600 + i * 150,
            capacity,
            8 + p,
            if p % 2 == 0 { "0.5" } else { "0.45" },
            80 + s * 5,
            capacity + 200 + p * 10,
        ));
    }
    out
}

/// Expected headline facts for site `s`, chunk `i`, for checking answers.
pub fn expected_capacity(s: usize, i: usize) -> usize {
    900 + s * 150 + i * 40
}

/// Seed `sites` documents of `chunks_per_doc` chunks each, embedding through
/// the engine and counting tokens with the chat tokenizer. Returns the
/// total chunk tokens.
pub fn seed(
    store: &Store,
    engine: &dyn InferenceEngine,
    sites: usize,
    chunks_per_doc: usize,
    doc_prefix: &str,
) -> Result<u32> {
    let root = match store.roots()?.into_iter().find(|r| r.name == "demo") {
        Some(r) => r,
        None => store.root_add("demo", std::path::Path::new("/demo"))?,
    };
    let mut total = 0u32;
    for s in 0..sites {
        let rel = format!(
            "reports/{}-interpretive-report.pdf",
            SITES[s % SITES.len()].to_lowercase().replace(' ', "-")
        );
        let file_id = store.file_upsert(root.id, &rel, None, FileState::Extracted)?;
        let mut chunks = Vec::new();
        let mut texts = Vec::new();
        for i in 0..chunks_per_doc {
            let text = chunk_text(s, i);
            let tokens = engine.tokenize(Role::Chat, &text)?.len() as i64;
            total += tokens as u32;
            texts.push(format!("{doc_prefix}{text}"));
            chunks.push(NewChunk {
                idx: i as i64,
                text,
                tokens,
                page: Some(i as i64 + 1),
                locator: Some(format!("p.{}", i + 1)),
            });
        }
        let embeddings = engine.embed(&texts, Lane::Bulk)?;
        store.chunks_replace_for_file(file_id, &chunks, Some(&embeddings))?;
    }
    Ok(total)
}
