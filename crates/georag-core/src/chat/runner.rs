//! Runs a chat turn: retrieval, planning, one or many passes, synthesis,
//! persistence. Events stream to the caller; controls arrive from the UI.

use std::sync::{Arc, Mutex};

use georag_engine::{CancelToken, ChatMessage, FinishReason, GenRequest, InferenceEngine, Lane};
use georag_store::{NewMessage, NewPass, Source, Store};
use serde::Serialize;

use super::budget::{self, Turn};
use super::planner::{Plan, PlanConfig, plan_auto};
use super::prompts;
use super::{ChatEvent, Mode, PassStatus, RunControl, Tier};
use crate::config::Settings;
use crate::error::{CoreError, Result};
use crate::retrieval::{Candidate, hybrid_retrieve};

/// What a run needs from the app.
pub struct ChatContext<'a> {
    pub store: &'a Store,
    pub engine: &'a dyn InferenceEngine,
    pub settings: &'a Settings,
    /// Query-time prefix of the embedding model (`search_query: ` for nomic).
    pub query_prefix: String,
    /// Largest context that fits, for auto mode.
    pub max_ctx: u32,
}

#[derive(Debug, Clone)]
pub struct ChatRequest {
    pub conversation_id: Option<i64>,
    pub message: String,
    pub tags: Vec<String>,
    pub mode: Option<Mode>,
    pub tier: Option<Tier>,
    pub min_relevance: Option<f32>,
    pub max_passes: Option<usize>,
    pub synthesis: Option<bool>,
}

/// Shared between the runner and the UI. Skip and Stop cancel the current
/// generation; Accept lets it finish.
#[derive(Clone, Default)]
pub struct Controls {
    inner: Arc<Mutex<ControlsInner>>,
}

#[derive(Default)]
struct ControlsInner {
    pending: Option<RunControl>,
    current: Option<CancelToken>,
}

impl Controls {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn request(&self, control: RunControl) {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        inner.pending = Some(control);
        if matches!(control, RunControl::SkipPass | RunControl::StopAll)
            && let Some(c) = &inner.current
        {
            c.cancel();
        }
    }

    fn begin_pass(&self) -> CancelToken {
        let token = CancelToken::new();
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        inner.current = Some(token.clone());
        token
    }

