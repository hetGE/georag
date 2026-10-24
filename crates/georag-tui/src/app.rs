//! The loop: terminal events from a reader thread, a 250 ms tick, `update`,
//! then `view`, coalescing whatever arrived since the last draw. Commands
//! from `update` are performed here; slow ones run on threads and report
//! back as messages.

use std::io::Write;
use std::sync::Arc;
use std::time::Duration;

use crossterm::event::{DisableMouseCapture, EnableMouseCapture, Event};
use crossterm::execute;
use georag_core::Home;
use georag_core::app::App;
use georag_core::chat::Controls;
use georag_core::models::{ModelRole, Registry, ops};
use tokio::sync::mpsc;

use crate::TuiError;
use crate::screens::chat;
use crate::state::{Cmd, EngineState, Msg, Overlay, State};
use crate::theme::Theme;
use crate::update::update;
use crate::view::draw;

pub const TICK: Duration = Duration::from_millis(250);

pub fn run(home: Home, state: State, theme: Theme) -> Result<(), TuiError> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(TuiError::Terminal)?;

    let mut terminal = ratatui::try_init().map_err(TuiError::Terminal)?;
    let mouse = state.mouse;
    if mouse {
        let _ = execute!(std::io::stdout(), EnableMouseCapture);
    }

    let result = runtime.block_on(event_loop(&mut terminal, home, state, theme));

    if mouse {
        let _ = execute!(std::io::stdout(), DisableMouseCapture);
    }
    ratatui::restore();
    let _ = std::io::stdout().flush();
    result
}

/// Reads crossterm events on a plain thread so the async loop never blocks.
fn spawn_reader(tx: mpsc::Sender<Msg>) {
    std::thread::Builder::new()
        .name("georag-tui-input".into())
        .spawn(move || {
            loop {
                if tx.is_closed() {
                    return;
                }
                match crossterm::event::poll(Duration::from_millis(100)) {
                    Ok(true) => match crossterm::event::read() {
                        Ok(Event::Key(k)) => {
                            if tx.blocking_send(Msg::Key(k)).is_err() {
                                return;
                            }
                        }
                        Ok(Event::Mouse(m)) => {
                            if tx.blocking_send(Msg::Mouse(m)).is_err() {
                                return;
                            }
                        }
                        Ok(Event::Resize(w, h)) => {
                            if tx.blocking_send(Msg::Resize(w, h)).is_err() {
                                return;
                            }
                        }
                        Ok(_) => {}
                        Err(e) => {
                            tracing::error!("terminal input failed: {e}");
                            return;
                        }
                    },
                    Ok(false) => {}
                    Err(e) => {
                        tracing::error!("terminal poll failed: {e}");
                        return;
                    }
                }
            }
        })
        .expect("spawning the input thread");
}

/// Load models on a thread; the UI opens at once and shows "loading".
fn spawn_app_open(home: Home, tx: mpsc::Sender<Msg>) {
    std::thread::Builder::new()
        .name("georag-app-open".into())
        .spawn(move || {
            let msg = match App::open(&home, None) {
                Ok(app) => Msg::AppReady(Arc::new(app)),
                Err(e) => Msg::AppFailed(e.to_string()),
            };
            let _ = tx.blocking_send(msg);
        })
        .expect("spawning the app thread");
}

async fn event_loop(
    terminal: &mut ratatui::DefaultTerminal,
    home: Home,
    mut state: State,
    theme: Theme,
) -> Result<(), TuiError> {
    let result = event_loop_inner(terminal, &home, &mut state, theme).await;
    shutdown(&mut state);
    result
}

/// Cancel a running pass, give its thread a few seconds to stop, then drop
/// the app so the engine thread unloads the models before we return.
fn shutdown(state: &mut State) {
    if let Some(c) = &state.chat.controls {
        c.request(georag_core::chat::RunControl::StopAll);
    }
    if let Some(handle) = state.chat.thread.take() {
        let deadline = std::time::Instant::now() + Duration::from_secs(8);
        while !handle.is_finished() && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(50));
        }
        if handle.is_finished() {
            let _ = handle.join();
        } else {
            tracing::warn!("chat run did not stop within 8 s; exiting anyway");
        }
    }
    state.app = None;
}

