//! Starting the resident engine from the registry and settings (handoff 5.1):
//! chat and embedding models load at launch and stay until quit; the context
//! comes from the ladder unless manual mode asks for a tier.

use std::sync::Arc;

use georag_engine::fit::{self, FitInput};
use georag_engine::{BackendPreference, GgufInfo, InferenceEngine, LlamaCppEngine, LoadSpec};
use serde::Serialize;

use crate::config::Settings;
use crate::error::{CoreError, Result};
use crate::home::Home;
use crate::models::{ModelRole, Registry};

/// What `plan_load` decided, for doctor and the status bar.
#[derive(Debug, Clone, Serialize)]
pub struct LoadPlan {
    pub chat_id: Option<String>,
    pub embed_id: Option<String>,
    pub chat_info: Option<GgufInfo>,
    pub ladder: Vec<u32>,
    pub n_ctx: u32,
    pub kv_cache_type: String,
    pub free_bytes: u64,
}

/// Choose the models and context size without loading anything.
pub fn plan_load(
    home: &Home,
    settings: &Settings,
    registry: &Registry,
    free_bytes: u64,
    want_n_ctx: Option<u32>,
) -> Result<LoadPlan> {
    let chat = registry.in_use(ModelRole::Chat);
    let embed = registry.in_use(ModelRole::Embed);
    let chat_info = match chat {
        Some(m) => {
            let path = m.resolve_path(home);
            if !path.is_file() {
                return Err(CoreError::Invalid(format!(
                    "the chat model {} is missing at {}: pull it again or pick another with `georag models use chat ID`",
                    m.id,
                    path.display()
                )));
            }
            Some(GgufInfo::read(&path)?)
        }
        None => None,
    };
    let reserved = embed.and_then(|m| m.size_bytes).unwrap_or(0);
    let input = FitInput {
        free_bytes,
        reserved_bytes: reserved,
        max_ctx: settings.engine.max_ctx,
    };
    let kv = settings.engine.kv_cache_type.clone();
    let ladder = chat_info
        .as_ref()
        .map(|i| fit::ladder_that_fits(i, &kv, input))
        .unwrap_or_default();
    let n_ctx = match (want_n_ctx, ladder.iter().max()) {
        (Some(want), Some(max)) if want <= *max => want,
        (Some(want), Some(max)) => {
            return Err(CoreError::Invalid(format!(
                "a context of {want} tokens does not fit: the largest that fits is {max} (free memory {}, model {})",
                crate::doctor::fmt_gib(free_bytes),
                chat_info
                    .as_ref()
                    .map(|i| crate::doctor::fmt_gib(i.weight_bytes))
                    .unwrap_or_default()
            )));
        }
        (_, Some(max)) => *max,
        (_, None) if chat_info.is_some() => {
            let info = chat_info.as_ref().expect("checked");
            return Err(CoreError::Invalid(format!(
                "the chat model needs {} for weights plus overhead but only {} is free: pick a smaller model",
                crate::doctor::fmt_gib(fit::needed_bytes(info, 4096, &kv)),
                crate::doctor::fmt_gib(free_bytes.saturating_sub(reserved))
            )));
        }
        (_, None) => 0,
    };
    Ok(LoadPlan {
        chat_id: chat.map(|m| m.id.clone()),
        embed_id: embed.map(|m| m.id.clone()),
        chat_info,
        ladder,
        n_ctx,
        kv_cache_type: kv,
        free_bytes,
    })
}

/// A started engine with its plan.
pub struct Resident {
    pub engine: Arc<dyn InferenceEngine>,
    pub plan: LoadPlan,
}

/// Start the engine thread, plan the load and make the in-use models
/// resident. `want_n_ctx` is a manual tier's context; `None` takes the
/// largest rung that fits.
pub fn start(
    home: &Home,
    settings: &Settings,
    registry: &Registry,
    want_n_ctx: Option<u32>,
) -> Result<Resident> {
    let pref: BackendPreference = settings.engine.backend.parse()?;
    let engine = LlamaCppEngine::start(pref, settings.engine.max_ctx)?;
    let free = engine.backend().device.memory_free;
    let plan = plan_load(home, settings, registry, free, want_n_ctx)?;
    if plan.chat_id.is_none() && plan.embed_id.is_none() {
        return Err(CoreError::Invalid(
            "no models in use: run `georag models pull nomic-embed-text-v1.5` and `georag models pull qwen3.5-9b-q4_k_m`".into(),
        ));
    }
    let spec = LoadSpec {
        chat: registry
            .in_use(ModelRole::Chat)
            .map(|m| m.resolve_path(home)),
        mmproj: None,
        embed: registry
            .in_use(ModelRole::Embed)
            .map(|m| m.resolve_path(home)),
        n_ctx: plan.n_ctx.max(512),
        gpu_layers: settings.engine.gpu_layers,
        kv_cache_type: plan.kv_cache_type.clone(),
    };
    tracing::info!(n_ctx = spec.n_ctx, chat = ?plan.chat_id, embed = ?plan.embed_id, "loading models");
    engine.load(spec)?;
    Ok(Resident { engine, plan })
}
