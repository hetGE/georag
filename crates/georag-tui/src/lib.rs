//! Full-screen terminal UI (ratatui, Elm-style loop).
//!
//! The P0 chrome (status bar, tab row, key bar, help overlay, Home with the
//! first-run checklist) is the next P0 step after `doctor` is confirmed.

use georag_core::Home;

/// Errors from the TUI.
#[derive(Debug, thiserror::Error)]
pub enum TuiError {
    #[error("the full-screen UI is not built yet: use `georag doctor` and `georag init` for now")]
    NotBuilt,
}

/// Run the TUI against `home` until the user quits.
pub fn run(_home: &Home) -> Result<(), TuiError> {
    Err(TuiError::NotBuilt)
}
