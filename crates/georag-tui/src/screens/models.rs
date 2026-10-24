//! Models screen: installed table, catalogue with search or a pasted spec,
//! downloads with gauge, speed and ETA (handoff 9.2).

use crossterm::event::{KeyCode, KeyEvent};
use georag_core::doctor::fmt_gib;
use georag_core::models::catalog::CatalogEntry;
use georag_core::models::ops::ModelView;
use georag_core::models::{PullState, PullStatus};
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Gauge, List, ListItem, Paragraph, Wrap};

use crate::state::{Cmd, ConfirmAction, InputPurpose, Overlay, State};
use crate::theme::Theme;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelsFocus {
    Installed,
    Catalog,
}

#[derive(Debug, Clone, Default)]
pub struct ModelsState {
    pub installed: Vec<ModelView>,
    pub installed_cursor: usize,
    pub catalog_query: String,
    pub catalog_cursor: usize,
    pub pulls: Vec<PullStatus>,
    pub focus: Option<ModelsFocus>,
    /// Largest context that fits per installed chat model id, from the plan.
    pub fits: Vec<(String, Option<u32>)>,
    pub loaded_chat: Option<String>,
    pub loaded_embed: Option<String>,
}

impl ModelsState {
    pub fn focus(&self) -> ModelsFocus {
        self.focus.unwrap_or(ModelsFocus::Installed)
    }

    pub fn catalog(&self) -> Vec<&'static CatalogEntry> {
        georag_core::models::catalog::search(&self.catalog_query)
    }

    pub fn pull_for(&self, id: &str) -> Option<&PullStatus> {
        self.pulls.iter().find(|p| p.id == id)
    }
}

pub fn on_key(state: &mut State, key: KeyEvent) -> Vec<Cmd> {
    let focus = state.models.focus();
    match key.code {
        KeyCode::Tab | KeyCode::BackTab => {
            state.models.focus = Some(match focus {
                ModelsFocus::Installed => ModelsFocus::Catalog,
                ModelsFocus::Catalog => ModelsFocus::Installed,
            });
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
        KeyCode::Char('g') => {
            set_cursor(state, 0);
            Vec::new()
        }
        KeyCode::Char('G') => {
            set_cursor(state, usize::MAX);
            Vec::new()
        }
        KeyCode::Char('/') => {
            state.overlay = Some(Overlay::TextInput {
                title: "Search the catalogue, or paste org/repo[:file]".into(),
                value: state.models.catalog_query.clone(),
                purpose: InputPurpose::CatalogQuery,
            });
            Vec::new()
        }
        KeyCode::Enter => match focus {
            ModelsFocus::Catalog => {
                let entries = state.models.catalog();
                match entries.get(state.models.catalog_cursor) {
                    Some(e) => vec![Cmd::PullModel {
                        spec: e.id.to_string(),
                    }],
                    None if state.models.catalog_query.contains('/') => vec![Cmd::PullModel {
                        spec: state.models.catalog_query.clone(),
                    }],
                    None => Vec::new(),
                }
            }
            ModelsFocus::Installed => Vec::new(),
        },
        KeyCode::Char('p') => {
            let id = selected_installed_id(state).or_else(|| {
                state
                    .models
                    .catalog()
                    .get(state.models.catalog_cursor)
                    .map(|e| e.id.to_string())
            });
            match id {
                Some(id) => vec![Cmd::PausePull { id }],
                None => Vec::new(),
            }
        }
        KeyCode::Char('u') if focus == ModelsFocus::Installed => match selected_installed_id(state)
        {
            Some(id) => vec![Cmd::UseModel { id }],
            None => Vec::new(),
        },
        KeyCode::Char('l') => {
            state.overlay = Some(Overlay::TextInput { title: "Path to a GGUF file to link (role chat; prefix with embed: or vision: for other roles)".into(), value: String::new(), purpose: InputPurpose::LinkModel });
            Vec::new()
        }
        KeyCode::Char('r') if focus == ModelsFocus::Installed => match selected_installed_id(state)
        {
            Some(id) => {
                state.overlay = Some(Overlay::TextInput {
                    title: format!("Role for {id} (chat, embed or vision)"),
                    value: String::new(),
                    purpose: InputPurpose::SetRole { id },
                });
                Vec::new()
            }
            None => Vec::new(),
        },
        KeyCode::Char('d') if focus == ModelsFocus::Installed => match selected_installed_id(state)
        {
            Some(id) => {
                state.overlay = Some(Overlay::Confirm {
                    text: format!("Remove {id}? Managed files are deleted from models/."),
                    on_yes: ConfirmAction::RemoveModel { id },
                });
                Vec::new()
            }
            None => Vec::new(),
        },
        KeyCode::Char('v') if focus == ModelsFocus::Installed => match selected_installed_id(state)
        {
            Some(id) => vec![Cmd::VerifyModel { id }],
            None => Vec::new(),
        },
        _ => Vec::new(),
    }
}

fn selected_installed_id(state: &State) -> Option<String> {
    state
        .models
        .installed
        .get(state.models.installed_cursor)
        .map(|m| m.entry.id.clone())
}

fn move_cursor(state: &mut State, delta: i32) {
    let (cur, len) = match state.models.focus() {
        ModelsFocus::Installed => (state.models.installed_cursor, state.models.installed.len()),
        ModelsFocus::Catalog => (state.models.catalog_cursor, state.models.catalog().len()),
    };
    if len == 0 {
        return;
    }
    let next = (cur as i64 + delta as i64).clamp(0, len as i64 - 1) as usize;
    set_cursor(state, next);
}

fn set_cursor(state: &mut State, to: usize) {
    match state.models.focus() {
        ModelsFocus::Installed => {
            state.models.installed_cursor = to.min(state.models.installed.len().saturating_sub(1))
        }
        ModelsFocus::Catalog => {
            state.models.catalog_cursor = to.min(state.models.catalog().len().saturating_sub(1))
        }
    }
}

pub fn key_bar(state: &State) -> Vec<(&'static str, &'static str)> {
    match state.models.focus() {
        ModelsFocus::Installed => vec![
            ("u", "use"),
            ("r", "role"),
            ("v", "verify"),
            ("d", "remove"),
            ("l", "link"),
            ("p", "pause"),
            ("tab", "catalogue"),
            ("q", "back"),
        ],
        ModelsFocus::Catalog => vec![
            ("enter", "pull"),
            ("/", "search or paste"),
            ("p", "pause"),
            ("tab", "installed"),
            ("q", "back"),
        ],
    }
}

pub fn draw(frame: &mut Frame, area: Rect, state: &State, theme: &Theme) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(45), Constraint::Min(5)])
        .split(area);
    draw_installed(frame, rows[0], state, theme);
    draw_catalog(frame, rows[1], state, theme);
}

