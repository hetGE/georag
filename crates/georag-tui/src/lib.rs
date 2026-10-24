//! Full-screen terminal UI: ratatui and crossterm in an Elm-style loop.
//!
//! `update` is the only place state changes; `view` renders it; `app` runs
//! the loop over terminal events, a 250 ms tick and messages from
//! background work (model loading, chat runs, pulls).

pub mod app;
pub mod screens;
pub mod state;
pub mod theme;
pub mod update;
pub mod view;

use georag_core::config::Settings;
use georag_core::{Home, status::Snapshot};

/// Errors from the TUI.
#[derive(Debug, thiserror::Error)]
pub enum TuiError {
    #[error(
        "the terminal could not be set up ({0}): run GeoRAG from a terminal, or use the subcommands"
    )]
    Terminal(#[source] std::io::Error),
    #[error("the UI loop failed ({0})")]
    Loop(#[source] std::io::Error),
    #[error(
        "stdout is not a terminal: run `georag` from a terminal, or use subcommands such as `georag doctor`"
    )]
    NotATerminal,
}

/// Run the TUI against `home` until the user quits. The backend is probed
/// before the alternate screen opens; models load in the background.
pub fn run(home: &Home, settings: &Settings) -> Result<(), TuiError> {
    use std::io::IsTerminal;
    if !std::io::stdout().is_terminal() {
        return Err(TuiError::NotATerminal);
    }
    let snapshot = Snapshot::take(home, settings);
    let theme = theme::Theme::from_setting(&settings.tui.color);
    let state = state::State::new(snapshot, settings.tui.mouse, theme);
    app::run(home.clone(), state, theme)
}
