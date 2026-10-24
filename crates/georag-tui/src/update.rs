//! `update(state, msg) -> commands`: every state change lives here.

use crossterm::event::{
    KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};

use crate::screens::update::UpdatePhase;
use crate::screens::{chat, models, update as update_screen};
use crate::state::{
    Cmd, ConfirmAction, EngineState, HOME_MENU, HomeFocus, InputPurpose, MenuItem, Msg, Overlay,
    Screen, State,
};
use crate::view::tab_at_column;

pub fn update(state: &mut State, msg: Msg) -> Vec<Cmd> {
    match msg {
        Msg::Tick => {
            state.tick += 1;
            if let Some((_, left)) = &mut state.toast {
                *left = left.saturating_sub(1);
                if *left == 0 {
                    state.toast = None;
                }
            }
            state.models.pulls = state.pulls.statuses();
            Vec::new()
        }
        Msg::Resize(w, h) => {
            state.width = w;
            state.height = h;
            Vec::new()
        }
        Msg::Key(key) => {
            if key.kind == KeyEventKind::Release {
                return Vec::new();
            }
            on_key(state, key)
        }
        Msg::Mouse(mouse) => on_mouse(state, mouse),
        Msg::Chat(ev) => {
            state.chat.apply_event(ev);
            Vec::new()
        }
        Msg::ChatFinished(result) => {
            state.chat.running = false;
            state.chat.controls = None;
            state.engine_state = if state.app.is_some() {
                EngineState::Ready
            } else {
                EngineState::NotLoaded
            };
            match result {
                Ok(outcome) => {
                    if let Some(run) = state.chat.run.as_mut() {
                        run.done = true;
                        if run.final_text.is_empty() && run.passes.len() <= 1 {
                            run.final_text = outcome.final_answer.clone();
                        }
                    }
                    let mut cmds = vec![Cmd::RefreshConversations];
                    if outcome.stopped
                        && outcome
                            .passes
                            .iter()
                            .any(|p| p.status == georag_core::chat::PassStatus::Done)
                    {
                        state.overlay = Some(Overlay::Confirm {
                            text: "Stopped. Combine what is done into one answer?".into(),
                            on_yes: ConfirmAction::CombineDone {
                                message_id: outcome.message_id,
                            },
                        });
                    }
                    cmds.push(Cmd::OpenConversation {
                        id: outcome.conversation_id,
                    });
                    cmds
                }
                Err(e) => {
                    if let Some(run) = state.chat.run.as_mut() {
                        run.done = true;
                        run.error = Some(e.clone());
                    }
                    state.show_toast(format!("run failed: {e}"));
                    Vec::new()
                }
            }
        }
        Msg::AppReady(app) => {
            state.engine_state = EngineState::Ready;
            state.models.loaded_chat = app.plan.chat_id.clone();
            state.models.loaded_embed = app.plan.embed_id.clone();
            state.app = Some(app);
            vec![Cmd::RefreshModels, Cmd::RefreshConversations]
        }
        Msg::AppFailed(e) => {
            state.engine_state = EngineState::Failed(e.clone());
            state.app = None;
            state.show_toast(format!("models not loaded: {e}"));
            vec![Cmd::RefreshModels, Cmd::RefreshConversations]
        }
        Msg::ConversationLoaded(view) => {
            state.chat.current = Some(view);
            state.chat.follow = true;
            Vec::new()
        }
        Msg::ConversationsListed(list) => {
            state.chat.conversations = list;
            let n = state.chat.filtered_conversations().len();
            if state.chat.conv_cursor >= n {
                state.chat.conv_cursor = n.saturating_sub(1);
            }
            Vec::new()
        }
        Msg::ModelsChanged => vec![Cmd::RefreshModels],
        Msg::Toast(text) => {
            state.show_toast(text);
            Vec::new()
        }
        Msg::UpdateChecked(result) => {
            match result {
                Ok((report, notes)) => {
                    if report.newer {
                        state.update_notice = Some(format!("update {} available", report.latest));
                    }
                    state.update.report = Some(report);
                    state.update.notes = notes;
                    state.update.phase = UpdatePhase::Checked;
                }
                Err(e) => state.update.phase = UpdatePhase::Failed(e),
            }
            Vec::new()
        }
        Msg::UpdateProgress { bytes, total } => {
            state.update.phase = UpdatePhase::Downloading { bytes, total };
            Vec::new()
        }
        Msg::UpdateStaged(result) => {
            state.update.phase = match result {
                Ok(p) => UpdatePhase::Staged(p),
                Err(e) => UpdatePhase::Failed(e),
            };
            Vec::new()
        }
        Msg::UpdateApplied(result) => {
            state.update.phase = match result {
                Ok(s) => UpdatePhase::Done(s),
                Err(e) => UpdatePhase::Failed(e),
            };
            Vec::new()
        }
        Msg::UpdateNotice(text) => {
            state.update_notice = Some(text);
            Vec::new()
        }
    }
}

