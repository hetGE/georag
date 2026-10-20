//! Auto-depth planner (handoff 7.4): keep everything above the relevance
//! threshold, group by document, first-fit into passes.

use serde::Serialize;

use crate::retrieval::Candidate;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Pass {
    /// Chunk ids in prompt order.
    pub chunk_ids: Vec<i64>,
    pub tokens: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub enum Plan {
    Single {
        chunk_ids: Vec<i64>,
        tokens: u32,
        fallback: bool,
    },
    Multi {
        passes: Vec<Pass>,
        dropped: Vec<i64>,
        fallback: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PlanSummary {
    pub passes: usize,
    pub kept: usize,
    pub total_tokens: u32,
    pub dropped: usize,
    pub fallback: bool,
    pub budget: u32,
    pub min_relevance: f32,
}

impl Plan {
    pub fn summary(&self, budget: u32, min_relevance: f32) -> PlanSummary {
        match self {
            Plan::Single {
                chunk_ids,
                tokens,
                fallback,
            } => PlanSummary {
                passes: 1,
                kept: chunk_ids.len(),
                total_tokens: *tokens,
                dropped: 0,
                fallback: *fallback,
                budget,
                min_relevance,
            },
            Plan::Multi {
                passes,
                dropped,
                fallback,
            } => PlanSummary {
                passes: passes.len(),
                kept: passes.iter().map(|p| p.chunk_ids.len()).sum(),
                total_tokens: passes.iter().map(|p| p.tokens).sum(),
                dropped: dropped.len(),
                fallback: *fallback,
                budget,
                min_relevance,
            },
        }
    }

    /// Chunk ids in run order (pass by pass).
    pub fn ordered_chunk_ids(&self) -> Vec<i64> {
        match self {
            Plan::Single { chunk_ids, .. } => chunk_ids.clone(),
            Plan::Multi { passes, .. } => passes
                .iter()
                .flat_map(|p| p.chunk_ids.iter().copied())
                .collect(),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct PlanConfig {
    pub min_relevance: f32,
    pub max_passes: usize,
    pub min_candidates: usize,
}

struct Piece {
    chunk_ids: Vec<i64>,
    tokens: u32,
}

pub fn plan_auto(cands: &[Candidate], cfg: PlanConfig, budget: u32) -> Plan {
    let mut kept: Vec<&Candidate> = cands
        .iter()
        .filter(|c| c.score >= cfg.min_relevance)
        .collect();
    let fallback = kept.is_empty();
    if fallback {
        let mut by_rrf: Vec<&Candidate> = cands.iter().collect();
        by_rrf.sort_by(|a, b| {
            b.rrf
                .partial_cmp(&a.rrf)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        kept = by_rrf.into_iter().take(cfg.min_candidates).collect();
    }
    let total: u32 = kept.iter().map(|c| c.tokens).sum();
    if total <= budget {
        return Plan::Single {
            chunk_ids: kept.iter().map(|c| c.chunk_id).collect(),
            tokens: total,
            fallback,
        };
    }

    // Group by document: relevance = max chunk score, chunks in file order.
    let mut docs: Vec<(i64, f32, Vec<&Candidate>)> = Vec::new();
    for c in &kept {
        match docs.iter_mut().find(|(f, _, _)| *f == c.file_id) {
            Some((_, rel, chunks)) => {
                if c.score > *rel {
                    *rel = c.score;
                }
                chunks.push(c);
            }
            None => docs.push((c.file_id, c.score, vec![c])),
        }
    }
    docs.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    for (_, _, chunks) in &mut docs {
        chunks.sort_by_key(|c| c.idx);
    }

    let mut passes: Vec<Piece> = Vec::new();
    let mut dropped: Vec<i64> = Vec::new();
    for (_, _, chunks) in docs {
        let doc_tokens: u32 = chunks.iter().map(|c| c.tokens).sum();
        let pieces: Vec<Piece> = if doc_tokens <= budget {
            vec![Piece {
                chunk_ids: chunks.iter().map(|c| c.chunk_id).collect(),
                tokens: doc_tokens,
            }]
        } else {
            split_in_page_order(&chunks, budget)
        };
        for piece in pieces {
            if piece.tokens > budget {
                dropped.extend(piece.chunk_ids);
                continue;
            }
            match passes
                .iter()
                .position(|p| p.tokens + piece.tokens <= budget)
            {
                Some(i) => {
                    passes[i].chunk_ids.extend(piece.chunk_ids);
                    passes[i].tokens += piece.tokens;
                }
                None if passes.len() < cfg.max_passes => passes.push(piece),
                None => dropped.extend(piece.chunk_ids),
            }
        }
    }
    Plan::Multi {
        passes: passes
            .into_iter()
            .map(|p| Pass {
                chunk_ids: p.chunk_ids,
                tokens: p.tokens,
            })
            .collect(),
        dropped,
        fallback,
    }
}

/// The only case where a document is split: consecutive chunks packed up to
/// the budget, in page order.
fn split_in_page_order(chunks: &[&Candidate], budget: u32) -> Vec<Piece> {
    let mut pieces = Vec::new();
    let mut cur = Piece {
        chunk_ids: Vec::new(),
        tokens: 0,
    };
    for c in chunks {
        if !cur.chunk_ids.is_empty() && cur.tokens + c.tokens > budget {
            pieces.push(std::mem::replace(
                &mut cur,
                Piece {
                    chunk_ids: Vec::new(),
                    tokens: 0,
                },
            ));
        }
        cur.chunk_ids.push(c.chunk_id);
        cur.tokens += c.tokens;
    }
    if !cur.chunk_ids.is_empty() {
        pieces.push(cur);
    }
    pieces
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cand(id: i64, file: i64, idx: i64, tokens: u32, score: f32) -> Candidate {
        Candidate {
            chunk_id: id,
            file_id: file,
            rel_path: format!("doc{file}.pdf"),
            idx,
            page: Some(idx + 1),
            locator: Some(format!("p.{}", idx + 1)),
            text: String::new(),
            tokens,
            score,
            rrf: 1.0 / (60.0 + id as f32),
        }
    }

    const CFG: PlanConfig = PlanConfig {
        min_relevance: 0.65,
        max_passes: 6,
        min_candidates: 16,
    };

    #[test]
    fn everything_fits_is_a_single_pass() {
        let c = vec![
            cand(1, 1, 0, 500, 0.9),
            cand(2, 1, 1, 500, 0.8),
            cand(3, 2, 0, 100, 0.2),
        ];
        match plan_auto(&c, CFG, 2000) {
            Plan::Single {
                chunk_ids,
                tokens,
                fallback,
            } => {
                assert_eq!(chunk_ids, vec![1, 2]);
                assert_eq!(tokens, 1000);
                assert!(!fallback);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn fallback_takes_min_candidates_by_rrf() {
        let c: Vec<Candidate> = (1..=20).map(|i| cand(i, i, 0, 10, 0.3)).collect();
        match plan_auto(&c, CFG, 10_000) {
            Plan::Single {
                chunk_ids,
                fallback,
                ..
            } => {
                assert!(fallback);
                assert_eq!(chunk_ids.len(), 16);
                assert_eq!(chunk_ids[0], 1);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn documents_pack_first_fit_in_relevance_order() {
        // doc1: 2 chunks of 600 (max 0.95); doc2: 1 chunk of 900 (0.9); doc3: 1 chunk of 300 (0.8)
        let c = vec![
            cand(1, 1, 1, 600, 0.7),
            cand(2, 1, 0, 600, 0.95),
            cand(3, 2, 0, 900, 0.9),
            cand(4, 3, 0, 300, 0.8),
        ];
        match plan_auto(&c, CFG, 1500) {
            Plan::Multi {
                passes,
                dropped,
                fallback,
            } => {
                assert!(!fallback);
                assert!(dropped.is_empty());
                assert_eq!(passes.len(), 2);
                // doc1 first (page order inside it), doc3 fits beside it; doc2 opens pass 2.
                assert_eq!(passes[0].chunk_ids, vec![2, 1, 4]);
                assert_eq!(passes[0].tokens, 1500);
                assert_eq!(passes[1].chunk_ids, vec![3]);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn oversized_document_is_split_in_page_order() {
        let c = vec![
            cand(1, 1, 0, 700, 0.9),
            cand(2, 1, 1, 700, 0.9),
            cand(3, 1, 2, 700, 0.9),
        ];
        match plan_auto(&c, CFG, 1500) {
            Plan::Multi {
                passes, dropped, ..
            } => {
                assert!(dropped.is_empty());
                assert_eq!(passes.len(), 2);
                assert_eq!(passes[0].chunk_ids, vec![1, 2]);
                assert_eq!(passes[1].chunk_ids, vec![3]);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn max_passes_drops_the_rest() {
        let c: Vec<Candidate> = (1..=10)
            .map(|i| cand(i, i, 0, 900, 0.9 - i as f32 * 0.01))
            .collect();
        let cfg = PlanConfig {
            max_passes: 3,
            ..CFG
        };
        match plan_auto(&c, cfg, 1000) {
            Plan::Multi {
                passes, dropped, ..
            } => {
                assert_eq!(passes.len(), 3);
                assert_eq!(dropped.len(), 7);
                assert_eq!(passes[0].chunk_ids, vec![1]);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn single_chunk_over_budget_is_dropped() {
        let c = vec![cand(1, 1, 0, 5000, 0.9), cand(2, 2, 0, 100, 0.9)];
        match plan_auto(&c, CFG, 1000) {
            Plan::Multi {
                passes, dropped, ..
            } => {
                assert_eq!(dropped, vec![1]);
                assert_eq!(passes.len(), 1);
            }
            other => panic!("{other:?}"),
        }
    }
}
