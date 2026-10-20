//! GGUF header facts needed for the memory fit, read without loading weights.

use std::path::Path;

use llama_cpp_2::gguf::GgufContext;
use serde::Serialize;

use crate::error::EngineError;

/// What the header says. `weight_bytes` is the file size, which is within a
/// percent of the tensor bytes for GGUF.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GgufInfo {
    pub architecture: String,
    pub name: Option<String>,
    pub n_ctx_train: u32,
    pub n_layer: u32,
    pub n_head: u32,
    pub n_head_kv: u32,
    pub head_dim: u32,
    pub embed_dim: u32,
    pub weight_bytes: u64,
    pub n_tensors: u64,
}

// GGUF value type codes from ggml/include/gguf.h (enum gguf_type).
const GGUF_TYPE_UINT32: u32 = 4;
const GGUF_TYPE_INT32: u32 = 5;
const GGUF_TYPE_STRING: u32 = 8;
const GGUF_TYPE_UINT64: u32 = 10;

fn read_uint(ctx: &GgufContext, key: &str) -> Option<u64> {
    let idx = ctx.find_key(key);
    if idx < 0 {
        return None;
    }
    match ctx.kv_type(idx) {
        GGUF_TYPE_UINT32 => Some(u64::from(ctx.val_u32(idx))),
        GGUF_TYPE_INT32 => u64::try_from(ctx.val_i32(idx)).ok(),
        GGUF_TYPE_UINT64 => Some(ctx.val_u64(idx)),
        _ => None,
    }
}

fn read_str(ctx: &GgufContext, key: &str) -> Option<String> {
    let idx = ctx.find_key(key);
    if idx < 0 {
        return None;
    }
    if ctx.kv_type(idx) != GGUF_TYPE_STRING {
        return None;
    }
    ctx.val_str(idx).map(str::to_string)
}

impl GgufInfo {
    pub fn read(path: &Path) -> Result<GgufInfo, EngineError> {
        let ctx = GgufContext::from_file(path).ok_or_else(|| EngineError::Gguf {
            path: path.to_path_buf(),
            reason: "not a readable GGUF file".into(),
        })?;
        let architecture =
            read_str(&ctx, "general.architecture").ok_or_else(|| EngineError::Gguf {
                path: path.to_path_buf(),
                reason: "general.architecture is missing".into(),
            })?;
        let a = architecture.as_str();
        let get = |suffix: &str| read_uint(&ctx, &format!("{a}.{suffix}"));
        let n_ctx_train = get("context_length").unwrap_or(0) as u32;
        let n_layer = get("block_count").unwrap_or(0) as u32;
        let embed_dim = get("embedding_length").unwrap_or(0) as u32;
        let n_head = get("attention.head_count").unwrap_or(0) as u32;
        let n_head_kv = get("attention.head_count_kv")
            .map(|v| v as u32)
            .unwrap_or(n_head);
        let head_dim = get("attention.key_length")
            .map(|v| v as u32)
            .unwrap_or(embed_dim.checked_div(n_head).unwrap_or(0));
        let weight_bytes = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
        Ok(GgufInfo {
            architecture,
            name: read_str(&ctx, "general.name"),
            n_ctx_train,
            n_layer,
            n_head,
            n_head_kv,
            head_dim,
            embed_dim,
            weight_bytes,
            n_tensors: ctx.n_tensors().max(0) as u64,
        })
    }

    /// Bytes of KV cache per token for a cache type (`f16`, `q8_0`, `q4_0`).
    pub fn kv_bytes_per_token(&self, kv_cache_type: &str) -> u64 {
        let bits: f64 = match kv_cache_type {
            "q4_0" => 4.5,
            "q8_0" => 8.5,
            "f32" => 32.0,
            _ => 16.0,
        };
        let per_layer = 2.0 * f64::from(self.n_head_kv) * f64::from(self.head_dim) * bits / 8.0;
        (per_layer * f64::from(self.n_layer)).ceil() as u64
    }
}
