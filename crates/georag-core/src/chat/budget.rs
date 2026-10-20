//! Context budget (handoff 7.2).

use georag_engine::{InferenceEngine, Role};

use crate::error::Result;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Budget {
    pub n_ctx: u32,
    pub reply_reserve: u32,
    pub prompt_budget: u32,
    pub chunk_budget: u32,
}

pub fn count(engine: &dyn InferenceEngine, text: &str) -> Result<u32> {
    Ok(engine.tokenize(Role::Chat, text)?.len() as u32)
}

/// Compute the budget for one prompt shape.
pub fn budget(n_ctx: u32, system_tokens: u32, history_tokens: u32, question_tokens: u32) -> Budget {
    let reply_reserve = n_ctx / 2;
    let prompt_budget = n_ctx - reply_reserve;
    let chunk_budget =
        prompt_budget.saturating_sub(system_tokens + history_tokens + question_tokens + 256);
    Budget {
        n_ctx,
        reply_reserve,
        prompt_budget,
        chunk_budget,
    }
}

/// One stored turn, already stripped of think blocks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Turn {
    pub role: String,
    pub content: String,
    pub tokens: u32,
}

/// Keep the newest turns while they fit `prompt_budget / 4`, oldest dropped
/// first. Input is oldest first; output is oldest first too.
pub fn keep_history(turns: &[Turn], history_turns: usize, prompt_budget: u32) -> Vec<Turn> {
    let cap = prompt_budget / 4;
    let mut kept: Vec<Turn> = Vec::new();
    let mut used = 0u32;
    for turn in turns.iter().rev().take(history_turns * 2) {
        if used + turn.tokens > cap {
            break;
        }
        used += turn.tokens;
        kept.push(turn.clone());
    }
    kept.reverse();
    kept
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn budget_arithmetic() {
        let b = budget(32_768, 200, 100, 50);
        assert_eq!(b.reply_reserve, 16_384);
        assert_eq!(b.prompt_budget, 16_384);
        assert_eq!(b.chunk_budget, 16_384 - 200 - 100 - 50 - 256);
    }

    #[test]
    fn history_keeps_newest_within_quarter() {
        let turns: Vec<Turn> = (0..8)
            .map(|i| Turn {
                role: if i % 2 == 0 {
                    "user".into()
                } else {
                    "assistant".into()
                },
                content: format!("t{i}"),
                tokens: 1000,
            })
            .collect();
        let kept = keep_history(&turns, 4, 16_384);
        assert_eq!(kept.len(), 4);
        assert_eq!(kept[0].content, "t4");
        assert_eq!(kept[3].content, "t7");
        let kept = keep_history(&turns, 1, 16_384);
        assert_eq!(kept.len(), 2);
    }
}
