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
use crate::screens::update::UpdatePhase;
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

/// Once-a-day release check for the status-bar notice; never installs.
fn spawn_update_notice(home: Home, tx: mpsc::Sender<Msg>) {
    std::thread::spawn(move || {
        let mut settings =
            georag_core::config::Settings::load(&home.settings_path()).unwrap_or_default();
        match georag_core::update::check_for_notice(&home, &mut settings) {
            Ok(Some(n)) if n.newer => {
                let _ =
                    tx.blocking_send(Msg::UpdateNotice(format!("update {} available", n.latest)));
            }
            Ok(_) => {}
            Err(e) => tracing::info!("update check skipped: {e}"),
        }
    });
}

/// Watch the roots while the UI is open; changes rescan and queue indexing.
fn spawn_watcher(home: Home, tx: mpsc::Sender<Msg>) {
    std::thread::Builder::new()
        .name("georag-watch".into())
        .spawn(move || {
            let Ok(store) = georag_core::app::open_store(&home) else {
                return;
            };
            let roots: Vec<std::path::PathBuf> = store
                .roots()
                .unwrap_or_default()
                .into_iter()
                .map(|r| std::path::PathBuf::from(r.path))
                .collect();
            if roots.is_empty() {
                return;
            }
            let debounce = georag_core::config::Settings::load(&home.settings_path())
                .map(|s| Duration::from_millis(s.index.watch_debounce_ms))
                .unwrap_or(Duration::from_secs(2));
            let watcher = match georag_core::pipeline::watch::Watcher::new(&roots, debounce) {
                Ok(w) => w,
                Err(e) => {
                    tracing::warn!("{e}");
                    return;
                }
            };
            loop {
                if tx.is_closed() {
                    return;
                }
                let changed = watcher.changed_roots(Duration::from_secs(5));
                if changed.is_empty() {
                    continue;
                }
                for root in store.roots().unwrap_or_default() {
                    if changed
                        .iter()
                        .any(|c| c == std::path::Path::new(&root.path))
                    {
                        if let Err(e) = georag_core::pipeline::scan_root(&store, &root) {
                            tracing::warn!("{e}");
                        }
                    }
                }
                let _ = tx.blocking_send(Msg::Toast("library changed: indexing".into()));
                let _ = tx.blocking_send(Msg::StartIndexing);
            }
        })
        .expect("spawning the watcher thread");
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
    if let Some(flag) = &state.indexing {
        flag.store(false, std::sync::atomic::Ordering::Relaxed);
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
    spawn_update_notice(home.clone(), tx.clone());
    spawn_watcher(home.clone(), tx.clone());

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
        Cmd::UpdateCheck => {
            state.update.phase = UpdatePhase::Checking;
            let channel = georag_core::config::Settings::load(&home.settings_path())
                .map(|s| s.update.channel)
                .unwrap_or_else(|_| "stable".into());
            let tx = tx.clone();
            std::thread::spawn(move || {
                let result = georag_core::update::check(&channel).map(|r| {
                    let notes = georag_core::update::fetch_notes(&r).unwrap_or_default();
                    (r, notes)
                });
                let _ = tx.blocking_send(Msg::UpdateChecked(result.map_err(|e| e.to_string())));
            });
        }
        Cmd::UpdateDownload => {
            let Some(report) = state.update.report.clone() else {
                return;
            };
            state.update.phase = UpdatePhase::Downloading {
                bytes: 0,
                total: report.asset.as_ref().map(|a| a.size),
            };
            let channel = georag_core::config::Settings::load(&home.settings_path())
                .map(|s| s.update.channel)
                .unwrap_or_else(|_| "stable".into());
            let home = home.clone();
            let tx = tx.clone();
            std::thread::spawn(move || {
                let progress_tx = tx.clone();
                let result =
                    georag_core::update::download(&home, &report, &channel, |bytes, total| {
                        let _ = progress_tx.try_send(Msg::UpdateProgress { bytes, total });
                    })
                    .and_then(|dl| georag_core::update::stage(&home, &dl.bundle));
                let _ = tx.blocking_send(Msg::UpdateStaged(result.map_err(|e| e.to_string())));
            });
        }
        Cmd::UpdateApply => {
            let UpdatePhase::Staged(staged) = state.update.phase.clone() else {
                return;
            };
            if state.chat.running {
                state.show_toast("finish or stop the run before updating");
                return;
            }
            // Drain: unload the engine so the new binary can take over cleanly.
            state.app = None;
            state.engine_state = EngineState::NotLoaded;
            state.update.phase = UpdatePhase::Applying;
            let home = home.clone();
            let tx = tx.clone();
            std::thread::spawn(move || {
                let result = std::env::current_exe()
                    .map_err(|e| e.to_string())
                    .and_then(|exe| georag_core::update::apply(&home, &staged, &exe).map_err(|e| e.to_string()))
                    .and_then(|r| {
                        if r.rolled_back {
                            Err("the new version failed its self-check; the previous version was restored".to_string())
                        } else {
                            Ok("updated: quit and start GeoRAG again to run the new version".to_string())
                        }
                    });
                let _ = tx.blocking_send(Msg::UpdateApplied(result));
            });
        }
        Cmd::UpdateRollback => {
            let result = std::env::current_exe()
                .map_err(|e| e.to_string())
                .and_then(|exe| {
                    georag_core::update::rollback(home, &exe).map_err(|e| e.to_string())
                });
            match result {
                Ok(true) => {
                    state.update.phase =
                        UpdatePhase::Done("previous version restored: quit and start again".into())
                }
                Ok(false) => state.show_toast("nothing to roll back: no .old copies found"),
                Err(e) => state.update.phase = UpdatePhase::Failed(e),
            }
        }
        Cmd::RefreshLibrary => {
            let home = home.clone();
            let query = state.library.query.clone();
            let tx = tx.clone();
            std::thread::spawn(move || {
                let result = (|| -> Result<Msg, String> {
                    let store = georag_core::app::open_store(&home).map_err(|e| e.to_string())?;
                    let q = georag_core::FileQuery {
                        limit: 5000,
                        ..query
                    };
                    let files = store.files_query(&q).map_err(|e| e.to_string())?;
                    let facets = store.files_facets(&q).map_err(|e| e.to_string())?;
                    let mut tags = std::collections::HashMap::new();
                    for f in &files {
                        let t: Vec<String> = store
                            .file_tags(f.id)
                            .map_err(|e| e.to_string())?
                            .into_iter()
                            .map(|t| t.tag)
                            .collect();
                        if !t.is_empty() {
                            tags.insert(f.id, t);
                        }
                    }
                    Ok(Msg::LibraryLoaded {
                        files,
                        tags,
                        facets,
                    })
                })();
                let _ = tx.blocking_send(result.unwrap_or_else(Msg::Toast));
            });
        }
        Cmd::OpenRecord { file_id } => {
            match georag_core::app::open_store(home).and_then(|store| {
                let file = store
                    .file_get(file_id)?
                    .ok_or_else(|| georag_core::CoreError::Invalid("file not found".into()))?;
                let tags = store.file_tags(file_id)?;
                let chunks = store.chunks_for_file(file_id)?;
                Ok(crate::screens::library::RecordView {
                    file,
                    tags,
                    chunks,
                    scroll: 0,
                })
            }) {
                Ok(rec) => state.library.record = Some(rec),
                Err(e) => state.show_toast(e.to_string()),
            }
        }
        Cmd::TagFiles { ids, tag, remove } => {
            match georag_core::app::open_store(home) {
                Ok(store) => {
                    let mut errors = 0;
                    for id in &ids {
                        let r = if remove {
                            store.file_tag_remove(*id, &tag).map(|_| ())
                        } else {
                            store.file_tag_add(*id, &tag, "manual", None, false)
                        };
                        if r.is_err() {
                            errors += 1;
                        }
                    }
                    if errors > 0 {
                        state.show_toast(format!(
                            "tag {tag:?} unknown: add it on the command line with `georag tags add`"
                        ));
                    }
                }
                Err(e) => state.show_toast(e.to_string()),
            }
            state.library.selected.clear();
            perform(Cmd::RefreshLibrary, state, home, tx);
        }
        Cmd::Reprocess { ids } => {
            match georag_core::app::open_store(home) {
                Ok(store) => {
                    for id in &ids {
                        let _ = store.job_enqueue("extract", Some(*id), None);
                    }
                    state.show_toast(format!("queued {} file(s)", ids.len()));
                }
                Err(e) => state.show_toast(e.to_string()),
            }
            perform(Cmd::StartIndexing, state, home, tx);
        }
        Cmd::Rescan => {
            match georag_core::app::open_store(home)
                .and_then(|store| georag_core::pipeline::scan_all(&store))
            {
                Ok(reports) => {
                    let queued: usize = reports.iter().map(|r| r.queued).sum();
                    state.show_toast(format!(
                        "rescanned {} root(s): {queued} file(s) queued",
                        reports.len()
                    ));
                }
                Err(e) => state.show_toast(e.to_string()),
            }
            perform(Cmd::StartIndexing, state, home, tx);
        }
        Cmd::AddRoot { path } => {
            let expanded = if let Some(rest) = path.strip_prefix("~/") {
                std::env::var("HOME")
                    .map(|h| format!("{h}/{rest}"))
                    .unwrap_or(path.clone())
            } else {
                path.clone()
            };
            match georag_core::app::open_store(home).and_then(|store| {
                let root =
                    georag_core::library::add(&store, std::path::Path::new(&expanded), None)?;
                let report = georag_core::pipeline::scan_root(&store, &root)?;
                Ok((root, report))
            }) {
                Ok((root, report)) => {
                    state.show_toast(format!(
                        "added {:?}: {} files seen, {} queued",
                        root.name, report.seen, report.queued
                    ));
                    perform(Cmd::StartIndexing, state, home, tx);
                }
                Err(e) => state.show_toast(e.to_string()),
            }
            perform(Cmd::RefreshSettings, state, home, tx);
            perform(Cmd::RefreshLibrary, state, home, tx);
        }
        Cmd::RemoveRoot { name } => {
            match georag_core::app::open_store(home)
                .and_then(|store| georag_core::library::remove(&store, &name))
            {
                Ok(()) => state.show_toast(format!("removed root {name:?}")),
                Err(e) => state.show_toast(e.to_string()),
            }
            perform(Cmd::RefreshSettings, state, home, tx);
            perform(Cmd::RefreshLibrary, state, home, tx);
        }
        Cmd::StartIndexing => {
            if state.indexing.is_some() {
                return;
            }
            let Some(app) = state.app.clone() else {
                state.show_toast(
                    "models are not loaded: indexing waits for a chat and an embedding model",
                );
                return;
            };
            if state.chat.running {
                state.show_toast("indexing starts when the current run finishes");
                return;
            }
            let flag = Arc::new(std::sync::atomic::AtomicBool::new(true));
            state.indexing = Some(flag.clone());
            let tx = tx.clone();
            std::thread::Builder::new()
                .name("georag-index".into())
                .spawn(move || {
                    let keep = flag.clone();
                    let ev_tx = tx.clone();
                    let opts = georag_core::pipeline::IndexOptions::default();
                    let result =
                        georag_core::pipeline::supervisor::queue(&app, &opts).and_then(|_| {
                            georag_core::pipeline::run(
                                &app,
                                &opts,
                                &mut |ev| {
                                    let _ = ev_tx.try_send(Msg::Pipeline(ev));
                                },
                                &|| keep.load(std::sync::atomic::Ordering::Relaxed),
                            )
                        });
                    let _ =
                        tx.blocking_send(Msg::IndexingFinished(result.map_err(|e| e.to_string())));
                })
                .expect("spawning the indexing thread");
        }
        Cmd::RefreshSettings => {
            let entries = georag_core::config::Settings::load(&home.settings_path())
                .map(|s| s.entries())
                .unwrap_or_default();
            let roots = georag_core::app::open_store(home)
                .and_then(|s| Ok(s.roots()?))
                .unwrap_or_default();
            let _ = tx.try_send(Msg::SettingsLoaded { entries, roots });
        }
        Cmd::SetSetting { key, value } => {
            let result =
                georag_core::config::Settings::load(&home.settings_path()).and_then(|mut s| {
                    s.set(&key, &value)?;
                    s.save(&home.settings_path())?;
                    Ok(s)
                });
            match result {
                Ok(_) => {
                    state.show_toast(format!("{key} = {value}"));
                    if key == "engine.backend" {
                        perform(Cmd::ReloadApp, state, home, tx);
                    }
                }
                Err(e) => state.show_toast(e.to_string()),
            }
            perform(Cmd::RefreshSettings, state, home, tx);
        }
        Cmd::GotoUpdate => {
            state.screen = crate::state::Screen::Update;
        }
        Cmd::RunDoctor => {
            let home = home.clone();
            let tx = tx.clone();
            state.show_toast("running doctor (loads the models; a few seconds)");
            std::thread::spawn(move || {
                let settings =
                    georag_core::config::Settings::load(&home.settings_path()).unwrap_or_default();
                let text = match georag_core::doctor::run(
                    &home,
                    &settings,
                    &georag_core::doctor::DoctorOptions {
                        backend: None,
                        ci: true,
                        scores: None,
                    },
                ) {
                    Ok(r) => r.to_string(),
                    Err(e) => format!("doctor failed: {e}"),
                };
                let _ = tx.blocking_send(Msg::DoctorText(text));
            });
        }
        Cmd::GotoLibrary => {
            state.screen = crate::state::Screen::Library;
            perform(Cmd::RefreshLibrary, state, home, tx);
        }
        Cmd::ImportLocate { path } => {
            let path = crate::update::expand_home(&path);
            let tx = tx.clone();
            state.import.busy = true;
            std::thread::spawn(move || {
                let result = georag_core::import_v1::locate(std::path::Path::new(&path))
                    .map(|(l, _)| (l.default_root, l.source))
                    .map_err(|e| e.to_string());
                let _ = tx.blocking_send(Msg::ImportLocated(result));
            });
        }
        Cmd::ImportPreview => {
            let (Some(source), Some(root)) =
                (state.import.source.clone(), state.import.root.clone())
            else {
                return;
            };
            let home = home.clone();
            let tx = tx.clone();
            state.import.busy = true;
            std::thread::spawn(move || {
                let result = (|| -> Result<georag_core::import_v1::Preview, String> {
                    let (_, dump) =
                        georag_core::import_v1::locate(&source).map_err(|e| e.to_string())?;
                    let store = georag_core::app::open_store(&home).map_err(|e| e.to_string())?;
                    let defs = georag_core::tags::TagFile::load_or_create(&home)
                        .map_err(|e| e.to_string())?;
                    georag_core::import_v1::preview(&store, &dump, &root, &defs)
                        .map_err(|e| e.to_string())
                })();
                let _ = tx.blocking_send(Msg::ImportPreviewed(result));
            });
        }
        Cmd::ImportRun => {
            let (Some(source), Some(root)) =
                (state.import.source.clone(), state.import.root.clone())
            else {
                return;
            };
            let opts = georag_core::import_v1::ImportOptions {
                with_chats: state.import.with_chats,
                retag: state.import.retag,
                root_name: None,
            };
            let home = home.clone();
            let tx = tx.clone();
            std::thread::spawn(move || {
                let result = (|| -> Result<georag_core::import_v1::ImportReport, String> {
                    let (_, dump) =
                        georag_core::import_v1::locate(&source).map_err(|e| e.to_string())?;
                    let store = georag_core::app::open_store(&home).map_err(|e| e.to_string())?;
                    georag_core::import_v1::run(&home, &store, &dump, &root, &opts)
                        .map_err(|e| e.to_string())
                })();
                let _ = tx.blocking_send(Msg::ImportFinished(result));
            });
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