fn on_key(state: &mut State, key: KeyEvent) -> Vec<Cmd> {
    // ctrl-c quits now, from anywhere; the daemon (P3) keeps running.
    if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
        state.should_quit = true;
        return vec![Cmd::Quit];
    }

    if let Some(overlay) = state.overlay.clone() {
        return on_overlay_key(state, overlay, key);
    }

    // Screens with text input take printable keys first.
    if state.screen == Screen::Chat {
        if let Some(cmds) = chat::on_screen_key(state, key) {
            return cmds;
        }
        let typing = state.chat.focus == chat::ChatFocus::Transcript;
        if typing
            && !matches!(key.code, KeyCode::Esc | KeyCode::Tab | KeyCode::BackTab)
            && !(key.modifiers.contains(KeyModifiers::CONTROL))
        {
            if matches!(key.code, KeyCode::Char(_)) && state.chat.input.is_empty() {
                // Fall through to global keys for single-letter commands when the box is empty.
                if let Some(cmds) = global_key(state, key) {
                    return cmds;
                }
            }
            return chat::on_key(state, key);
        }
        if let Some(cmds) = global_key(state, key) {
            return cmds;
        }
        return chat::on_key(state, key);
    }

    if let Some(cmds) = global_key(state, key) {
        return cmds;
    }
    match state.screen {
        Screen::Home => on_home_key(state, key),
        Screen::Models => models::on_key(state, key),
        Screen::Update => update_screen::on_key(state, key),
        _ => Vec::new(),
    }
}

/// Keys that mean the same on every screen. `None` when not a global key.
fn global_key(state: &mut State, key: KeyEvent) -> Option<Vec<Cmd>> {
    match key.code {
        KeyCode::Char('?') => {
            state.overlay = Some(Overlay::Help { scroll: 0 });
            Some(Vec::new())
        }
        KeyCode::Char(c @ '1'..='5')
            if state.screen != Screen::Chat || state.chat.focus != chat::ChatFocus::Sources =>
        {
            let idx = (c as u8 - b'1') as usize;
            Some(goto(state, Screen::TABS[idx]))
        }
        KeyCode::Tab if state.screen != Screen::Chat && state.screen != Screen::Models => {
            let next = match state.screen.tab_index() {
                Some(i) => Screen::TABS[(i + 1) % Screen::TABS.len()],
                None => Screen::TABS[0],
            };
            Some(goto(state, next))
        }
        KeyCode::BackTab if state.screen != Screen::Chat && state.screen != Screen::Models => {
            let n = Screen::TABS.len();
            let prev = match state.screen.tab_index() {
                Some(i) => Screen::TABS[(i + n - 1) % n],
                None => Screen::TABS[n - 1],
            };
            Some(goto(state, prev))
        }
        KeyCode::Char('q') => Some(if state.screen == Screen::Home {
            quit(state)
        } else {
            goto(state, Screen::Home)
        }),
        KeyCode::Esc if !(state.screen == Screen::Chat && state.chat.running) => Some(Vec::new()),
        _ => None,
    }
}