    fn take(&self) -> Option<RunControl> {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        inner.current = None;
        inner.pending.take()
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct PassRecord {
    pub k: u16,
    pub status: PassStatus,
    pub chunk_ids: Vec<i64>,
    pub sources_from: u32,
    pub sources_to: u32,
    pub answer: String,
    pub cut_off: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct RunOutcome {
    pub conversation_id: i64,
    pub message_id: i64,
    pub mode: Mode,
    pub plan: super::PlanSummary,
    pub passes: Vec<PassRecord>,
    pub final_answer: String,
    pub sources: Vec<Source>,
    pub cut_off: bool,
    pub stopped: bool,
}

struct Tagged<'a> {
    tag: u32,
    cand: &'a Candidate,
}

pub fn run(
    ctx: &ChatContext<'_>,
    req: &ChatRequest,
    controls: &Controls,
    on_event: &mut dyn FnMut(ChatEvent),
) -> Result<RunOutcome> {
    let settings = ctx.settings;
    let mode = req.mode.unwrap_or(match settings.chat.mode.as_str() {
        "manual" => Mode::Manual,
        _ => Mode::Auto,
    });
    let tier = req
        .tier
        .unwrap_or(settings.chat.manual_tier.parse().unwrap_or(Tier::Optimal));
    let min_relevance = req.min_relevance.unwrap_or(settings.auto.min_relevance);
    let max_passes = req.max_passes.unwrap_or(settings.auto.max_passes as usize);
    let synthesis = req.synthesis.unwrap_or(settings.auto.synthesis);

    // Conversation and history.
    let conversation = match req.conversation_id {
        Some(id) => ctx
            .store
            .conversation_get(id)?
            .ok_or_else(|| CoreError::Invalid(format!("no conversation {id}")))?,
        None => {
            let title: String = req.message.chars().take(60).collect();
            ctx.store
                .conversation_create(Some(title.trim()), &req.tags)?
        }
    };
    let tags = if req.tags.is_empty() {
        conversation.tags.clone()
    } else {
        req.tags.clone()
    };
    let history = ctx.store.messages(conversation.id)?;
    let mut turns = Vec::with_capacity(history.len());
    for m in &history {
        let content = prompts::strip_think(&m.content);
        let tokens = budget::count(ctx.engine, &content)?;
        turns.push(Turn {
            role: m.role.clone(),
            content,
            tokens,
        });
    }

    // Context size: auto takes the largest rung; manual takes the tier's.
    let n_ctx = match mode {
        Mode::Auto => ctx.max_ctx,
        Mode::Manual => {
            let want = tier.n_ctx();
            if want > ctx.max_ctx {
                return Err(CoreError::Invalid(format!(
                    "depth {tier} needs a context of {want} tokens but the largest that fits is {}: use a smaller tier or auto mode",
                    ctx.max_ctx
                )));
            }
            want
        }
    };
    let current = ctx.engine.stats().n_ctx;
    if current != n_ctx {
        ctx.engine.resize_ctx(n_ctx)?;
    }

    let question_tokens = budget::count(ctx.engine, &req.message)?;
    let system_tokens = budget::count(ctx.engine, &prompts::pass_system(1, 1, 1))? + 64;
    let prelim = budget::budget(n_ctx, system_tokens, 0, question_tokens);
    let kept_turns = budget::keep_history(
        &turns,
        settings.chat.history_turns as usize,
        prelim.prompt_budget,
    );
    let history_tokens: u32 = kept_turns.iter().map(|t| t.tokens).sum();
    let b = budget::budget(n_ctx, system_tokens, history_tokens, question_tokens);

    // Retrieval.
    let cands = if ctx.store.embed_info()?.is_some() && ctx.store.chunk_count()? > 0 {
        hybrid_retrieve(
            ctx.store,
            ctx.engine,
            &req.message,
            &tags,
            settings.auto.candidate_k as usize,
            &ctx.query_prefix,
        )?
    } else {
        Vec::new()
    };

    let plan = match mode {
        Mode::Auto => plan_auto(
            &cands,
            PlanConfig {
                min_relevance,
                max_passes,
                min_candidates: settings.auto.min_candidates as usize,
            },
            b.chunk_budget,
        ),
        Mode::Manual => manual_plan(&cands, tier, &tags, b.chunk_budget),
    };
    let summary = plan.summary(b.chunk_budget, min_relevance);
    on_event(ChatEvent::Plan {
        summary: summary.clone(),
    });

    // Global citation numbering across the run.
    let order = plan.ordered_chunk_ids();
    let tagged: Vec<Tagged<'_>> = order
        .iter()
        .enumerate()
        .filter_map(|(i, id)| {
            cands.iter().find(|c| c.chunk_id == *id).map(|cand| Tagged {
                tag: i as u32 + 1,
                cand,
            })
        })
        .collect();
    let find_tag = |id: i64| tagged.iter().find(|t| t.cand.chunk_id == id);

    let user_message = NewMessage {
        conversation_id: conversation.id,
        role: "user",
        content: &req.message,
        sources: &[],
        mode: None,
        cut_off: false,
    };
    ctx.store.message_add(&user_message)?;

    let history_msgs: Vec<ChatMessage> = kept_turns
        .iter()
        .map(|t| ChatMessage::new(&t.role, t.content.clone()))
        .collect();
    let thinking = settings.chat.thinking;
    let max_tokens = b.reply_reserve;

    let mut passes: Vec<PassRecord> = Vec::new();
    let mut final_answer = String::new();
    let mut cut_off = false;
    let mut stopped = false;

    match &plan {
        Plan::Single { chunk_ids, .. } => {
            let block: Vec<(u32, &Candidate)> = chunk_ids
                .iter()
                .filter_map(|id| find_tag(*id))
                .map(|t| (t.tag, t.cand))
                .collect();
            let system = if block.is_empty() {
                format!(
                    "{}\n\nNo sources matched this question in the library; answer from the conversation and say that no sources were found.",
                    prompts::SYSTEM_BASE
                )
            } else {
                format!(
                    "{}\n\nSources:\n\n{}",
                    prompts::SYSTEM_BASE,
                    prompts::sources_block(&block)
                )
            };
            let cancel = controls.begin_pass();
            let (text, finish) = generate(
                ctx.engine,
                &system,
                &history_msgs,
                &req.message,
                thinking,
                max_tokens,
                cancel,
                &mut |t| {
                    on_event(ChatEvent::Token {
                        k: None,
                        text: t.to_string(),
                    })
                },
            )?;
            let status = match controls.take() {
                Some(RunControl::StopAll) if finish == FinishReason::Cancelled => {
                    stopped = true;
                    PassStatus::Stopped
                }
                _ => PassStatus::Done,
            };
            cut_off = finish == FinishReason::Length;
            final_answer = text.clone();
            if cut_off {
                final_answer.push_str("\n\n[The answer was cut off at the context limit.]");
            }
            passes.push(PassRecord {
                k: 1,
                status,
                chunk_ids: chunk_ids.clone(),
                sources_from: 1,
                sources_to: chunk_ids.len() as u32,
                answer: text,
                cut_off,
            });
        }
        Plan::Multi {
            passes: planned, ..
        } => {
            let n_sources = order.len() as u32;
            let mut accepted = false;
            for (i, pass) in planned.iter().enumerate() {
                let k = i as u16 + 1;
                let block: Vec<(u32, &Candidate)> = pass
                    .chunk_ids
                    .iter()
                    .filter_map(|id| find_tag(*id))
                    .map(|t| (t.tag, t.cand))
                    .collect();
                let from = block.first().map(|b| b.0).unwrap_or(0);
                let to = block.last().map(|b| b.0).unwrap_or(0);
                on_event(ChatEvent::PassStart {
                    k,
                    sources_from: from,
                    sources_to: to,
                });
                let system = format!(
                    "{}\n\nSources:\n\n{}",
                    prompts::pass_system(from, to, n_sources),
                    prompts::sources_block(&block)
                );
                let cancel = controls.begin_pass();
                let (text, finish) = generate(
                    ctx.engine,
                    &system,
                    &history_msgs,
                    &req.message,
                    thinking,
                    max_tokens,
                    cancel,
                    &mut |t| {
                        on_event(ChatEvent::Token {
                            k: Some(k),
                            text: t.to_string(),
                        })
                    },
                )?;
                let pass_cut = finish == FinishReason::Length;
                let control = controls.take();
                let status = match (control, finish) {
                    (Some(RunControl::SkipPass), _) => PassStatus::Skipped,
                    (Some(RunControl::StopAll), _) => PassStatus::Stopped,
                    (Some(RunControl::AcceptPass), _) => PassStatus::Accepted,
                    (None, FinishReason::Cancelled) => PassStatus::Stopped,
                    _ => PassStatus::Done,
                };
                on_event(ChatEvent::PassEnd {
                    k,
                    status,
                    cut_off: pass_cut,
                });
                passes.push(PassRecord {
                    k,
                    status,
                    chunk_ids: pass.chunk_ids.clone(),
                    sources_from: from,
                    sources_to: to,
                    answer: text.clone(),
                    cut_off: pass_cut,
                });
                match status {
                    PassStatus::Accepted => {
                        accepted = true;
                        final_answer = text;
                        cut_off = pass_cut;
                        break;
                    }
                    PassStatus::Stopped => {
                        stopped = true;
                        break;
                    }
                    _ => {}
                }
            }
            if !accepted && !stopped {
                let done: Vec<&PassRecord> = passes
                    .iter()
                    .filter(|p| p.status == PassStatus::Done)
                    .collect();
                if done.is_empty() {
                    final_answer =
                        "Every pass was skipped, so there is no answer to combine.".to_string();
                } else if synthesis {
                    on_event(ChatEvent::FinalStart);
                    let cancel = controls.begin_pass();
                    let (text, finish) = synthesise(
                        ctx.engine,
                        &req.message,
                        &done,
                        b.prompt_budget,
                        max_tokens,
                        thinking,
                        cancel,
                        &mut |t| {
                            on_event(ChatEvent::Token {
                                k: Some(0),
                                text: t.to_string(),
                            })
                        },
                    )?;
                    cut_off = finish == FinishReason::Length;
                    if finish == FinishReason::Cancelled {
                        stopped = true;
                    }
                    final_answer = text.clone();
                    passes.push(PassRecord {
                        k: 0,
                        status: if stopped {
                            PassStatus::Stopped
                        } else {
                            PassStatus::Done
                        },
                        chunk_ids: Vec::new(),
                        sources_from: 1,
                        sources_to: n_sources,
                        answer: text,
                        cut_off,
                    });
                    let _ = controls.take();
                } else {
                    final_answer = done
                        .iter()
                        .map(|p| {
                            format!(
                                "Pass {} (sources {} to {})\n\n{}",
                                p.k, p.sources_from, p.sources_to, p.answer
                            )
                        })
                        .collect::<Vec<_>>()
                        .join("\n\n");
                }
            } else if stopped && final_answer.is_empty() {
                final_answer = format!(
                    "Stopped after {} of {} passes. Combine what is done to get an answer.",
                    passes.len(),
                    planned.len()
                );
            }
        }
    }

    // Sources: cited tags, else the chunks used.
    let cited = prompts::cited_tags(&final_answer);
    let mut sources: Vec<Source> = Vec::new();
    let chosen: Vec<&Tagged<'_>> = if cited.is_empty() {
        tagged.iter().take(16).collect()
    } else {
        cited
            .iter()
            .filter_map(|t| tagged.iter().find(|x| x.tag == *t))
            .collect()
    };
    for t in chosen {
        sources.push(Source {
            file_id: t.cand.file_id,
            rel_path: t.cand.rel_path.clone(),
            locator: t.cand.locator.clone(),
            chunk_id: t.cand.chunk_id,
            score: t.cand.score,
            tag: t.tag,
        });
    }

    let mode_str = match mode {
        Mode::Auto => "auto",
        Mode::Manual => "manual",
    };
    let message = ctx.store.message_add(&NewMessage {
        conversation_id: conversation.id,
        role: "assistant",
        content: &final_answer,
        sources: &sources,
        mode: Some(mode_str),
        cut_off,
    })?;
    let rows: Vec<NewPass<'_>> = passes
        .iter()
        .map(|p| NewPass {
            idx: i64::from(p.k),
            status: p.status.as_str(),
            chunk_ids: &p.chunk_ids,
            answer: Some(&p.answer),
            started_at: None,
            finished_at: None,
        })
        .collect();
    ctx.store.passes_add(message.id, &rows)?;
    on_event(ChatEvent::Done {
        message_id: message.id,
        sources: sources.clone(),
        cut_off,
    });

    Ok(RunOutcome {
        conversation_id: conversation.id,
        message_id: message.id,
        mode,
        plan: summary,
        passes,
        final_answer,
        sources,
        cut_off,
        stopped,
    })
}

/// Manual tiers (7.6): top k per tag by RRF, capped at max chunks, one pass.
fn manual_plan(cands: &[Candidate], tier: Tier, tags: &[String], budget: u32) -> Plan {
    let mut by_rrf: Vec<&Candidate> = cands.iter().collect();
    by_rrf.sort_by(|a, b| {
        b.rrf
            .partial_cmp(&a.rrf)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let per_tag = tier.top_k_per_tag() * tags.len().max(1);
    let mut ids = Vec::new();
    let mut tokens = 0u32;
    for c in by_rrf.into_iter().take(per_tag.min(tier.max_chunks())) {
        if tokens + c.tokens > budget {
            break;
        }
        tokens += c.tokens;
        ids.push(c.chunk_id);
    }
    Plan::Single {
        chunk_ids: ids,
        tokens,
        fallback: false,
    }
}

#[allow(clippy::too_many_arguments)]
fn generate(
    engine: &dyn InferenceEngine,
    system: &str,
    history: &[ChatMessage],
    question: &str,
    thinking: bool,
    max_tokens: u32,
    cancel: CancelToken,
    on_token: &mut dyn FnMut(&str),
) -> Result<(String, FinishReason)> {
    let mut messages = Vec::with_capacity(history.len() + 2);
    messages.push(ChatMessage::new("system", system));
    messages.extend(history.iter().cloned());
    messages.push(ChatMessage::new("user", question));
    let prompt = engine.chat_prompt(&messages, thinking)?;
    let (tx, rx) = std::sync::mpsc::channel::<String>();
    let sink: georag_engine::TokenSink = Box::new(move |t| {
        let _ = tx.send(t.to_string());
    });
    // Generation runs on the engine thread; forward tokens as they arrive.
    let result = std::thread::scope(|s| {
        let handle = s.spawn(|| {
            engine.generate(
                GenRequest {
                    prompt,
                    max_tokens,
                    temperature: 0.2,
                    seed: 42,
                },
                Lane::Interactive,
                sink,
                cancel,
            )
        });
        for t in rx {
            on_token(&t);
        }
        handle
            .join()
            .map_err(|_| CoreError::Invalid("generation thread panicked".into()))
    })??;
    let text = prompts::strip_think(&result.text).trim().to_string();
    Ok((text, result.finish_reason))
}

#[allow(clippy::too_many_arguments)]
fn synthesise(
    engine: &dyn InferenceEngine,
    question: &str,
    done: &[&PassRecord],
    prompt_budget: u32,
    max_tokens: u32,
    thinking: bool,
    cancel: CancelToken,
    on_token: &mut dyn FnMut(&str),
) -> Result<(String, FinishReason)> {
    let mut parts: Vec<(String, u32)> = Vec::new();
    for p in done {
        let text = format!(
            "Pass {} (sources {} to {})\n\n{}",
            p.k, p.sources_from, p.sources_to, p.answer
        );
        let tokens = budget::count(engine, &text)?;
        parts.push((text, tokens));
    }
    let overhead =
        budget::count(engine, prompts::FINAL_SYSTEM)? + budget::count(engine, question)? + 128;
    let room = prompt_budget.saturating_sub(overhead).max(256);
    // Combine in groups when the partial answers do not fit at once.
    let mut level = 0;
    loop {
        let total: u32 = parts.iter().map(|p| p.1).sum();
        if parts.len() <= 1 || total <= room {
            let input = parts
                .iter()
                .map(|p| p.0.as_str())
                .collect::<Vec<_>>()
                .join("\n\n");
            let user = format!("Question: {question}\n\nPartial answers:\n\n{input}");
            return generate(
                engine,
                prompts::FINAL_SYSTEM,
                &[],
                &user,
                thinking,
                max_tokens,
                cancel.clone(),
                on_token,
            );
        }
        level += 1;
        let mut groups: Vec<Vec<(String, u32)>> = vec![Vec::new()];
        let mut used = 0u32;
        for p in parts.drain(..) {
            if used + p.1 > room && !groups.last().expect("one group").is_empty() {
                groups.push(Vec::new());
                used = 0;
            }
            used += p.1;
            groups.last_mut().expect("one group").push(p);
        }
        let mut next = Vec::new();
        for (gi, g) in groups.into_iter().enumerate() {
            let input = g
                .iter()
                .map(|p| p.0.as_str())
                .collect::<Vec<_>>()
                .join("\n\n");
            let user = format!("Question: {question}\n\nPartial answers:\n\n{input}");
            let (text, finish) = generate(
                engine,
                prompts::FINAL_SYSTEM,
                &[],
                &user,
                thinking,
                max_tokens,
                cancel.clone(),
                &mut |_| {},
            )?;
            if finish == FinishReason::Cancelled {
                return Ok((text, finish));
            }
            let labelled = format!("Group {level}.{} \n\n{text}", gi + 1);
            let tokens = budget::count(engine, &labelled)?;
            next.push((labelled, tokens));
        }
        parts = next;
        if level > 4 {
            let input = parts
                .iter()
                .map(|p| p.0.as_str())
                .collect::<Vec<_>>()
                .join("\n\n");
            return Ok((input, FinishReason::Stop));
        }
    }
}

/// Run the final pass over the done passes of a stopped message, replacing
/// its content ("combine what is done").
pub fn combine_done(
    ctx: &ChatContext<'_>,
    message_id: i64,
    controls: &Controls,
    on_event: &mut dyn FnMut(ChatEvent),
) -> Result<String> {
    let rows = ctx.store.passes(message_id)?;
    let done: Vec<PassRecord> = rows
        .iter()
        .filter(|p| p.idx > 0 && p.status == "done")
        .map(|p| PassRecord {
            k: p.idx as u16,
            status: PassStatus::Done,
            chunk_ids: p.chunk_ids.clone(),
            sources_from: 0,
            sources_to: 0,
            answer: p.answer.clone().unwrap_or_default(),
            cut_off: false,
        })
        .collect();
    if done.is_empty() {
        return Err(CoreError::Invalid("no completed passes to combine".into()));
    }
    let question = ctx
        .store
        .messages(ctx.store.messages_conversation(message_id)?)?
        .iter()
        .rev()
        .find(|m| m.role == "user" && m.id < message_id)
        .map(|m| m.content.clone())
        .unwrap_or_default();
    let n_ctx = ctx.engine.stats().n_ctx.max(1024);
    let b = budget::budget(n_ctx, 0, 0, 0);
    on_event(ChatEvent::FinalStart);
    let cancel = controls.begin_pass();
    let refs: Vec<&PassRecord> = done.iter().collect();
    let (text, finish) = synthesise(
        ctx.engine,
        &question,
        &refs,
        b.prompt_budget,
        b.reply_reserve,
        ctx.settings.chat.thinking,
        cancel,
        &mut |t| {
            on_event(ChatEvent::Token {
                k: Some(0),
                text: t.to_string(),
            })
        },
    )?;
    let _ = controls.take();
    ctx.store
        .message_set_content(message_id, &text, finish == FinishReason::Length)?;
    ctx.store.passes_add(
        message_id,
        &[NewPass {
            idx: 0,
            status: "done",
            chunk_ids: &[],
            answer: Some(&text),
            started_at: None,
            finished_at: None,
        }],
    )?;
    on_event(ChatEvent::Done {
        message_id,
        sources: Vec::new(),
        cut_off: finish == FinishReason::Length,
    });
    Ok(text)
}