async fn event_loop_inner(
    terminal: &mut ratatui::DefaultTerminal,
    home: &Home,
    state: &mut State,
    theme: Theme,
) -> Result<(), TuiError> {
    let (tx, mut rx) = mpsc::channel::<Msg>(1024);
    spawn_reader(tx.clone());
    let mut tick = tokio::time::interval(TICK);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    let size = terminal.size().map_err(TuiError::Terminal)?;
    state.width = size.width;
    state.height = size.height;

    let registry = Registry::load(home).unwrap_or_default();
    if registry.in_use(ModelRole::Chat).is_some() || registry.in_use(ModelRole::Embed).is_some() {
        state.engine_state = EngineState::Loading;
        spawn_app_open(home.clone(), tx.clone());
    }
    perform(Cmd::RefreshConversations, state, home, &tx);
    perform(Cmd::RefreshModels, state, home, &tx);

    loop {
        terminal
            .draw(|f| draw(f, state, &theme))
            .map_err(TuiError::Loop)?;

        let first = tokio::select! {
            msg = rx.recv() => match msg {
                Some(m) => m,
                None => return Ok(()),
            },
            _ = tick.tick() => Msg::Tick,
        };
        let mut batch = vec![first];
        while let Ok(m) = rx.try_recv() {
            batch.push(m);
        }
        for msg in batch {
            let cmds = update(state, msg);
            for cmd in cmds {
                if cmd == Cmd::Quit {
                    return Ok(());
                }
                perform(cmd, state, home, &tx);
            }
            if state.should_quit {
                return Ok(());
            }
        }
    }
}

