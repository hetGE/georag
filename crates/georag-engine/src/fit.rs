//! Memory fit and the context ladder (handoff 5.4).
//!
//! needed(n_ctx) = weights + KV(n_ctx) + 1 GiB overhead, compared with the
//! free memory the backend reported before loading.

use serde::Serialize;

use crate::gguf::GgufInfo;
use crate::types::FitVerdict;

/// Fixed overhead for compute buffers, the embedding model's activations and
/// the backend's own allocations.
pub const OVERHEAD_BYTES: u64 = 1 << 30;

/// The documented rungs: 32k, 128k, 256k.
pub const LADDER: [u32; 3] = [32_768, 131_072, 262_144];

/// Smaller rungs tried only when 32k does not fit, so a small machine still
/// answers questions instead of refusing.
pub const FALLBACK_RUNGS: [u32; 3] = [16_384, 8_192, 4_096];

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct FitInput {
    pub free_bytes: u64,
    /// Bytes already committed by other resident models (the embedder).
    pub reserved_bytes: u64,
    /// User cap from settings (`engine.max_ctx`); 0 means none.
    pub max_ctx: u32,
}

pub fn needed_bytes(info: &GgufInfo, n_ctx: u32, kv_cache_type: &str) -> u64 {
    info.weight_bytes + info.kv_bytes_per_token(kv_cache_type) * u64::from(n_ctx) + OVERHEAD_BYTES
}

pub fn fits(info: &GgufInfo, n_ctx: u32, kv_cache_type: &str, input: FitInput) -> FitVerdict {
    let needed = needed_bytes(info, n_ctx, kv_cache_type);
    let free = input.free_bytes.saturating_sub(input.reserved_bytes);
    if needed <= free {
        FitVerdict::Fits {
            needed_bytes: needed,
            free_bytes: free,
        }
    } else {
        FitVerdict::DoesNotFit {
            needed_bytes: needed,
            free_bytes: free,
        }
    }
}

/// Every rung that fits, ascending, capped at `n_ctx_train` and `max_ctx`.
pub fn ladder_that_fits(info: &GgufInfo, kv_cache_type: &str, input: FitInput) -> Vec<u32> {
    let cap = |n: u32| {
        let mut c = n;
        if info.n_ctx_train > 0 {
            c = c.min(info.n_ctx_train);
        }
        if input.max_ctx > 0 {
            c = c.min(input.max_ctx);
        }
        c
    };
    let mut rungs: Vec<u32> = LADDER.iter().map(|r| cap(*r)).collect();
    rungs.sort_unstable();
    rungs.dedup();
    let mut fitting: Vec<u32> = rungs
        .into_iter()
        .filter(|n| {
            matches!(
                fits(info, *n, kv_cache_type, input),
                FitVerdict::Fits { .. }
            )
        })
        .collect();
    if fitting.is_empty() {
        for r in FALLBACK_RUNGS {
            let n = cap(r);
            if matches!(fits(info, n, kv_cache_type, input), FitVerdict::Fits { .. }) {
                fitting.push(n);
                break;
            }
        }
    }
    fitting
}

/// The largest rung that fits, if any.
pub fn max_ctx_that_fits(info: &GgufInfo, kv_cache_type: &str, input: FitInput) -> Option<u32> {
    ladder_that_fits(info, kv_cache_type, input)
        .into_iter()
        .max()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model(weight_gib: u64, n_ctx_train: u32) -> GgufInfo {
        GgufInfo {
            architecture: "qwen3".into(),
            name: None,
            n_ctx_train,
            n_layer: 36,
            n_head: 32,
            n_head_kv: 8,
            head_dim: 128,
            embed_dim: 4096,
            weight_bytes: weight_gib << 30,
            n_tensors: 1,
        }
    }

    #[test]
    fn kv_bytes_follow_cache_type() {
        let m = model(5, 262_144);
        // 2 * 8 heads * 128 dims * 36 layers = 73 728 values per token.
        assert_eq!(m.kv_bytes_per_token("f16"), 73_728 * 2);
        assert!(m.kv_bytes_per_token("q8_0") < m.kv_bytes_per_token("f16"));
        assert!(m.kv_bytes_per_token("q4_0") < m.kv_bytes_per_token("q8_0"));
    }

    #[test]
    fn ladder_on_a_16gb_mac() {
        let m = model(5, 262_144);
        let input = FitInput {
            free_bytes: 12 << 30,
            reserved_bytes: 0,
            max_ctx: 0,
        };
        // q8_0 KV: 73 728 * 8.5 / 8 = 78 336 bytes per token: 32k = 2.4 GiB fits,
        // 128k = 9.6 GiB does not beside 5 GiB of weights and 1 GiB overhead.
        assert_eq!(ladder_that_fits(&m, "q8_0", input), vec![32_768]);
        assert_eq!(max_ctx_that_fits(&m, "q8_0", input), Some(32_768));
    }

    #[test]
    fn caps_and_fallbacks() {
        let m = model(5, 40_960);
        let input = FitInput {
            free_bytes: 64 << 30,
            reserved_bytes: 0,
            max_ctx: 0,
        };
        assert_eq!(ladder_that_fits(&m, "q8_0", input), vec![32_768, 40_960]);
        let capped = FitInput {
            max_ctx: 16_384,
            ..input
        };
        assert_eq!(ladder_that_fits(&m, "q8_0", capped), vec![16_384]);
        let tiny = FitInput {
            free_bytes: 7 << 30,
            reserved_bytes: 0,
            max_ctx: 0,
        };
        assert_eq!(ladder_that_fits(&m, "q8_0", tiny), vec![8_192]);
        let none = FitInput {
            free_bytes: 5 << 30,
            reserved_bytes: 0,
            max_ctx: 0,
        };
        assert_eq!(max_ctx_that_fits(&m, "q8_0", none), None);
    }
}
