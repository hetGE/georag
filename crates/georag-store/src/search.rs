//! Vector KNN (sqlite-vec, cosine), keyword search (FTS5 bm25) and cosine
//! lookups for fusion in core.

use rusqlite::{Connection, params};

use crate::Result;
use crate::StoreError;
use crate::chunks::{blob_to_vec, vec_to_blob};
use crate::store::Store;

/// Restrict search to chunks of files carrying any of these tags.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SearchFilter {
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Hit {
    pub chunk_id: i64,
    /// Cosine similarity for `knn`, bm25 rank score (higher is better) for `fts`.
    pub score: f32,
}

fn tag_subquery(filter: &SearchFilter) -> Option<(String, Vec<String>)> {
    if filter.tags.is_empty() {
        return None;
    }
    let placeholders = vec!["?"; filter.tags.len()].join(",");
    Some((
        format!(
            "SELECT c.id FROM chunks c JOIN file_tags ft ON ft.file_id = c.file_id JOIN tags t ON t.id = ft.tag_id WHERE t.name IN ({placeholders})"
        ),
        filter.tags.clone(),
    ))
}

/// Turn free text into an FTS5 query: quoted terms joined with OR, so
/// punctuation never breaks the parser and any matching term counts.
pub fn fts_query(text: &str) -> Option<String> {
    let mut terms: Vec<String> = Vec::new();
    for raw in text.split(|c: char| !c.is_alphanumeric() && c != '_' && c != '-') {
        let t = raw.trim_matches('-').to_lowercase();
        if t.chars().count() < 2 || terms.contains(&t) {
            continue;
        }
        terms.push(t);
        if terms.len() >= 16 {
            break;
        }
    }
    if terms.is_empty() {
        return None;
    }
    Some(
        terms
            .iter()
            .map(|t| format!("\"{}\"", t.replace('"', "")))
            .collect::<Vec<_>>()
            .join(" OR "),
    )
}

impl Store {
    /// Nearest chunks by cosine similarity.
    pub fn knn(&self, query: &[f32], filter: &SearchFilter, k: usize) -> Result<Vec<Hit>> {
        if self.embed_info()?.is_none() {
            return Err(StoreError::NoVectorTable);
        }
        self.read(|c| knn_on(c, query, filter, k))
    }

    /// Keyword hits ranked by bm25 (returned as a positive score).
    pub fn fts(&self, text: &str, filter: &SearchFilter, k: usize) -> Result<Vec<Hit>> {
        let Some(q) = fts_query(text) else {
            return Ok(Vec::new());
        };
        self.read(|c| {
            // Positional placeholders only: mixing `?N` with bare `?` renumbers them.
            let (sql, binds): (String, Vec<String>) = match tag_subquery(filter) {
                Some((sub, tags)) => (
                    format!(
                        "SELECT rowid, bm25(chunk_fts) FROM chunk_fts WHERE chunk_fts MATCH ? AND rowid IN ({sub}) ORDER BY bm25(chunk_fts) LIMIT ?"
                    ),
                    tags,
                ),
                None => (
                    "SELECT rowid, bm25(chunk_fts) FROM chunk_fts WHERE chunk_fts MATCH ? ORDER BY bm25(chunk_fts) LIMIT ?"
                        .to_string(),
                    Vec::new(),
                ),
            };
            let mut st = c.prepare(&sql)?;
            let mut values: Vec<rusqlite::types::Value> = vec![q.clone().into()];
            values.extend(binds.into_iter().map(rusqlite::types::Value::from));
            values.push((k as i64).into());
            let rows = st.query_map(rusqlite::params_from_iter(values.iter()), |r| {
                let bm25: f64 = r.get(1)?;
                Ok(Hit { chunk_id: r.get(0)?, score: (-bm25) as f32 })
            })?;
            Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
        })
    }

