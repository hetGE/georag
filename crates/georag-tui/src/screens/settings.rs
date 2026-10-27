//! Settings screen: every settings.toml key with inline edit and validation,
//! library roots, the tag file, update check and doctor.

use crossterm::event::{KeyCode, KeyEvent};
use georag_core::Root;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap};

use crate::state::{Cmd, ConfirmAction, InputPurpose, Overlay, State};
use crate::theme::Theme;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Row {
    Setting(String, String),
    Root(Root),
    AddRoot,
    TagFile(String),
    CheckUpdate,
    Doctor,
}

#[derive(Debug, Clone, Default)]
pub struct SettingsState {
    pub entries: Vec<(String, String)>,
    pub roots: Vec<Root>,
    pub cursor: usize,
    pub doctor_text: Option<String>,
    pub doctor_scroll: u16,
}

impl SettingsState {
    pub fn rows(&self, tags_path: &str) -> Vec<Row> {
        let mut rows: Vec<Row> = self
            .entries
            .iter()
            .map(|(k, v)| Row::Setting(k.clone(), v.clone()))
            .collect();
        rows.extend(self.roots.iter().cloned().map(Row::Root));
        rows.push(Row::AddRoot);
        rows.push(Row::TagFile(tags_path.to_string()));
        rows.push(Row::CheckUpdate);
        rows.push(Row::Doctor);
        rows
    }
}

pub fn on_key(state: &mut State, key: KeyEvent) -> Vec<Cmd> {
    if state.settings.doctor_text.is_some() {
        match key.code {
            KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q') => state.settings.doctor_text = None,
            KeyCode::Char('j') | KeyCode::Down => {
                state.settings.doctor_scroll = state.settings.doctor_scroll.saturating_add(1)
            }
            KeyCode::Char('k') | KeyCode::Up => {
                state.settings.doctor_scroll = state.settings.doctor_scroll.saturating_sub(1)
            }
            _ => {}
        }
        return Vec::new();
    }
    let tags_path = state
        .snapshot
        .home
        .join("config/tags.toml")
        .display()
        .to_string();
    let rows = state.settings.rows(&tags_path);
    match key.code {
        KeyCode::Char('j') | KeyCode::Down => {
            state.settings.cursor = (state.settings.cursor + 1).min(rows.len().saturating_sub(1));
            Vec::new()
        }
        KeyCode::Char('k') | KeyCode::Up => {
            state.settings.cursor = state.settings.cursor.saturating_sub(1);
            Vec::new()
        }
        KeyCode::Char('g') => {
            state.settings.cursor = 0;
            Vec::new()
        }
        KeyCode::Char('G') => {
            state.settings.cursor = rows.len().saturating_sub(1);
            Vec::new()
        }
        KeyCode::Char('a') => {
            state.overlay = Some(Overlay::TextInput {
                title: "Folder to add as a library root".into(),
                value: String::new(),
                purpose: InputPurpose::AddRoot,
            });
            Vec::new()
        }
        KeyCode::Char('u') => vec![Cmd::UpdateCheck, Cmd::GotoUpdate],
        KeyCode::Char('d') => vec![Cmd::RunDoctor],
        KeyCode::Enter => match rows.get(state.settings.cursor).cloned() {
            Some(Row::Setting(k, v)) => {
                state.overlay = Some(Overlay::TextInput {
                    title: format!("{k} ="),
                    value: v,
                    purpose: InputPurpose::SettingValue { key: k },
                });
                Vec::new()
            }
            Some(Row::Root(r)) => {
                state.overlay = Some(Overlay::Confirm {
                    text: format!("Remove root {:?} and its index entries?", r.name),
                    on_yes: ConfirmAction::RemoveRoot { name: r.name },
                });
                Vec::new()
            }
            Some(Row::AddRoot) => {
                state.overlay = Some(Overlay::TextInput {
                    title: "Folder to add as a library root".into(),
                    value: String::new(),
                    purpose: InputPurpose::AddRoot,
                });
                Vec::new()
            }
            Some(Row::TagFile(_)) => {
                state
                    .show_toast("edit config/tags.toml in a text editor; `georag tags` applies it");
                Vec::new()
            }
            Some(Row::CheckUpdate) => vec![Cmd::UpdateCheck, Cmd::GotoUpdate],
            Some(Row::Doctor) => vec![Cmd::RunDoctor],
            None => Vec::new(),
        },
        KeyCode::Char('x') => match rows.get(state.settings.cursor).cloned() {
            Some(Row::Root(r)) => {
                state.overlay = Some(Overlay::Confirm {
                    text: format!("Remove root {:?} and its index entries?", r.name),
                    on_yes: ConfirmAction::RemoveRoot { name: r.name },
                });
                Vec::new()
            }
            _ => Vec::new(),
        },
        _ => Vec::new(),
    }
}

