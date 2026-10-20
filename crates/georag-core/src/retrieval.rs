//! Hybrid retrieval (handoff 7.1): vector KNN plus FTS5 keywords, unioned by
//! chunk id, cosine on every candidate, reciprocal rank fusion for ordering.

use std::collections::HashMap;

use georag_engine::{InferenceEngine, Lane};
use georag_store::{SearchFilter, Store};
use serde::Serialize;

use crate::error::Result;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Candidate {
    pub chunk_id: i64,
    pub file_id: i64,
    pub rel_path: String,
    pub idx: i64,
    pub page: Option<i64>,
    pub locator: Option<String>,
    pub text: String,
    pub tokens: u32,
    /// Cosine similarity: the relevance score.
    pub score: f32,
    /// Reciprocal rank fusion of the vector and keyword orderings.
    pub rrf: f32,
}

pub const RRF_K: f32 = 60.0;

/// Embed the question (with the model's query prefix) and gather candidates.
pub fn hybrid_retrieve(
    store: &Store,
    engine: &dyn InferenceEngine,
    question: &str,
    tags: &[String],
    candidate_k: usize,
    query_prefix: &str,
) -> Result<Vec<Candidate>> {
    let filter = SearchFilter {
        tags: tags.to_vec(),
    };
    let q = engine.embed(&[format!("{query_prefix}{question}")], Lane::Interactive)?;
    let q = q.into_iter().next().unwrap_or_default();
    let vec_hits = store.knn(&q, &filter, candidate_k)?;
    let kw_hits = store.fts(question, &filter, candidate_k)?;
    fuse(store, &q, &vec_hits, &kw_hits)
}

/// Union the two hit lists, fill in cosine for keyword-only hits, compute
/// RRF, and load the chunk rows. Ordered by cosine, ties by RRF.
pub fn fuse(
    store: &Store,
    query: &[f32],
    vec_hits: &[georag_store::Hit],
    kw_hits: &[georag_store::Hit],
) -> Result<Vec<Candidate>> {
    let mut score: HashMap<i64, f32> = HashMap::new();
    let mut rrf: HashMap<i64, f32> = HashMap::new();
    for (rank, h) in vec_hits.iter().enumerate() {
        score.insert(h.chunk_id, h.score);
        *rrf.entry(h.chunk_id).or_default() += 1.0 / (RRF_K + rank as f32 + 1.0);
    }
    let mut kw_only = Vec::new();
    for (rank, h) in kw_hits.iter().enumerate() {
        *rrf.entry(h.chunk_id).or_default() += 1.0 / (RRF_K + rank as f32 + 1.0);
        if !score.contains_key(&h.chunk_id) {
            kw_only.push(h.chunk_id);
        }
    }
    if !kw_only.is_empty() {
        for h in store.cosine_for(&kw_only, query)? {
            score.insert(h.chunk_id, h.score);
        }
    }
    let ids: Vec<i64> = score.keys().copied().collect();
    let rows = store.chunks_by_ids(&ids)?;
    let mut out: Vec<Candidate> = rows
        .into_iter()
        .map(|c| Candidate {
            score: *score.get(&c.id).unwrap_or(&0.0),
            rrf: *rrf.get(&c.id).unwrap_or(&0.0),
            chunk_id: c.id,
            file_id: c.file_id,
            rel_path: c.rel_path,
            idx: c.idx,
            page: c.page,
            locator: c.locator,
            text: c.text,
            tokens: c.tokens.max(0) as u32,
        })
        .collect();
    out.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(
                b.rrf
                    .partial_cmp(&a.rrf)
                    .unwrap_or(std::cmp::Ordering::Equal),
            )
            .then(a.chunk_id.cmp(&b.chunk_id))
    });
    Ok(out)
}

/// Pure keyword and vector search for `georag search`.
#[derive(Debug, Clone, Serialize)]
pub struct SearchResult {
    pub chunk_id: i64,
    pub rel_path: String,
    pub locator: Option<String>,
    pub score: f32,
    pub rrf: f32,
    pub snippet: String,
}

pub fn search(
    store: &Store,
    engine: &dyn InferenceEngine,
    query: &str,
    tags: &[String],
    k: usize,
    query_prefix: &str,
) -> Result<Vec<SearchResult>> {
    let cands = hybrid_retrieve(store, engine, query, tags, k.max(20), query_prefix)?;
    Ok(cands
        .into_iter()
        .take(k)
        .map(|c| SearchResult {
            chunk_id: c.chunk_id,
            rel_path: c.rel_path,
            locator: c.locator,
            score: c.score,
            rrf: c.rrf,
            snippet: c
                .text
                .chars()
                .take(200)
                .collect::<String>()
                .replace('\n', " "),
        })
        .collect())
}