    /// Cosine similarity between `query` and the stored vectors of `chunk_ids`.
    /// Missing vectors are skipped.
    pub fn cosine_for(&self, chunk_ids: &[i64], query: &[f32]) -> Result<Vec<Hit>> {
        if chunk_ids.is_empty() {
            return Ok(Vec::new());
        }
        if self.embed_info()?.is_none() {
            return Err(StoreError::NoVectorTable);
        }
        self.read(|c| {
            let placeholders = vec!["?"; chunk_ids.len()].join(",");
            let mut st = c.prepare(&format!(
                "SELECT chunk_id, embedding FROM chunk_vec WHERE chunk_id IN ({placeholders})"
            ))?;
            let rows = st.query_map(rusqlite::params_from_iter(chunk_ids.iter()), |r| {
                let id: i64 = r.get(0)?;
                let blob: Vec<u8> = r.get(1)?;
                Ok((id, blob))
            })?;
            let mut hits = Vec::new();
            for row in rows {
                let (id, blob) = row?;
                let v = blob_to_vec(&blob);
                hits.push(Hit {
                    chunk_id: id,
                    score: cosine(query, &v),
                });
            }
            Ok(hits)
        })
    }
}

fn knn_on(c: &Connection, query: &[f32], filter: &SearchFilter, k: usize) -> Result<Vec<Hit>> {
    let blob = vec_to_blob(query);
    let mut hits = Vec::new();
    match tag_subquery(filter) {
        None => {
            let mut st = c.prepare("SELECT chunk_id, distance FROM chunk_vec WHERE embedding MATCH ?1 AND k = ?2 ORDER BY distance")?;
            let rows = st.query_map(params![blob, k as i64], |r| {
                let d: f64 = r.get(1)?;
                Ok(Hit {
                    chunk_id: r.get(0)?,
                    score: (1.0 - d) as f32,
                })
            })?;
            for h in rows {
                hits.push(h?);
            }
        }
        Some((sub, tags)) => {
            let sql = format!(
                "SELECT chunk_id, distance FROM chunk_vec WHERE embedding MATCH ? AND k = ? AND chunk_id IN ({sub}) ORDER BY distance"
            );
            let mut st = c.prepare(&sql)?;
            let mut values: Vec<rusqlite::types::Value> = vec![blob.into(), (k as i64).into()];
            values.extend(tags.into_iter().map(rusqlite::types::Value::from));
            let rows = st.query_map(rusqlite::params_from_iter(values.iter()), |r| {
                let d: f64 = r.get(1)?;
                Ok(Hit {
                    chunk_id: r.get(0)?,
                    score: (1.0 - d) as f32,
                })
            })?;
            for h in rows {
                hits.push(h?);
            }
        }
    }
    Ok(hits)
}