pub fn key_bar(state: &State) -> Vec<(&'static str, &'static str)> {
    if state.settings.doctor_text.is_some() {
        return vec![("j k", "scroll"), ("esc", "close")];
    }
    vec![
        ("enter", "edit"),
        ("a", "add root"),
        ("x", "remove root"),
        ("u", "check update"),
        ("d", "doctor"),
        ("q", "back"),
    ]
}

pub fn draw(frame: &mut Frame, area: Rect, state: &State, theme: &Theme) {
    let tags_path = state
        .snapshot
        .home
        .join("config/tags.toml")
        .display()
        .to_string();
    let rows = state.settings.rows(&tags_path);
    let inner_h = area.height.saturating_sub(2) as usize;
    let first = state
        .settings
        .cursor
        .saturating_sub(inner_h.saturating_sub(1));
    let mut items: Vec<ListItem> = Vec::new();
    let mut last_section = String::new();
    for (i, row) in rows.iter().enumerate().skip(first).take(inner_h) {
        let marker = if i == state.settings.cursor { ">" } else { " " };
        let line = match row {
            Row::Setting(k, v) => {
                let section = k.split('.').next().unwrap_or("").to_string();
                let prefix = if section != last_section {
                    format!("[{section}] ")
                } else {
                    "          ".to_string()
                };
                last_section = section;
                Line::from(vec![
                    Span::raw(format!("{marker} ")),
                    Span::styled(prefix, theme.dim()),
                    Span::raw(format!("{:<26} ", k.split('.').nth(1).unwrap_or(k))),
                    Span::styled(v.clone(), theme.accent()),
                ])
            }
            Row::Root(r) => Line::from(vec![
                Span::raw(format!("{marker} ")),
                Span::styled("[root]    ", theme.dim()),
                Span::raw(format!("{:<26} ", r.name)),
                Span::styled(r.path.clone(), theme.accent()),
            ]),
            Row::AddRoot => Line::from(vec![
                Span::raw(format!("{marker} ")),
                Span::styled("[root]    ", theme.dim()),
                Span::raw("add a library root"),
            ]),
            Row::TagFile(p) => Line::from(vec![
                Span::raw(format!("{marker} ")),
                Span::styled("[tags]    ", theme.dim()),
                Span::raw(format!("{:<26} ", "tag file")),
                Span::styled(p.clone(), theme.accent()),
            ]),
            Row::CheckUpdate => Line::from(vec![
                Span::raw(format!("{marker} ")),
                Span::styled("[action]  ", theme.dim()),
                Span::raw("re-check update now"),
            ]),
            Row::Doctor => Line::from(vec![
                Span::raw(format!("{marker} ")),
                Span::styled("[action]  ", theme.dim()),
                Span::raw("run doctor"),
            ]),
        };
        items.push(if i == state.settings.cursor {
            ListItem::new(line).style(theme.selected())
        } else {
            ListItem::new(line)
        });
    }
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Settings ")
        .border_style(theme.accent());
    frame.render_widget(List::new(items).block(block), area);
    if let Some(text) = &state.settings.doctor_text {
        let width = area.width.saturating_sub(4).clamp(30, 110);
        let height = area.height.saturating_sub(2).max(6);
        let rect = Rect {
            x: area.x + (area.width.saturating_sub(width)) / 2,
            y: area.y + 1,
            width,
            height,
        };
        frame.render_widget(Clear, rect);
        let block = Block::default()
            .borders(Borders::ALL)
            .title(" Doctor ")
            .border_style(theme.accent());
        frame.render_widget(
            Paragraph::new(text.clone())
                .block(block)
                .wrap(Wrap { trim: false })
                .scroll((state.settings.doctor_scroll, 0)),
            rect,
        );
    }
}