fn state_text(state: &State, m: &ModelView) -> (String, Style, Option<f64>) {
    let theme = &state.theme;
    if let Some(p) = state.models.pull_for(&m.entry.id) {
        return pull_text(p, theme);
    }
    if !m.present {
        return (
            "missing on disk".into(),
            Style::default().fg(theme.warn),
            None,
        );
    }
    let loaded = state.models.loaded_chat.as_deref() == Some(&m.entry.id)
        || state.models.loaded_embed.as_deref() == Some(&m.entry.id);
    if loaded {
        return ("loaded".into(), Style::default().fg(theme.ok), None);
    }
    if m.entry.in_use {
        return ("in use".into(), Style::default().fg(theme.ok), None);
    }
    match state.models.fits.iter().find(|(id, _)| id == &m.entry.id) {
        Some((_, Some(n))) => (format!("fits at {n} ctx"), Style::default(), None),
        Some((_, None)) => ("does not fit".into(), Style::default().fg(theme.warn), None),
        None => (String::new(), Style::default(), None),
    }
}

fn pull_text(p: &PullStatus, theme: &Theme) -> (String, Style, Option<f64>) {
    match &p.state {
        PullState::Queued => ("queued".into(), theme.dim(), None),
        PullState::Downloading => {
            let eta = p
                .eta_seconds
                .map(|s| format!(", {}m{:02}s left", s / 60, s % 60))
                .unwrap_or_default();
            (
                format!(
                    "{} / {} at {:.1} MB/s{eta}",
                    fmt_gib(p.bytes),
                    p.total.map(fmt_gib).unwrap_or_else(|| "?".into()),
                    p.bytes_per_second / 1e6
                ),
                Style::default(),
                p.fraction(),
            )
        }
        PullState::Paused => (
            format!("paused at {}", fmt_gib(p.bytes)),
            Style::default().fg(theme.warn),
            p.fraction(),
        ),
        PullState::Verifying => ("verifying sha256".into(), Style::default(), Some(1.0)),
        PullState::Done => ("downloaded".into(), Style::default().fg(theme.ok), None),
        PullState::Failed { error } => (
            format!("failed: {error}"),
            Style::default().fg(theme.warn),
            None,
        ),
    }
}