fn on_overlay_key(state: &mut State, overlay: Overlay, key: KeyEvent) -> Vec<Cmd> {
    match overlay {
        Overlay::Help { scroll } => {
            match key.code {
                KeyCode::Esc | KeyCode::Char('?') | KeyCode::Char('q') | KeyCode::Enter => {
                    state.overlay = None
                }
                KeyCode::Char('j') | KeyCode::Down => {
                    state.overlay = Some(Overlay::Help {
                        scroll: scroll.saturating_add(1),
                    })
                }
                KeyCode::Char('k') | KeyCode::Up => {
                    state.overlay = Some(Overlay::Help {
                        scroll: scroll.saturating_sub(1),
                    })
                }
                KeyCode::Char('g') => state.overlay = Some(Overlay::Help { scroll: 0 }),
                KeyCode::PageDown => {
                    state.overlay = Some(Overlay::Help {
                        scroll: scroll.saturating_add(10),
                    })
                }
                KeyCode::PageUp => {
                    state.overlay = Some(Overlay::Help {
                        scroll: scroll.saturating_sub(10),
                    })
                }
                _ => {}
            }
            Vec::new()
        }
        Overlay::Chunk {
            title,
            text,
            scroll,
        } => {
            match key.code {
                KeyCode::Esc | KeyCode::Char('q') | KeyCode::Enter => state.overlay = None,
                KeyCode::Char('j') | KeyCode::Down => {
                    state.overlay = Some(Overlay::Chunk {
                        title,
                        text,
                        scroll: scroll.saturating_add(1),
                    })
                }
                KeyCode::Char('k') | KeyCode::Up => {
                    state.overlay = Some(Overlay::Chunk {
                        title,
                        text,
                        scroll: scroll.saturating_sub(1),
                    })
                }
                KeyCode::PageDown => {
                    state.overlay = Some(Overlay::Chunk {
                        title,
                        text,
                        scroll: scroll.saturating_add(10),
                    })
                }
                KeyCode::PageUp => {
                    state.overlay = Some(Overlay::Chunk {
                        title,
                        text,
                        scroll: scroll.saturating_sub(10),
                    })
                }
                _ => {}
            }
            Vec::new()
        }
        Overlay::Confirm { text, on_yes } => match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
                state.overlay = None;
                match on_yes {
                    ConfirmAction::DeleteConversation { id } => {
                        vec![Cmd::DeleteConversation { id }]
                    }
                    ConfirmAction::RemoveModel { id } => vec![Cmd::RemoveModel { id }],
                    ConfirmAction::CombineDone { message_id } => {
                        vec![Cmd::CombineDone { message_id }]
                    }
                    ConfirmAction::ApplyUpdate => vec![Cmd::UpdateApply],
                    ConfirmAction::RollbackUpdate => vec![Cmd::UpdateRollback],
                    ConfirmAction::Quit => {
                        state.should_quit = true;
                        vec![Cmd::Quit]
                    }
                }
            }
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                state.overlay = None;
                Vec::new()
            }
            _ => {
                state.overlay = Some(Overlay::Confirm { text, on_yes });
                Vec::new()
            }
        },
        Overlay::TextInput {
            title,
            mut value,
            purpose,
        } => match key.code {
            KeyCode::Esc => {
                state.overlay = None;
                Vec::new()
            }
            KeyCode::Enter => {
                state.overlay = None;
                commit_input(state, purpose, value)
            }
            KeyCode::Backspace => {
                value.pop();
                state.overlay = Some(Overlay::TextInput {
                    title,
                    value,
                    purpose,
                });
                Vec::new()
            }
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                value.push(c);
                state.overlay = Some(Overlay::TextInput {
                    title,
                    value,
                    purpose,
                });
                Vec::new()
            }
            _ => {
                state.overlay = Some(Overlay::TextInput {
                    title,
                    value,
                    purpose,
                });
                Vec::new()
            }
        },
    }
}

fn commit_input(state: &mut State, purpose: InputPurpose, value: String) -> Vec<Cmd> {
    let value = value.trim().to_string();
    match purpose {
        InputPurpose::ChatTags => {
            state.chat.tags = value
                .split(',')
                .map(|t| t.trim().to_string())
                .filter(|t| !t.is_empty())
                .collect();
            Vec::new()
        }
        InputPurpose::ConversationFilter => {
            state.chat.conv_filter = value;
            state.chat.conv_cursor = 0;
            Vec::new()
        }
        InputPurpose::CatalogQuery => {
            state.models.catalog_query = value;
            state.models.catalog_cursor = 0;
            state.models.focus = Some(crate::screens::models::ModelsFocus::Catalog);
            Vec::new()
        }
        InputPurpose::LinkModel => {
            if value.is_empty() {
                Vec::new()
            } else {
                vec![Cmd::LinkModel { spec: value }]
            }
        }
        InputPurpose::SetRole { id } => {
            if value.is_empty() {
                Vec::new()
            } else {
                vec![Cmd::SetRole { id, role: value }]
            }
        }
    }
}

