//! `update(state, msg) -> commands`: every state change lives here.

use crossterm::event::{
    KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};

use crate::state::{Cmd, HOME_MENU, HomeFocus, MenuItem, Msg, Overlay, Screen, State};
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
    }
}

fn on_key(state: &mut State, key: KeyEvent) -> Vec<Cmd> {
    // ctrl-c quits now, from anywhere; the daemon (P3) keeps running.
    if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
        return quit(state);
    }

    if let Some(overlay) = state.overlay.clone() {
        return on_overlay_key(state, overlay, key);
    }

    match key.code {
        KeyCode::Char('?') => {
            state.overlay = Some(Overlay::Help { scroll: 0 });
            Vec::new()
        }
        KeyCode::Char(c @ '1'..='5') => {
            let idx = (c as u8 - b'1') as usize;
            goto(state, Screen::TABS[idx])
        }
        KeyCode::Tab => {
            let next = match state.screen.tab_index() {
                Some(i) => Screen::TABS[(i + 1) % Screen::TABS.len()],
                None => Screen::TABS[0],
            };
            goto(state, next)
        }
        KeyCode::BackTab => {
            let n = Screen::TABS.len();
            let prev = match state.screen.tab_index() {
                Some(i) => Screen::TABS[(i + n - 1) % n],
                None => Screen::TABS[n - 1],
            };
            goto(state, prev)
        }
        KeyCode::Char('q') => {
            if state.screen == Screen::Home {
                quit(state)
            } else {
                goto(state, Screen::Home)
            }
        }
        KeyCode::Esc => Vec::new(),
        _ => match state.screen {
            Screen::Home => on_home_key(state, key),
            _ => Vec::new(),
        },
    }
}

fn on_overlay_key(state: &mut State, overlay: Overlay, key: KeyEvent) -> Vec<Cmd> {
    match overlay {
        Overlay::Help { scroll } => match key.code {
            KeyCode::Esc | KeyCode::Char('?') | KeyCode::Char('q') | KeyCode::Enter => {
                state.overlay = None;
            }
            KeyCode::Char('j') | KeyCode::Down => {
                state.overlay = Some(Overlay::Help {
                    scroll: scroll.saturating_add(1),
                });
            }
            KeyCode::Char('k') | KeyCode::Up => {
                state.overlay = Some(Overlay::Help {
                    scroll: scroll.saturating_sub(1),
                });
            }
            KeyCode::Char('g') => state.overlay = Some(Overlay::Help { scroll: 0 }),
            KeyCode::PageDown => {
                state.overlay = Some(Overlay::Help {
                    scroll: scroll.saturating_add(10),
                });
            }
            KeyCode::PageUp => {
                state.overlay = Some(Overlay::Help {
                    scroll: scroll.saturating_sub(10),
                });
            }
            _ => {}
        },
    }
    Vec::new()
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
    Vec::new()
}

fn quit(state: &mut State) -> Vec<Cmd> {
    if state.busy() {
        // A confirm overlay guards running passes and jobs from P1 on.
        state.show_toast("a run is in progress: press ctrl-c to quit now");
        return Vec::new();
    }
    state.should_quit = true;
    vec![Cmd::Quit]
}

#[cfg(test)]
mod tests {
    use super::*;
    use georag_core::status::Snapshot;
    use std::path::PathBuf;

    fn state() -> State {
        State::new(Snapshot::empty(PathBuf::from("/tmp/georag-test")), true)
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
        press(&mut s, KeyCode::Tab);
        assert_eq!(s.screen, Screen::Chat);
        for _ in 0..4 {
            press(&mut s, KeyCode::Tab);
        }
        assert_eq!(s.screen, Screen::Settings);
        press(&mut s, KeyCode::Tab);
        assert_eq!(s.screen, Screen::Chat);
        press(&mut s, KeyCode::BackTab);
        assert_eq!(s.screen, Screen::Settings);
    }

    #[test]
    fn help_overlay_opens_scrolls_and_closes() {
        let mut s = state();
        press(&mut s, KeyCode::Char('?'));
        assert_eq!(s.overlay, Some(Overlay::Help { scroll: 0 }));
        press(&mut s, KeyCode::Char('j'));
        assert_eq!(s.overlay, Some(Overlay::Help { scroll: 1 }));
        // Screen keys are swallowed while the overlay is open.
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
}