fn draw_installed(frame: &mut Frame, area: Rect, state: &State, theme: &Theme) {
    let focused = state.models.focus() == ModelsFocus::Installed;
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Installed ")
        .border_style(if focused { theme.accent() } else { theme.dim() });
    if state.models.installed.is_empty() {
        frame.render_widget(
            Paragraph::new(Line::styled("No models yet. Pull the defaults from the catalogue below: an embedding model and a chat model.", theme.dim())).block(block).wrap(Wrap { trim: false }),
            area,
        );
        return;
    }
    let header = Line::from(vec![Span::styled(
        format!(
            "  {:<7} {:<28} {:>9} {:>8}  {:<26} state",
            "role", "id", "size", "n_ctx", "where"
        ),
        theme.dim(),
    )]);
    let mut items: Vec<ListItem> = vec![ListItem::new(header)];
    for (i, m) in state.models.installed.iter().enumerate() {
        let marker = if focused && i == state.models.installed_cursor {
            ">"
        } else {
            " "
        };
        let flag = if m.entry.in_use { "*" } else { " " };
        let size = m
            .entry
            .size_bytes
            .map(fmt_gib)
            .unwrap_or_else(|| "?".into());
        let ctx = m
            .entry
            .n_ctx_train
            .map(|c| c.to_string())
            .unwrap_or_else(|| "?".into());
        let where_ = match m.entry.kind {
            georag_core::models::ModelKind::Managed => "managed".to_string(),
            georag_core::models::ModelKind::Linked => truncate(&m.entry.path, 26),
        };
        let (text, style, _) = state_text(state, m);
        let line = Line::from(vec![
            Span::raw(format!("{marker}{flag}{:<7} ", m.entry.role.as_str())),
            Span::styled(
                format!("{:<28} ", truncate(&m.entry.id, 28)),
                theme.accent(),
            ),
            Span::raw(format!("{size:>9} {ctx:>8}  {where_:<26} ")),
            Span::styled(text, style),
        ]);
        items.push(if focused && i == state.models.installed_cursor {
            ListItem::new(line).style(theme.selected())
        } else {
            ListItem::new(line)
        });
    }
    frame.render_widget(List::new(items).block(block), area);
}

fn draw_catalog(frame: &mut Frame, area: Rect, state: &State, theme: &Theme) {
    let focused = state.models.focus() == ModelsFocus::Catalog;
    let title = if state.models.catalog_query.is_empty() {
        " Catalogue ".to_string()
    } else {
        format!(" Catalogue /{} ", state.models.catalog_query)
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .border_style(if focused { theme.accent() } else { theme.dim() });
    let entries = state.models.catalog();
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if entries.is_empty() {
        let hint = if state.models.catalog_query.contains('/') {
            format!(
                "No catalogue match. Press enter to pull {} from Hugging Face.",
                state.models.catalog_query
            )
        } else {
            "No catalogue match. Paste org/repo[:file] with / to pull any GGUF.".to_string()
        };
        frame.render_widget(
            Paragraph::new(Line::styled(hint, theme.dim())).wrap(Wrap { trim: false }),
            inner,
        );
        return;
    }
    let free = state
        .snapshot
        .backend
        .as_ref()
        .map(|b| b.device.memory_free)
        .unwrap_or(0);
    // Two rows per entry; scroll so the cursor stays visible.
    let visible = (inner.height / 2).max(1) as usize;
    let first = state
        .models
        .catalog_cursor
        .saturating_sub(visible.saturating_sub(1));
    let mut y = inner.y;
    for (i, e) in entries.iter().enumerate().skip(first) {
        if y + 2 > inner.y + inner.height {
            break;
        }
        let selected = focused && i == state.models.catalog_cursor;
        let marker = if selected { ">" } else { " " };
        let installed = state.models.installed.iter().any(|m| m.entry.id == e.id);
        let pull = state.models.pull_for(e.id);
        let fit = if e.role == georag_core::models::ModelRole::Chat {
            if free > 0 && e.size_bytes + (1 << 30) + (2 << 30) <= free {
                "fits"
            } else if free > 0 {
                "tight or no fit"
            } else {
                ""
            }
        } else {
            ""
        };
        let status = match pull {
            Some(p) => pull_text(p, theme).0,
            None if installed => "installed".to_string(),
            None => fit.to_string(),
        };
        let line1 = Line::from(vec![
            Span::raw(format!("{marker} {:<7} ", e.role.as_str())),
            Span::styled(format!("{:<26} ", e.id), theme.accent()),
            Span::raw(format!("{:>9}  {}  ", fmt_gib(e.size_bytes), e.licence)),
            Span::styled(
                status,
                if installed {
                    Style::default().fg(theme.ok)
                } else {
                    Style::default()
                },
            ),
        ]);
        let line1 = if selected {
            line1.style(theme.selected())
        } else {
            line1
        };
        frame.render_widget(
            Paragraph::new(line1),
            Rect {
                x: inner.x,
                y,
                width: inner.width,
                height: 1,
            },
        );
        y += 1;
        match pull.and_then(|p| pull_text(p, theme).2) {
            Some(fraction) if y < inner.y + inner.height => {
                let gauge = Gauge::default()
                    .ratio(fraction.clamp(0.0, 1.0))
                    .gauge_style(Style::default().fg(theme.accent))
                    .label(format!("{:.0}%", fraction * 100.0));
                frame.render_widget(
                    gauge,
                    Rect {
                        x: inner.x + 12,
                        y,
                        width: inner.width.saturating_sub(14),
                        height: 1,
                    },
                );
                y += 1;
            }
            _ => {
                if y < inner.y + inner.height {
                    frame.render_widget(
                        Paragraph::new(Line::styled(
                            format!("           {}:{}  {}", e.repo, e.file, e.notes),
                            theme.dim(),
                        )),
                        Rect {
                            x: inner.x,
                            y,
                            width: inner.width,
                            height: 1,
                        },
                    );
                    y += 1;
                }
            }
        }
    }
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        let keep: String = s
            .chars()
            .rev()
            .take(n.saturating_sub(1))
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        format!("…{keep}")
    }
}