fn on_home_key(state: &mut State, key: KeyEvent) -> Vec<Cmd> {
    let first_run = state.snapshot.is_first_run();
    match key.code {
        KeyCode::Char('i') => goto(state, Screen::Import),
        KeyCode::Char('u') => goto(state, Screen::Update),
        KeyCode::Left | KeyCode::Char('h') => {
            state.home_focus = HomeFocus::Menu;
            Vec::new()
        }
        KeyCode::Right | KeyCode::Char('l') if first_run => {
            state.home_focus = HomeFocus::Checklist;
            Vec::new()
        }
        KeyCode::Char('j') | KeyCode::Down => {
            move_cursor(state, 1);
            Vec::new()
        }
        KeyCode::Char('k') | KeyCode::Up => {
            move_cursor(state, -1);
            Vec::new()
        }
        KeyCode::Char('g') | KeyCode::Home => {
            set_cursor(state, 0);
            Vec::new()
        }
        KeyCode::Char('G') | KeyCode::End => {
            set_cursor(state, usize::MAX);
            Vec::new()
        }
        KeyCode::Enter => match state.home_focus {
            HomeFocus::Menu => activate_menu(state, HOME_MENU[state.menu_cursor].2),
            HomeFocus::Checklist => {
                let item = state.checklist()[state.checklist_cursor].clone();
                goto(state, item.opens)
            }
        },
        _ => Vec::new(),
    }
}

fn move_cursor(state: &mut State, delta: i32) {
    let (cursor, len) = cursor_and_len(state);
    let next = (cursor as i64 + delta as i64).clamp(0, len as i64 - 1) as usize;
    set_cursor(state, next);
}

fn set_cursor(state: &mut State, to: usize) {
    let (_, len) = cursor_and_len(state);
    let to = to.min(len.saturating_sub(1));
    match state.home_focus {
        HomeFocus::Menu => state.menu_cursor = to,
        HomeFocus::Checklist => state.checklist_cursor = to,
    }
}

fn cursor_and_len(state: &State) -> (usize, usize) {
    match state.home_focus {
        HomeFocus::Menu => (state.menu_cursor, HOME_MENU.len()),
        HomeFocus::Checklist => (state.checklist_cursor, state.checklist().len()),
    }
}

fn activate_menu(state: &mut State, item: MenuItem) -> Vec<Cmd> {
    match item {
        MenuItem::Screen(s) => goto(state, s),
        MenuItem::Help => {
            state.overlay = Some(Overlay::Help { scroll: 0 });
            Vec::new()
        }
        MenuItem::Quit => quit(state),
    }
}

fn on_mouse(state: &mut State, mouse: MouseEvent) -> Vec<Cmd> {
    if !state.mouse || state.overlay.is_some() {
        return Vec::new();
    }
    if let MouseEventKind::Down(MouseButton::Left) = mouse.kind {
        // Row 1 (zero-based) is the tab row on every screen.
        if mouse.row == 1
            && let Some(screen) = tab_at_column(mouse.column)
        {
            return goto(state, screen);
        }
    }
    Vec::new()
}

fn goto(state: &mut State, screen: Screen) -> Vec<Cmd> {
    state.screen = screen;
    state.overlay = None;
    match screen {
        Screen::Chat => vec![Cmd::RefreshConversations],
        Screen::Models => vec![Cmd::RefreshModels],
        _ => Vec::new(),
    }
}

