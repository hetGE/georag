//! End-to-end auto depth over a seeded index with real models (handoff P1
//! acceptance). Needs a home folder with a chat and an embedding model in
//! use; point `GEORAG_TEST_HOME` at it and run with `--ignored`:
//!
//!   GEORAG_TEST_HOME=.dev-home cargo test -p georag-core --test auto_depth -- --ignored

use georag_core::Home;
use georag_core::app::App;
use georag_core::chat::{ChatEvent, ChatRequest, Controls, PassStatus, RunControl};

fn test_home() -> Option<Home> {
    let path = std::env::var("GEORAG_TEST_HOME").ok()?;
    let home = Home::resolve(Some(std::path::Path::new(&path))).ok()?;
    home.is_initialised().then_some(home)
}

#[test]
#[ignore = "needs real models; set GEORAG_TEST_HOME"]
fn forty_thousand_tokens_run_in_passes_then_combine() {
    let Some(home) = test_home() else {
        eprintln!("GEORAG_TEST_HOME not set; skipping");
        return;
    };
    let app = App::open(&home, None).expect("models load");
    let tokens = app.seed_demo(8, 5).expect("seed");
    assert!(tokens >= 40_000, "seeded {tokens} tokens");

    let ctx = app.chat_context();
    let controls = Controls::new();
    let mut events = Vec::new();
    let req = ChatRequest {
        conversation_id: None,
        message:
            "What pile capacity was adopted at the Riverbend site, and how long are the piles?"
                .into(),
        tags: Vec::new(),
        mode: None,
        tier: None,
        min_relevance: None,
        max_passes: None,
        synthesis: None,
    };
    let outcome =
        georag_core::chat::run(&ctx, &req, &controls, &mut |ev| events.push(ev)).expect("run");

    assert!(
        outcome.plan.passes >= 2,
        "expected several passes, got {}",
        outcome.plan.passes
    );
    let done: Vec<_> = outcome
        .passes
        .iter()
        .filter(|p| p.k > 0 && p.status == PassStatus::Done)
        .collect();
    assert_eq!(done.len(), outcome.plan.passes);
    assert!(
        outcome.passes.iter().any(|p| p.k == 0),
        "final pass recorded"
    );
    assert!(events.iter().any(|e| matches!(e, ChatEvent::FinalStart)));
    assert!(
        outcome.final_answer.contains("900")
            || outcome.final_answer.to_lowercase().contains("riverbend")
    );

    // Every pass is persisted on the message.
    let view =
        georag_core::conversations::show(&home, outcome.conversation_id).expect("conversation");
    let reply = view.messages.last().expect("assistant message");
    assert_eq!(reply.passes.len(), outcome.passes.len());

    // Controls: skip the first pass, accept the second.
    let controls = Controls::new();
    let mut seen_pass = 0u16;
    let outcome = georag_core::chat::run(
        &ctx,
        &ChatRequest {
            conversation_id: Some(outcome.conversation_id),
            message: "List the groundwater depths reported at every site.".into(),
            ..req.clone()
        },
        &controls,
        &mut |ev| {
            if let ChatEvent::PassStart { k, .. } = ev {
                seen_pass = k;
                if k == 1 {
                    controls.request(RunControl::SkipPass);
                }
            }
            if let ChatEvent::Token { k: Some(2), .. } = ev {
                controls.request(RunControl::AcceptPass);
            }
        },
    )
    .expect("run with controls");
    assert!(seen_pass >= 2);
    assert_eq!(outcome.passes[0].status, PassStatus::Skipped);
    assert_eq!(outcome.passes[1].status, PassStatus::Accepted);
    assert_eq!(outcome.passes.len(), 2, "run stops at the accepted pass");
    assert!(
        !outcome.passes.iter().any(|p| p.k == 0),
        "no synthesis after accept"
    );
}