/// Cosine similarity; vectors are expected L2-normalised but this is exact
/// either way.
pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    let mut dot = 0.0f32;
    let mut na = 0.0f32;
    let mut nb = 0.0f32;
    for (x, y) in a.iter().zip(b) {
        dot += x * y;
        na += x * x;
        nb += y * y;
    }
    if na == 0.0 || nb == 0.0 {
        0.0
    } else {
        dot / (na.sqrt() * nb.sqrt())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunks::NewChunk;
    use crate::files::FileState;

    fn unit(v: &[f32]) -> Vec<f32> {
        let n = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        v.iter().map(|x| x / n).collect()
    }

    fn seeded() -> (Store, Vec<i64>, Vec<i64>) {
        let store = Store::open_in_memory().unwrap();
        store.ensure_vectors("test-embed", 3).unwrap();
        let root = store.root_add("lib", std::path::Path::new("/lib")).unwrap();
        let piles = store
            .file_upsert(root.id, "piles/report.pdf", Some(10), FileState::New)
            .unwrap();
        let slopes = store
            .file_upsert(root.id, "slopes/notes.txt", Some(10), FileState::New)
            .unwrap();
        let a = store
            .chunks_replace_for_file(
                piles,
                &[
                    NewChunk {
                        idx: 0,
                        text: "bored pile capacity in clay".into(),
                        tokens: 6,
                        page: Some(1),
                        locator: Some("p.1".into()),
                    },
                    NewChunk {
                        idx: 1,
                        text: "driven pile set and hammer energy".into(),
                        tokens: 7,
                        page: Some(2),
                        locator: Some("p.2".into()),
                    },
                ],
                Some(&[unit(&[1.0, 0.1, 0.0]), unit(&[0.9, 0.3, 0.0])]),
            )
            .unwrap();
        let b = store
            .chunks_replace_for_file(
                slopes,
                &[NewChunk {
                    idx: 0,
                    text: "slope stability factor of safety".into(),
                    tokens: 6,
                    page: None,
                    locator: None,
                }],
                Some(&[unit(&[0.0, 0.0, 1.0])]),
            )
            .unwrap();
        (store, a, b)
    }

    #[test]
    fn knn_orders_by_cosine() {
        let (store, a, b) = seeded();
        let hits = store
            .knn(&unit(&[1.0, 0.0, 0.0]), &SearchFilter::default(), 10)
            .unwrap();
        assert_eq!(hits.len(), 3);
        assert_eq!(hits[0].chunk_id, a[0]);
        assert!(hits[0].score > 0.99);
        assert_eq!(hits[2].chunk_id, b[0]);
        assert!(hits[2].score.abs() < 0.01);
    }

    #[test]
    fn fts_finds_keywords_and_cosine_for_fills_scores() {
        let (store, a, _b) = seeded();
        let hits = store
            .fts("What is the pile capacity?", &SearchFilter::default(), 10)
            .unwrap();
        assert!(hits.iter().any(|h| h.chunk_id == a[0]));
        let cos = store
            .cosine_for(&[a[0], a[1]], &unit(&[1.0, 0.0, 0.0]))
            .unwrap();
        assert_eq!(cos.len(), 2);
        assert!(cos.iter().all(|h| h.score > 0.9));
    }

    #[test]
    fn tag_filter_restricts_both_searches() {
        let (store, a, b) = seeded();
        {
            let conn = store.writer();
            conn.execute(
                "INSERT INTO tags(name, display_name) VALUES ('piling', 'Piling')",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO file_tags(file_id, tag_id, source, confidence, created_at) SELECT file_id, 1, 'manual', 1.0, 'now' FROM chunks WHERE id = ?1",
                [a[0]],
            )
            .unwrap();
        }
        let filter = SearchFilter {
            tags: vec!["piling".into()],
        };
        let hits = store.knn(&unit(&[0.0, 0.0, 1.0]), &filter, 10).unwrap();
        assert!(hits.iter().all(|h| h.chunk_id != b[0]));
        assert_eq!(hits.len(), 2);
        let hits = store.fts("slope stability pile", &filter, 10).unwrap();
        assert!(hits.iter().all(|h| h.chunk_id != b[0]));
    }

    #[test]
    fn replacing_chunks_keeps_other_files_ids_stable() {
        let (store, a, b) = seeded();
        let piles_file = store.chunks_by_ids(&[a[0]]).unwrap()[0].file_id;
        let new_ids = store
            .chunks_replace_for_file(
                piles_file,
                &[NewChunk {
                    idx: 0,
                    text: "pile cap design".into(),
                    tokens: 3,
                    page: None,
                    locator: None,
                }],
                Some(&[unit(&[1.0, 0.0, 0.0])]),
            )
            .unwrap();
        assert_ne!(new_ids[0], a[0]);
        assert_eq!(store.chunk_count().unwrap(), 2);
        let again = store.chunks_by_ids(&[b[0]]).unwrap();
        assert_eq!(again[0].text, "slope stability factor of safety");
        let file = store.file_get(piles_file).unwrap().unwrap();
        assert_eq!(file.chunk_count, 1);
        assert_eq!(file.state, FileState::Indexed);
    }

    #[test]
    fn dimension_mismatch_is_refused() {
        let store = Store::open_in_memory().unwrap();
        store.ensure_vectors("a", 3).unwrap();
        assert!(matches!(
            store.ensure_vectors("b", 4),
            Err(StoreError::DimensionMismatch {
                existing: 3,
                requested: 4
            })
        ));
        store.reset_vectors().unwrap();
        store.ensure_vectors("b", 4).unwrap();
    }

    #[test]
    fn fts_query_sanitises_punctuation() {
        assert_eq!(
            fts_query("What's the pile-capacity (kN)?").unwrap(),
            "\"what\" OR \"the\" OR \"pile-capacity\" OR \"kn\""
        );
        assert_eq!(fts_query("?? !"), None);
    }
}
