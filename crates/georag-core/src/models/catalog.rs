//! The curated catalogue: leading GGUF chat, vision and embedding models.
//! Repos and file names verified against the Hugging Face tree API on
//! 2026-10-20; sizes are the LFS sizes reported then. The user can also
//! paste any `org/repo[:file]`.

use serde::Serialize;

use super::registry::ModelRole;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CatalogEntry {
    pub id: &'static str,
    pub role: ModelRole,
    pub repo: &'static str,
    pub file: &'static str,
    pub size_bytes: u64,
    pub licence: &'static str,
    pub notes: &'static str,
    /// For embedding models: the task prefixes applied at index and query time.
    pub embed_prefixes: Option<(&'static str, &'static str)>,
    /// Vision projector that pairs with this chat model, by catalogue id.
    pub pairs_with: Option<&'static str>,
    /// Relevance threshold that suits this embedding model (section 7.5).
    pub min_relevance: Option<f32>,
}

const CATALOG: &[CatalogEntry] = &[
    CatalogEntry {
        id: "qwen3.5-9b-q4_k_m",
        role: ModelRole::Chat,
        repo: "lmstudio-community/Qwen3.5-9B-GGUF",
        file: "Qwen3.5-9B-Q4_K_M.gguf",
        size_bytes: 5_627_044_256,
        licence: "Apache-2.0",
        notes: "Default chat model. 262k context, multimodal with the matching mmproj.",
        embed_prefixes: None,
        pairs_with: Some("mmproj-qwen3.5-9b-bf16"),
        min_relevance: None,
    },
    CatalogEntry {
        id: "mmproj-qwen3.5-9b-bf16",
        role: ModelRole::Vision,
        repo: "lmstudio-community/Qwen3.5-9B-GGUF",
        file: "mmproj-Qwen3.5-9B-BF16.gguf",
        size_bytes: 921_704_480,
        licence: "Apache-2.0",
        notes: "Vision projector for Qwen3.5-9B; needed for image captions.",
        embed_prefixes: None,
        pairs_with: None,
        min_relevance: None,
    },
    CatalogEntry {
        id: "qwen3-8b-q4_k_m",
        role: ModelRole::Chat,
        repo: "bartowski/Qwen_Qwen3-8B-GGUF",
        file: "Qwen_Qwen3-8B-Q4_K_M.gguf",
        size_bytes: 5_027_784_224,
        licence: "Apache-2.0",
        notes: "Qwen3 8B, 40k context (128k with YaRN). Text only.",
        embed_prefixes: None,
        pairs_with: None,
        min_relevance: None,
    },
    CatalogEntry {
        id: "qwen3-4b-q4_k_m",
        role: ModelRole::Chat,
        repo: "bartowski/Qwen_Qwen3-4B-GGUF",
        file: "Qwen_Qwen3-4B-Q4_K_M.gguf",
        size_bytes: 2_497_280_960,
        licence: "Apache-2.0",
        notes: "Small and quick; suits 8 GB machines and testing.",
        embed_prefixes: None,
        pairs_with: None,
        min_relevance: None,
    },
    CatalogEntry {
        id: "gemma-3-12b-it-q4_k_m",
        role: ModelRole::Chat,
        repo: "ggml-org/gemma-3-12b-it-GGUF",
        file: "gemma-3-12b-it-Q4_K_M.gguf",
        size_bytes: 7_300_574_976,
        licence: "Gemma Terms of Use",
        notes: "Gemma 3 12B instruct, 128k context, multimodal with its mmproj.",
        embed_prefixes: None,
        pairs_with: Some("mmproj-gemma-3-12b-f16"),
        min_relevance: None,
    },
    CatalogEntry {
        id: "mmproj-gemma-3-12b-f16",
        role: ModelRole::Vision,
        repo: "ggml-org/gemma-3-12b-it-GGUF",
        file: "mmproj-model-f16.gguf",
        size_bytes: 854_200_224,
        licence: "Gemma Terms of Use",
        notes: "Vision projector for Gemma 3 12B.",
        embed_prefixes: None,
        pairs_with: None,
        min_relevance: None,
    },
    CatalogEntry {
        id: "nomic-embed-text-v1.5",
        role: ModelRole::Embed,
        repo: "nomic-ai/nomic-embed-text-v1.5-GGUF",
        file: "nomic-embed-text-v1.5.Q8_0.gguf",
        size_bytes: 146_146_432,
        licence: "Apache-2.0",
        notes: "Default embedding model, 768 dimensions, 8k context. Uses search_document and search_query prefixes.",
        embed_prefixes: Some(("search_document: ", "search_query: ")),
        pairs_with: None,
        min_relevance: Some(0.65),
    },
];

pub fn catalog() -> &'static [CatalogEntry] {
    CATALOG
}

pub fn find(id: &str) -> Option<&'static CatalogEntry> {
    CATALOG.iter().find(|e| e.id == id)
}

/// Match a repo and file to a catalogue entry (for prefixes and thresholds
/// after a pull).
pub fn find_by_file(repo: &str, file: &str) -> Option<&'static CatalogEntry> {
    CATALOG
        .iter()
        .find(|e| e.repo.eq_ignore_ascii_case(repo) && e.file == file)
}

/// Embedding task prefixes for a model id or file, by catalogue or by name.
pub fn embed_prefixes_for(id_or_file: &str) -> Option<(&'static str, &'static str)> {
    if let Some(e) = CATALOG
        .iter()
        .find(|e| e.id == id_or_file || e.file == id_or_file)
    {
        return e.embed_prefixes;
    }
    let lower = id_or_file.to_ascii_lowercase();
    if lower.contains("nomic-embed") {
        Some(("search_document: ", "search_query: "))
    } else {
        None
    }
}

/// Case-insensitive search over id, repo, file and notes.
pub fn search(q: &str) -> Vec<&'static CatalogEntry> {
    let q = q.trim().to_ascii_lowercase();
    CATALOG
        .iter()
        .filter(|e| {
            q.is_empty()
                || e.id.contains(&q)
                || e.repo.to_ascii_lowercase().contains(&q)
                || e.file.to_ascii_lowercase().contains(&q)
                || e.notes.to_ascii_lowercase().contains(&q)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalogue_ids_are_unique_and_pairs_resolve() {
        let mut ids: Vec<&str> = CATALOG.iter().map(|e| e.id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), CATALOG.len());
        for e in CATALOG {
            if let Some(p) = e.pairs_with {
                assert_eq!(
                    find(p).unwrap().role,
                    ModelRole::Vision,
                    "{} pairs with a non-vision entry",
                    e.id
                );
            }
        }
    }

    #[test]
    fn nomic_prefixes_by_name() {
        assert_eq!(
            embed_prefixes_for("nomic-embed-text-v1.5").unwrap().1,
            "search_query: "
        );
        assert_eq!(embed_prefixes_for("some-other-embed.gguf"), None);
        assert_eq!(search("gemma").len(), 2);
    }
}