fn perform(cmd: Cmd, state: &mut State, home: &Home, tx: &mpsc::Sender<Msg>) {
    match cmd {
        Cmd::Quit => {}
        Cmd::Ask { message } => {
            let Some(app) = state.app.clone() else {
                state.show_toast("models are not loaded yet: pull a chat and an embedding model on the Models screen");
                return;
            };
            let req = chat::request_for(state, message.clone());
            let controls = Controls::new();
            state.chat.controls = Some(controls.clone());
            state.chat.running = true;
            state.chat.follow = true;
            state.chat.run = Some(chat::RunView {
                question: message,
                ..Default::default()
            });
            state.engine_state = EngineState::Generating;
            state.chat.thread = Some(chat::spawn_run(app, req, controls, tx.clone()));
        }
        Cmd::CombineDone { message_id } => {
            let Some(app) = state.app.clone() else { return };
            let controls = Controls::new();
            state.chat.controls = Some(controls.clone());
            state.chat.running = true;
            state.engine_state = EngineState::Generating;
            if let Some(run) = state.chat.run.as_mut() {
                run.done = false;
                run.final_started = true;
                run.final_text.clear();
            }
            let tx = tx.clone();
            std::thread::spawn(move || {
                let ctx = app.chat_context();
                let tx_events = tx.clone();
                let result = georag_core::chat::runner::combine_done(
                    &ctx,
                    message_id,
                    &controls,
                    &mut |ev| {
                        let _ = tx_events.blocking_send(Msg::Chat(ev));
                    },
                );
                let msg = match result {
                    Ok(_) => Msg::ChatFinished(Err("combined".into())),
                    Err(e) => Msg::ChatFinished(Err(e.to_string())),
                };
                // Report through the finished path without a full outcome.
                let _ = tx.blocking_send(match msg {
                    Msg::ChatFinished(Err(ref s)) if s == "combined" => {
                        Msg::Toast("combined the finished passes".into())
                    }
                    other => other,
                });
                let _ = tx.blocking_send(Msg::ChatFinished(Err(String::new())));
            });
        }
        Cmd::OpenConversation { id } => match georag_core::conversations::show(home, id) {
            Ok(view) => {
                let _ = tx.try_send(Msg::ConversationLoaded(view));
                state.chat.run = None;
            }
            Err(e) => state.show_toast(e.to_string()),
        },
        Cmd::DeleteConversation { id } => {
            match georag_core::conversations::delete(home, id) {
                Ok(()) => {
                    if state
                        .chat
                        .current
                        .as_ref()
                        .map(|v| v.conversation.id == id)
                        .unwrap_or(false)
                    {
                        state.chat.current = None;
                    }
                    state.show_toast(format!("conversation {id} moved to trash"));
                }
                Err(e) => state.show_toast(e.to_string()),
            }
            perform(Cmd::RefreshConversations, state, home, tx);
        }
        Cmd::ExportConversation => {
            let Some(id) = state.chat.current.as_ref().map(|v| v.conversation.id) else {
                state.show_toast("no conversation open");
                return;
            };
            match georag_core::conversations::export(home, id) {
                Ok(path) => state.show_toast(format!("exported to {}", path.display())),
                Err(e) => state.show_toast(e.to_string()),
            }
        }
        Cmd::ViewChunk { chunk_id } => {
            let Some(app) = &state.app else { return };
            match app.chunk(chunk_id) {
                Ok(Some(c)) => {
                    state.overlay = Some(Overlay::Chunk {
                        title: format!("{} {}", c.rel_path, c.locator.as_deref().unwrap_or("")),
                        text: c.text,
                        scroll: 0,
                    });
                }
                Ok(None) => state.show_toast("chunk not found"),
                Err(e) => state.show_toast(e.to_string()),
            }
        }
        Cmd::OpenFile { file_id } => {
            let Some(app) = &state.app else { return };
            match app.file_path(file_id) {
                Ok(Some(path)) if path.exists() => {
                    let program = if cfg!(target_os = "macos") {
                        "open"
                    } else if cfg!(target_os = "windows") {
                        "cmd"
                    } else {
                        "xdg-open"
                    };
                    let mut cmd = std::process::Command::new(program);
                    if cfg!(target_os = "windows") {
                        cmd.args(["/C", "start", ""]);
                    }
                    match cmd.arg(&path).spawn() {
                        Ok(_) => state.show_toast(format!("opened {}", path.display())),
                        Err(e) => {
                            state.show_toast(format!("could not open {}: {e}", path.display()))
                        }
                    }
                }
                Ok(Some(path)) => state.show_toast(format!("file not on disk: {}", path.display())),
                Ok(None) => state.show_toast("file not found"),
                Err(e) => state.show_toast(e.to_string()),
            }
        }
        Cmd::RefreshConversations => match georag_core::conversations::list(home, false) {
            Ok(list) => {
                let _ = tx.try_send(Msg::ConversationsListed(list));
            }
            Err(e) => tracing::warn!("listing conversations: {e}"),
        },
        Cmd::RefreshModels => {
            let registry = Registry::load(home).unwrap_or_default();
            state.models.installed = ops::list(home, &registry, Some(&state.pulls));
            let settings =
                georag_core::config::Settings::load(&home.settings_path()).unwrap_or_default();
            let free = state
                .snapshot
                .backend
                .as_ref()
                .map(|b| b.device.memory_free)
                .unwrap_or(0);
            state.models.fits = ops::fit_summary(home, &registry, &settings, free);
            state.snapshot.models.chat = registry.in_use(ModelRole::Chat).map(|m| m.id.clone());
            state.snapshot.models.embed = registry.in_use(ModelRole::Embed).map(|m| m.id.clone());
            state.snapshot.models.vision = registry.in_use(ModelRole::Vision).map(|m| m.id.clone());
            if state.models.installed_cursor >= state.models.installed.len() {
                state.models.installed_cursor = state.models.installed.len().saturating_sub(1);
            }
        }
        Cmd::PullModel { spec } => {
            let home = home.clone();
            let pulls = state.pulls.clone();
            let tx = tx.clone();
            let plan_tx = plan_sender();
            std::thread::spawn(move || {
                let result = ops::plan_pull(&home, &spec, None).and_then(|mut plan| {
                    ops::complete_plan(&mut plan)?;
                    ops::start_pull(&pulls, &plan)?;
                    Ok(plan)
                });
                match result {
                    Ok(plan) => {
                        let _ = plan_tx.send(plan.clone());
                        let _ = tx.blocking_send(Msg::Toast(format!(
                            "pulling {}:{}",
                            plan.repo, plan.file
                        )));
                    }
                    Err(e) => {
                        let _ = tx.blocking_send(Msg::Toast(format!("pull failed: {e}")));
                    }
                }
            });
        }
        Cmd::PausePull { id } => {
            state.pulls.pause(&id);
            state.show_toast(format!("pausing {id}; enter on it resumes"));
        }
        Cmd::UseModel { id } => {
            let mut registry = Registry::load(home).unwrap_or_default();
            match ops::use_model(home, &mut registry, &id) {
                Ok(entry) => {
                    state.show_toast(format!(
                        "{} is now the {} model; reloading",
                        entry.id, entry.role
                    ));
                    perform(Cmd::ReloadApp, state, home, tx);
                }
                Err(e) => state.show_toast(e.to_string()),
            }
            perform(Cmd::RefreshModels, state, home, tx);
        }
        Cmd::RemoveModel { id } => {
            let mut registry = Registry::load(home).unwrap_or_default();
            match ops::remove(home, &mut registry, &id) {
                Ok(entry) => state.show_toast(format!("removed {}", entry.id)),
                Err(e) => state.show_toast(e.to_string()),
            }
            perform(Cmd::RefreshModels, state, home, tx);
        }
        Cmd::VerifyModel { id } => {
            let home = home.clone();
            let tx = tx.clone();
            std::thread::spawn(move || {
                let registry = Registry::load(&home).unwrap_or_default();
                let text = match ops::verify(&home, &registry, &id) {
                    Ok(r) => match r.ok {
                        Some(true) => format!("{} verified", r.id),
                        Some(false) => format!("{} is corrupt: hash mismatch", r.id),
                        None => format!("{} hashed, no recorded sha256 to compare", r.id),
                    },
                    Err(e) => e.to_string(),
                };
                let _ = tx.blocking_send(Msg::Toast(text));
            });
        }
        Cmd::LinkModel { spec } => {
            let (role, path) = match spec.split_once(':') {
                Some((r, p)) if matches!(r, "chat" | "embed" | "vision") => (
                    r.parse::<ModelRole>().unwrap_or(ModelRole::Chat),
                    p.trim().to_string(),
                ),
                _ => (ModelRole::Chat, spec.clone()),
            };
            let mut registry = Registry::load(home).unwrap_or_default();
            match ops::link(home, &mut registry, std::path::Path::new(&path), role, None) {
                Ok(entry) => state.show_toast(format!("linked {} as {}", entry.id, entry.role)),
                Err(e) => state.show_toast(e.to_string()),
            }
            perform(Cmd::RefreshModels, state, home, tx);
        }
        Cmd::SetRole { id, role } => {
            let mut registry = Registry::load(home).unwrap_or_default();
            match role.parse::<ModelRole>() {
                Ok(role) => {
                    if let Some(m) = registry.models.iter_mut().find(|m| m.id == id) {
                        m.role = role;
                        m.in_use = false;
                    }
                    registry.ensure_in_use(role);
                    match registry.save(home) {
                        Ok(()) => state.show_toast(format!("{id} is now a {role} model")),
                        Err(e) => state.show_toast(e.to_string()),
                    }
                }
                Err(e) => state.show_toast(e.to_string()),
            }
            perform(Cmd::RefreshModels, state, home, tx);
        }
        Cmd::ReloadApp => {
            if state.chat.running {
                state.show_toast("finish or stop the run before reloading models");
                return;
            }
            state.app = None;
            state.engine_state = EngineState::Loading;
            spawn_app_open(home.clone(), tx.clone());
        }
    }
    // Finished pulls get registered on the next refresh.
    register_finished_pulls(state, home, tx);
}