fn quit(state: &mut State) -> Vec<Cmd> {
    if state.busy() {
        state.overlay = Some(Overlay::Confirm {
            text: "A run is in progress. Quit anyway?".into(),
            on_yes: ConfirmAction::Quit,
        });
        return Vec::new();
    }
    state.should_quit = true;
    vec![Cmd::Quit]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::{Depth, Theme};
    use georag_core::chat::{ChatEvent, PassStatus, PlanSummary};
    use georag_core::status::Snapshot;
    use std::path::PathBuf;

    fn state() -> State {
        State::new(
            Snapshot::empty(PathBuf::from("/tmp/georag-test")),
            true,
            Theme::with_depth(Depth::Basic16),
        )
    }

    fn key(code: KeyCode) -> Msg {
        Msg::Key(KeyEvent::new(code, KeyModifiers::NONE))
    }

    fn press(state: &mut State, code: KeyCode) -> Vec<Cmd> {
        update(state, key(code))
    }

    #[test]
    fn number_keys_jump_to_tabs_and_q_returns_home() {
        let mut s = state();
        assert!(press(&mut s, KeyCode::Char('2')).is_empty());
        assert_eq!(s.screen, Screen::Library);
        press(&mut s, KeyCode::Char('5'));
        assert_eq!(s.screen, Screen::Settings);
        press(&mut s, KeyCode::Char('q'));
        assert_eq!(s.screen, Screen::Home);
        assert!(!s.should_quit);
    }

    #[test]
    fn q_on_home_quits_and_ctrl_c_quits_anywhere() {
        let mut s = state();
        assert_eq!(press(&mut s, KeyCode::Char('q')), vec![Cmd::Quit]);
        assert!(s.should_quit);

        let mut s = state();
        press(&mut s, KeyCode::Char('3'));
        let cmds = update(
            &mut s,
            Msg::Key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)),
        );
        assert_eq!(cmds, vec![Cmd::Quit]);
    }

    #[test]
    fn tab_cycles_through_the_five_tabs() {
        let mut s = state();
        press(&mut s, KeyCode::Char('2'));
        press(&mut s, KeyCode::Tab);
        assert_eq!(s.screen, Screen::Models);
        press(&mut s, KeyCode::Char('5'));
        press(&mut s, KeyCode::Tab);
        assert_eq!(s.screen, Screen::Chat);
        press(&mut s, KeyCode::Char('4'));
        press(&mut s, KeyCode::BackTab);
        assert_eq!(s.screen, Screen::Models);
    }

    #[test]
    fn help_overlay_opens_scrolls_and_closes() {
        let mut s = state();
        press(&mut s, KeyCode::Char('?'));
        assert_eq!(s.overlay, Some(Overlay::Help { scroll: 0 }));
        press(&mut s, KeyCode::Char('j'));
        assert_eq!(s.overlay, Some(Overlay::Help { scroll: 1 }));
        press(&mut s, KeyCode::Char('2'));
        assert_eq!(s.screen, Screen::Home);
        press(&mut s, KeyCode::Esc);
        assert_eq!(s.overlay, None);
    }

    #[test]
    fn first_run_checklist_opens_the_right_screen() {
        let mut s = state();
        assert!(s.snapshot.is_first_run());
        assert_eq!(s.home_focus, HomeFocus::Checklist);
        press(&mut s, KeyCode::Char('j'));
        assert_eq!(s.checklist_cursor, 1);
        press(&mut s, KeyCode::Enter);
        assert_eq!(s.screen, Screen::Models);
        press(&mut s, KeyCode::Char('q'));
        press(&mut s, KeyCode::Char('G'));
        press(&mut s, KeyCode::Enter);
        assert_eq!(s.screen, Screen::Import);
    }

    #[test]
    fn menu_pane_enter_activates_items() {
        let mut s = state();
        press(&mut s, KeyCode::Left);
        assert_eq!(s.home_focus, HomeFocus::Menu);
        press(&mut s, KeyCode::Char('G'));
        assert_eq!(s.menu_cursor, HOME_MENU.len() - 1);
        assert_eq!(press(&mut s, KeyCode::Enter), vec![Cmd::Quit]);
    }

    #[test]
    fn release_events_and_ticks_do_nothing_harmful() {
        let mut s = state();
        let mut release = KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE);
        release.kind = KeyEventKind::Release;
        assert!(update(&mut s, Msg::Key(release)).is_empty());
        assert!(!s.should_quit);
        s.show_toast("hello");
        for _ in 0..crate::state::TOAST_TICKS {
            update(&mut s, Msg::Tick);
        }
        assert_eq!(s.toast, None);
        assert_eq!(s.tick, u64::from(crate::state::TOAST_TICKS));
    }

    #[test]
    fn mouse_click_on_tab_row_switches_screen() {
        let mut s = state();
        let col = crate::view::tab_column(Screen::Models);
        let click = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: col,
            row: 1,
            modifiers: KeyModifiers::NONE,
        };
        update(&mut s, Msg::Mouse(click));
        assert_eq!(s.screen, Screen::Models);
    }

    #[test]
    fn chat_typing_enter_and_run_events() {
        let mut s = state();
        press(&mut s, KeyCode::Char('1'));
        assert_eq!(s.screen, Screen::Chat);
        for c in "pile capacity".chars() {
            press(&mut s, KeyCode::Char(c));
        }
        assert_eq!(s.chat.input, "pile capacity");
        // Single-letter screen keys are swallowed by the input once text is present.
        press(&mut s, KeyCode::Char('q'));
        assert_eq!(s.screen, Screen::Chat);
        assert_eq!(s.chat.input, "pile capacityq");
        press(&mut s, KeyCode::Backspace);
        let cmds = press(&mut s, KeyCode::Enter);
        assert_eq!(
            cmds,
            vec![Cmd::Ask {
                message: "pile capacity".into()
            }]
        );
        assert!(s.chat.input.is_empty());

        // Simulate the loop starting a run.
        s.chat.running = true;
        s.chat.run = Some(crate::screens::chat::RunView {
            question: "pile capacity".into(),
            ..Default::default()
        });
        update(
            &mut s,
            Msg::Chat(ChatEvent::Plan {
                summary: PlanSummary {
                    passes: 2,
                    kept: 5,
                    total_tokens: 100,
                    dropped: 0,
                    fallback: false,
                    budget: 60,
                    min_relevance: 0.65,
                },
            }),
        );
        update(
            &mut s,
            Msg::Chat(ChatEvent::PassStart {
                k: 1,
                sources_from: 1,
                sources_to: 3,
            }),
        );
        update(
            &mut s,
            Msg::Chat(ChatEvent::Token {
                k: Some(1),
                text: "Twelve".into(),
            }),
        );
        update(
            &mut s,
            Msg::Chat(ChatEvent::Token {
                k: Some(1),
                text: " metres".into(),
            }),
        );
        update(
            &mut s,
            Msg::Chat(ChatEvent::PassEnd {
                k: 1,
                status: PassStatus::Done,
                cut_off: false,
            }),
        );
        let run = s.chat.run.as_ref().unwrap();
        assert_eq!(run.passes[0].text, "Twelve metres");
        assert_eq!(run.passes[0].status, Some(PassStatus::Done));
        assert_eq!(run.plan.as_ref().unwrap().passes, 2);
        // q while running goes back to Home; a second q asks to confirm the quit.
        press(&mut s, KeyCode::Char('q'));
        assert_eq!(s.screen, Screen::Home);
        press(&mut s, KeyCode::Char('q'));
        assert!(matches!(s.overlay, Some(Overlay::Confirm { .. })));
        assert!(!s.should_quit);
        press(&mut s, KeyCode::Char('n'));
        assert_eq!(s.overlay, None);
    }

    #[test]
    fn text_input_overlay_commits_tags() {
        let mut s = state();
        press(&mut s, KeyCode::Char('1'));
        press(&mut s, KeyCode::Char('t'));
        assert!(matches!(s.overlay, Some(Overlay::TextInput { .. })));
        for c in "piling, slopes".chars() {
            press(&mut s, KeyCode::Char(c));
        }
        press(&mut s, KeyCode::Enter);
        assert_eq!(
            s.chat.tags,
            vec!["piling".to_string(), "slopes".to_string()]
        );
        assert_eq!(s.overlay, None);
    }

    #[test]
    fn models_screen_navigation_and_pull() {
        let mut s = state();
        press(&mut s, KeyCode::Char('3'));
        press(&mut s, KeyCode::Tab);
        assert_eq!(
            s.models.focus(),
            crate::screens::models::ModelsFocus::Catalog
        );
        press(&mut s, KeyCode::Char('/'));
        for c in "nomic".chars() {
            press(&mut s, KeyCode::Char(c));
        }
        press(&mut s, KeyCode::Enter);
        assert_eq!(s.models.catalog_query, "nomic");
        let cmds = press(&mut s, KeyCode::Enter);
        assert_eq!(
            cmds,
            vec![Cmd::PullModel {
                spec: "nomic-embed-text-v1.5".into()
            }]
        );
    }
}
