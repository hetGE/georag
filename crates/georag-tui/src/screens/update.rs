//! Update screen (handoff 9.2 and 11.2): current and latest version,
//! release notes, download gauge, apply with confirmation, rollback.

use crossterm::event::{KeyCode, KeyEvent};
use georag_core::doctor::fmt_gib;
use georag_core::update::CheckReport;
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Gauge, Paragraph, Wrap};

use crate::state::{Cmd, ConfirmAction, Overlay, State};
use crate::theme::Theme;

#[derive(Debug, Clone, PartialEq)]
pub enum UpdatePhase {
    Idle,
    Checking,
    Checked,
    Downloading { bytes: u64, total: Option<u64> },
    Staged(std::path::PathBuf),
    Applying,
    Done(String),
    Failed(String),
}

#[derive(Debug, Clone)]
pub struct UpdateState {
    pub phase: UpdatePhase,
    pub report: Option<CheckReport>,
    pub notes: String,
    pub notes_scroll: u16,
}

impl Default for UpdateState {
    fn default() -> Self {
        Self {
            phase: UpdatePhase::Idle,
            report: None,
            notes: String::new(),
            notes_scroll: 0,
        }
    }
}

pub fn on_key(state: &mut State, key: KeyEvent) -> Vec<Cmd> {
    match key.code {
        KeyCode::Char('c') => vec![Cmd::UpdateCheck],
        KeyCode::Char('j') | KeyCode::Down => {
            state.update.notes_scroll = state.update.notes_scroll.saturating_add(1);
            Vec::new()
        }
        KeyCode::Char('k') | KeyCode::Up => {
            state.update.notes_scroll = state.update.notes_scroll.saturating_sub(1);
            Vec::new()
        }
        KeyCode::Enter => match &state.update.phase {
            UpdatePhase::Checked => match &state.update.report {
                Some(r) if r.newer && r.asset.is_some() => vec![Cmd::UpdateDownload],
                _ => {
                    state.show_toast("nothing to download: already up to date");
                    Vec::new()
                }
            },
            UpdatePhase::Staged(_) => {
                state.overlay = Some(Overlay::Confirm { text: "Apply the update now? GeoRAG restarts its engine and checks the new binary.".into(), on_yes: ConfirmAction::ApplyUpdate });
                Vec::new()
            }
            UpdatePhase::Idle => vec![Cmd::UpdateCheck],
            _ => Vec::new(),
        },
        KeyCode::Char('r') => {
            state.overlay = Some(Overlay::Confirm {
                text: "Put the previous version back from the .old copies?".into(),
                on_yes: ConfirmAction::RollbackUpdate,
            });
            Vec::new()
        }
        _ => Vec::new(),
    }
}

pub fn key_bar(state: &State) -> Vec<(&'static str, &'static str)> {
    match state.update.phase {
        UpdatePhase::Checked => vec![
            ("enter", "download"),
            ("c", "check again"),
            ("j k", "notes"),
            ("r", "rollback"),
            ("q", "back"),
        ],
        UpdatePhase::Staged(_) => vec![("enter", "apply"), ("r", "rollback"), ("q", "back")],
        _ => vec![
            ("c", "check"),
            ("enter", "check"),
            ("r", "rollback"),
            ("q", "back"),
        ],
    }
}

pub fn draw(frame: &mut Frame, area: Rect, state: &State, theme: &Theme) {
    let u = &state.update;
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(7), Constraint::Min(3)])
        .split(area);
    let mut lines: Vec<Line> = Vec::new();
    let current = state.snapshot.version;
    lines.push(Line::from(vec![
        Span::styled("current  ", theme.dim()),
        Span::raw(current.to_string()),
        Span::styled("   flavour ", theme.dim()),
        Span::raw(state.snapshot.flavour.to_string()),
    ]));
    match &u.report {
        Some(r) => {
            let verdict = if r.newer {
                "newer release available"
            } else {
                "up to date"
            };
            lines.push(Line::from(vec![
                Span::styled("latest   ", theme.dim()),
                Span::raw(format!("{} ({})  ", r.latest, r.channel)),
                Span::styled(
                    verdict.to_string(),
                    if r.newer {
                        theme.accent()
                    } else {
                        Style::default().fg(theme.ok)
                    },
                ),
            ]));
            if let Some(a) = &r.asset {
                lines.push(Line::from(vec![
                    Span::styled("asset    ", theme.dim()),
                    Span::raw(format!("{} ({})", a.file, fmt_gib(a.size))),
                ]));
            }
            lines.push(Line::from(vec![
                Span::styled("signed   ", theme.dim()),
                Span::raw(if r.signature_verified {
                    "yes".to_string()
                } else {
                    "not verified (no release key in this build)".to_string()
                }),
            ]));
        }
        None => lines.push(Line::styled("press c to check for a release", theme.dim())),
    }
    let phase = match &u.phase {
        UpdatePhase::Idle => String::new(),
        UpdatePhase::Checking => "checking…".into(),
        UpdatePhase::Checked => String::new(),
        UpdatePhase::Downloading { bytes, total } => format!(
            "downloading {} of {}",
            fmt_gib(*bytes),
            total.map(fmt_gib).unwrap_or_else(|| "?".into())
        ),
        UpdatePhase::Staged(p) => format!("verified and staged at {}", p.display()),
        UpdatePhase::Applying => "applying…".into(),
        UpdatePhase::Done(s) => s.clone(),
        UpdatePhase::Failed(e) => format!("failed: {e}"),
    };
    if !phase.is_empty() {
        lines.push(Line::styled(
            phase,
            if matches!(u.phase, UpdatePhase::Failed(_)) {
                Style::default().fg(theme.warn)
            } else {
                theme.accent()
            },
        ));
    }
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Update ")
        .border_style(theme.accent());
    let inner = block.inner(rows[0]);
    frame.render_widget(
        Paragraph::new(lines)
            .block(block)
            .wrap(Wrap { trim: false }),
        rows[0],
    );
    if let UpdatePhase::Downloading {
        bytes,
        total: Some(total),
    } = &u.phase
        && *total > 0
    {
        let ratio = (*bytes as f64 / *total as f64).clamp(0.0, 1.0);
        let gauge_area = Rect {
            x: inner.x,
            y: inner.y + inner.height.saturating_sub(1),
            width: inner.width,
            height: 1,
        };
        frame.render_widget(
            Gauge::default()
                .ratio(ratio)
                .gauge_style(Style::default().fg(theme.accent))
                .label(format!("{:.0}%", ratio * 100.0)),
            gauge_area,
        );
    }
    let notes = if u.notes.is_empty() {
        "Release notes appear here after a check.".to_string()
    } else {
        u.notes.clone()
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Release notes ")
        .border_style(theme.dim());
    frame.render_widget(
        Paragraph::new(notes)
            .block(block)
            .wrap(Wrap { trim: false })
            .scroll((u.notes_scroll, 0)),
        rows[1],
    );
}