/// Plans for pulls started from the UI, so finished downloads can be
/// registered. A channel because the plan is resolved on a thread.
fn plan_sender() -> std::sync::mpsc::Sender<ops::PullPlan> {
    let (tx, rx) = std::sync::mpsc::channel();
    PLAN_RX.with(|cell| {
        cell.borrow_mut().push(rx);
    });
    tx
}

thread_local! {
    static PLAN_RX: std::cell::RefCell<Vec<std::sync::mpsc::Receiver<ops::PullPlan>>> = const { std::cell::RefCell::new(Vec::new()) };
}

fn register_finished_pulls(state: &mut State, home: &Home, tx: &mpsc::Sender<Msg>) {
    PLAN_RX.with(|cell| {
        for rx in cell.borrow().iter() {
            while let Ok(plan) = rx.try_recv() {
                state.pending_pulls.push(plan);
            }
        }
    });
    let mut registered = Vec::new();
    for (i, plan) in state.pending_pulls.iter().enumerate() {
        if let Some(st) = state.pulls.status(&plan.id)
            && st.state == georag_core::models::PullState::Done
        {
            let mut registry = Registry::load(home).unwrap_or_default();
            match ops::register_pulled(home, &mut registry, plan) {
                Ok(entry) => {
                    let _ = tx.try_send(Msg::Toast(format!("{} ready ({})", entry.id, entry.role)));
                    registered.push(i);
                }
                Err(e) => {
                    let _ = tx.try_send(Msg::Toast(format!("could not register {}: {e}", plan.id)));
                    registered.push(i);
                }
            }
        }
    }
    for i in registered.into_iter().rev() {
        state.pending_pulls.remove(i);
    }
}
