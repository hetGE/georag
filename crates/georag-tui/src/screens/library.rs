//! Library screen (handoff 9.2): facets, file table, detail strip and the
//! record view.

use std::collections::{HashMap, HashSet};

use crossterm::event::{KeyCode, KeyEvent};
use georag_core::{Chunk, FacetCounts, File, FileQuery, FileState, FileTag};
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap};

use crate::state::{Cmd, ConfirmAction, InputPurpose, Overlay, State};
use crate::theme::Theme;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LibraryFocus {
    Facets,
    Table,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortBy {
    Path,
    Modified,
    State,
}

impl SortBy {
    fn next(self) -> SortBy {
        match self {
            SortBy::Path => SortBy::Modified,
            SortBy::Modified => SortBy::State,
            SortBy::State => SortBy::Path,
        }
    }
    fn label(self) -> &'static str {
        match self {
            SortBy::Path => "path",
            SortBy::Modified => "modified",
            SortBy::State => "state",
        }
    }
}

/// One facet row the cursor can land on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Facet {
    Filter,
    State(String, i64),
    Tag(String, i64),
    Ext(String, i64),
    Clear,
}

#[derive(Debug, Clone)]
pub struct RecordView {
    pub file: File,
    pub tags: Vec<FileTag>,
    pub chunks: Vec<Chunk>,
    pub scroll: u16,
}

#[derive(Debug, Clone)]
pub struct LibraryState {
    pub files: Vec<File>,
    pub tags_by_file: HashMap<i64, Vec<String>>,
    pub facets: FacetCounts,
    pub query: FileQuery,
    pub cursor: usize,
    pub selected: HashSet<i64>,
    pub focus: Option<LibraryFocus>,
    pub facet_cursor: usize,
    pub show_facets: bool,
    pub sort: Option<SortBy>,
    pub record: Option<RecordView>,
    pub indexing: Option<String>,
}

impl Default for LibraryState {
    fn default() -> Self {
        Self {
            files: Vec::new(),
            tags_by_file: HashMap::new(),
            facets: FacetCounts::default(),
            query: FileQuery::default(),
            cursor: 0,
            selected: HashSet::new(),
            focus: None,
            facet_cursor: 0,
            show_facets: true,
            sort: None,
            record: None,
            indexing: None,
        }
    }
}

impl LibraryState {
    pub fn focus(&self) -> LibraryFocus {
        self.focus.unwrap_or(LibraryFocus::Table)
    }
    pub fn sort(&self) -> SortBy {
        self.sort.unwrap_or(SortBy::Path)
    }

    pub fn facet_rows(&self) -> Vec<Facet> {
        let mut rows = vec![Facet::Filter];
        for (s, n) in &self.facets.by_state {
            rows.push(Facet::State(s.clone(), *n));
        }
        for (t, n) in &self.facets.by_tag {
            rows.push(Facet::Tag(t.clone(), *n));
        }
        for (e, n) in &self.facets.by_ext {
            if !e.is_empty() {
                rows.push(Facet::Ext(e.clone(), *n));
            }
        }
        rows.push(Facet::Clear);
        rows
    }

    pub fn current(&self) -> Option<&File> {
        self.files.get(self.cursor)
    }

    /// Selected ids, or the highlighted file.
    pub fn targets(&self) -> Vec<i64> {
        if self.selected.is_empty() {
            self.current().map(|f| vec![f.id]).unwrap_or_default()
        } else {
            self.selected.iter().copied().collect()
        }
    }

    pub fn sorted(&mut self) {
        match self.sort() {
            SortBy::Path => self.files.sort_by(|a, b| a.rel_path.cmp(&b.rel_path)),
            SortBy::Modified => self.files.sort_by(|a, b| {
                b.mtime
                    .partial_cmp(&a.mtime)
                    .unwrap_or(std::cmp::Ordering::Equal)
            }),
            SortBy::State => self.files.sort_by(|a, b| {
                a.state
                    .as_str()
                    .cmp(b.state.as_str())
                    .then(a.rel_path.cmp(&b.rel_path))
            }),
        }
        if self.cursor >= self.files.len() {
            self.cursor = self.files.len().saturating_sub(1);
        }
    }
}

pub fn state_glyph(state: FileState) -> &'static str {
    match state {
        FileState::Indexed => "●",
        FileState::Extracted | FileState::Tagged => "◐",
        FileState::New | FileState::Imported | FileState::Stale => "○",
        FileState::Failed => "✕",
        FileState::Skipped => "–",
    }
}

pub fn on_key(state: &mut State, key: KeyEvent) -> Vec<Cmd> {
    if let Some(rec) = state.library.record.as_mut() {
        match key.code {
            KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q') => state.library.record = None,
            KeyCode::Char('j') | KeyCode::Down => rec.scroll = rec.scroll.saturating_add(1),
            KeyCode::Char('k') | KeyCode::Up => rec.scroll = rec.scroll.saturating_sub(1),
            KeyCode::Char('o') => {
                return vec![Cmd::OpenFile {
                    file_id: rec.file.id,
                }];
            }
            KeyCode::Char('r') => {
                let id = rec.file.id;
                state.library.record = None;
                return vec![Cmd::Reprocess { ids: vec![id] }];
            }
            _ => {}
        }
        return Vec::new();
    }
    let n = state.library.files.len();
    match (state.library.focus(), key.code) {
        (_, KeyCode::Char('f')) => {
            state.library.show_facets = !state.library.show_facets;
            if !state.library.show_facets {
                state.library.focus = Some(LibraryFocus::Table);
            }
            Vec::new()
        }
        (_, KeyCode::Tab) | (_, KeyCode::BackTab) => {
            state.library.focus = Some(match state.library.focus() {
                LibraryFocus::Table if state.library.show_facets => LibraryFocus::Facets,
                _ => LibraryFocus::Table,
            });
            Vec::new()
        }
        (_, KeyCode::Char('/')) => {
            state.overlay = Some(Overlay::TextInput {
                title: "Filter by path".into(),
                value: state.library.query.text.clone().unwrap_or_default(),
                purpose: InputPurpose::LibraryFilter,
            });
            Vec::new()
        }
        (_, KeyCode::Char('a')) => {
            state.overlay = Some(Overlay::TextInput {
                title: "Folder to add as a library root".into(),
                value: String::new(),
                purpose: InputPurpose::AddRoot,
            });
            Vec::new()
        }
        (_, KeyCode::Char('R')) => vec![Cmd::Rescan],
        (_, KeyCode::Char('o')) if state.library.focus() == LibraryFocus::Table => {
            match state.library.current() {
                Some(f) => vec![Cmd::OpenFile { file_id: f.id }],
                None => Vec::new(),
            }
        }
        (_, KeyCode::Char('O')) => {
            state.library.sort = Some(state.library.sort().next());
            state.library.sorted();
            Vec::new()
        }
        (LibraryFocus::Facets, KeyCode::Char('j') | KeyCode::Down) => {
            let rows = state.library.facet_rows().len();
            state.library.facet_cursor =
                (state.library.facet_cursor + 1).min(rows.saturating_sub(1));
            Vec::new()
        }
        (LibraryFocus::Facets, KeyCode::Char('k') | KeyCode::Up) => {
            state.library.facet_cursor = state.library.facet_cursor.saturating_sub(1);
            Vec::new()
        }
        (LibraryFocus::Facets, KeyCode::Enter) => {
            let rows = state.library.facet_rows();
            match rows.get(state.library.facet_cursor).cloned() {
                Some(Facet::Filter) => {
                    state.overlay = Some(Overlay::TextInput {
                        title: "Filter by path".into(),
                        value: state.library.query.text.clone().unwrap_or_default(),
                        purpose: InputPurpose::LibraryFilter,
                    });
                    Vec::new()
                }
                Some(Facet::State(s, _)) => {
                    let st = FileState::parse(&s);
                    state.library.query.state = if state.library.query.state == st {
                        None
                    } else {
                        st
                    };
                    vec![Cmd::RefreshLibrary]
                }
                Some(Facet::Tag(t, _)) => {
                    state.library.query.tag = if state.library.query.tag.as_deref() == Some(&t) {
                        None
                    } else {
                        Some(t)
                    };
                    vec![Cmd::RefreshLibrary]
                }
                Some(Facet::Ext(e, _)) => {
                    state.library.query.ext = if state.library.query.ext.as_deref() == Some(&e) {
                        None
                    } else {
                        Some(e)
                    };
                    vec![Cmd::RefreshLibrary]
                }
                Some(Facet::Clear) => {
                    state.library.query = FileQuery::default();
                    vec![Cmd::RefreshLibrary]
                }
                None => Vec::new(),
            }
        }
        (LibraryFocus::Table, KeyCode::Char('j') | KeyCode::Down) => {
            if n > 0 {
                state.library.cursor = (state.library.cursor + 1).min(n - 1);
            }
            Vec::new()
        }
        (LibraryFocus::Table, KeyCode::Char('k') | KeyCode::Up) => {
            state.library.cursor = state.library.cursor.saturating_sub(1);
            Vec::new()
        }
        (LibraryFocus::Table, KeyCode::PageDown) => {
            if n > 0 {
                state.library.cursor = (state.library.cursor + 20).min(n - 1);
            }
            Vec::new()
        }
        (LibraryFocus::Table, KeyCode::PageUp) => {
            state.library.cursor = state.library.cursor.saturating_sub(20);
            Vec::new()
        }
        (LibraryFocus::Table, KeyCode::Char('g')) => {
            state.library.cursor = 0;
            Vec::new()
        }
        (LibraryFocus::Table, KeyCode::Char('G')) => {
            state.library.cursor = n.saturating_sub(1);
            Vec::new()
        }
        (LibraryFocus::Table, KeyCode::Char(' ')) => {
            if let Some(f) = state.library.current() {
                let id = f.id;
                if !state.library.selected.remove(&id) {
                    state.library.selected.insert(id);
                }
                if n > 0 {
                    state.library.cursor = (state.library.cursor + 1).min(n - 1);
                }
            }
            Vec::new()
        }
        (LibraryFocus::Table, KeyCode::Enter) => match state.library.current() {
            Some(f) => vec![Cmd::OpenRecord { file_id: f.id }],
            None => Vec::new(),
        },
        (LibraryFocus::Table, KeyCode::Char('t')) => {
            let ids = state.library.targets();
            if ids.is_empty() {
                return Vec::new();
            }
            state.overlay = Some(Overlay::TextInput {
                title: format!("Tag {} file(s) with (name)", ids.len()),
                value: String::new(),
                purpose: InputPurpose::LibraryTag { ids },
            });
            Vec::new()
        }
        (LibraryFocus::Table, KeyCode::Char('T')) => {
            let ids = state.library.targets();
            if ids.is_empty() {
                return Vec::new();
            }
            state.overlay = Some(Overlay::TextInput {
                title: format!("Remove tag from {} file(s) (name)", ids.len()),
                value: String::new(),
                purpose: InputPurpose::LibraryUntag { ids },
            });
            Vec::new()
        }
        (LibraryFocus::Table, KeyCode::Char('r')) => {
            let ids = state.library.targets();
            if ids.is_empty() {
                return Vec::new();
            }
            state.overlay = Some(Overlay::Confirm {
                text: format!(
                    "Reprocess {} file(s)? Chunks and vectors are rebuilt.",
                    ids.len()
                ),
                on_yes: ConfirmAction::Reprocess { ids },
            });
            Vec::new()
        }
        (LibraryFocus::Table, KeyCode::Char('x')) => {
            state.library.selected.clear();
            Vec::new()
        }
        _ => Vec::new(),
    }
}

pub fn key_bar(state: &State) -> Vec<(&'static str, &'static str)> {
    if state.library.record.is_some() {
        return vec![
            ("j k", "scroll"),
            ("o", "open"),
            ("r", "reprocess"),
            ("esc", "close"),
        ];
    }
    match state.library.focus() {
        LibraryFocus::Facets => vec![
            ("enter", "apply"),
            ("f", "hide facets"),
            ("tab", "table"),
            ("q", "back"),
        ],
        LibraryFocus::Table => vec![
            ("space", "select"),
            ("enter", "record"),
            ("t T", "tag untag"),
            ("r", "reprocess"),
            ("o", "open"),
            ("a", "add root"),
            ("R", "rescan"),
            ("/", "filter"),
            ("O", "sort"),
            ("q", "back"),
        ],
    }
}

pub fn draw(frame: &mut Frame, area: Rect, state: &State, theme: &Theme) {
    let lib = &state.library;
    let narrow = area.width < crate::view::NARROW;
    let show_facets = lib.show_facets && !narrow;
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(5), Constraint::Length(4)])
        .split(area);
    let cols = if show_facets {
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(26), Constraint::Min(30)])
            .split(rows[0])
    } else {
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Min(30)])
            .split(rows[0])
    };
    if narrow && lib.focus() == LibraryFocus::Facets && lib.show_facets {
        draw_facets(frame, rows[0], state, theme);
    } else {
        let mut i = 0;
        if show_facets {
            draw_facets(frame, cols[0], state, theme);
            i = 1;
        }
        draw_table(frame, cols[i], state, theme);
    }
    draw_detail(frame, rows[1], state, theme);
    if let Some(rec) = &lib.record {
        draw_record(frame, area, rec, theme);
    }
}

fn draw_facets(frame: &mut Frame, area: Rect, state: &State, theme: &Theme) {
    let lib = &state.library;
    let focused = lib.focus() == LibraryFocus::Facets;
    let items: Vec<ListItem> = lib
        .facet_rows()
        .iter()
        .enumerate()
        .map(|(i, f)| {
            let marker = if focused && i == lib.facet_cursor {
                ">"
            } else {
                " "
            };
            let (text, active) = match f {
                Facet::Filter => (
                    format!("filter: {}", lib.query.text.as_deref().unwrap_or("")),
                    lib.query.text.is_some(),
                ),
                Facet::State(s, n) => (
                    format!(
                        "{} {s:<10} {n:>5}",
                        state_glyph(FileState::parse(s).unwrap_or(FileState::New))
                    ),
                    lib.query.state.map(|st| st.as_str() == s).unwrap_or(false),
                ),
                Facet::Tag(t, n) => (
                    format!("#{t:<11} {n:>5}"),
                    lib.query.tag.as_deref() == Some(t),
                ),
                Facet::Ext(e, n) => (
                    format!(".{e:<11} {n:>5}"),
                    lib.query.ext.as_deref() == Some(e),
                ),
                Facet::Clear => ("clear filters".to_string(), false),
            };
            let line = Line::from(vec![
                Span::raw(format!("{marker} ")),
                Span::styled(
                    text,
                    if active {
                        theme.accent()
                    } else {
                        Style::default()
                    },
                ),
            ]);
            if focused && i == lib.facet_cursor {
                ListItem::new(line).style(theme.selected())
            } else {
                ListItem::new(line)
            }
        })
        .collect();
    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" Facets ({}) ", lib.facets.total))
        .border_style(if focused { theme.accent() } else { theme.dim() });
    frame.render_widget(List::new(items).block(block), area);
}

fn draw_table(frame: &mut Frame, area: Rect, state: &State, theme: &Theme) {
    let lib = &state.library;
    let focused = lib.focus() == LibraryFocus::Table;
    let inner_h = area.height.saturating_sub(3) as usize;
    let first = lib.cursor.saturating_sub(inner_h.saturating_sub(1));
    let width = area.width.saturating_sub(2) as usize;
    let path_w = width.saturating_sub(40).max(16);
    let mut items: Vec<ListItem> = vec![ListItem::new(Line::styled(
        format!(
            "   {:<1} {:<path_w$} {:<5} {:>6} {:<10} tags",
            " ", "path", "type", "chunks", "modified"
        ),
        theme.dim(),
    ))];
    for (i, f) in lib.files.iter().enumerate().skip(first).take(inner_h) {
        let marker = if focused && i == lib.cursor { ">" } else { " " };
        let sel = if lib.selected.contains(&f.id) {
            "*"
        } else {
            " "
        };
        let tags = lib
            .tags_by_file
            .get(&f.id)
            .map(|t| t.join(","))
            .unwrap_or_default();
        let modified = f
            .mtime
            .map(|m| georag_core::status::short_date(m as u64))
            .unwrap_or_default();
        let path = truncate_middle(&f.rel_path, path_w);
        let glyph_style = match f.state {
            FileState::Indexed => Style::default().fg(theme.ok),
            FileState::Failed => Style::default().fg(theme.warn),
            _ => theme.dim(),
        };
        let line = Line::from(vec![
            Span::raw(format!("{marker}{sel} ")),
            Span::styled(state_glyph(f.state).to_string(), glyph_style),
            Span::raw(format!(
                " {path:<path_w$} {:<5} {:>6} {:<10} ",
                f.ext.as_deref().unwrap_or(""),
                f.chunk_count,
                modified
            )),
            Span::styled(tags, theme.accent()),
        ]);
        items.push(if focused && i == lib.cursor {
            ListItem::new(line).style(theme.selected())
        } else {
            ListItem::new(line)
        });
    }
    let title = format!(
        " Library · {} files · sort {} {}",
        lib.files.len(),
        lib.sort().label(),
        lib.indexing
            .as_deref()
            .map(|s| format!("· {s}"))
            .unwrap_or_default()
    );
    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .border_style(if focused { theme.accent() } else { theme.dim() });
    if lib.files.is_empty() {
        let hint = if state.snapshot.roots.is_empty() && lib.facets.total == 0 {
            "No library roots yet. Press a to add a folder."
        } else {
            "No files match. Press / to change the filter or clear the facets."
        };
        frame.render_widget(
            Paragraph::new(Line::styled(hint, theme.dim()))
                .block(block)
                .wrap(Wrap { trim: false }),
            area,
        );
        return;
    }
    frame.render_widget(List::new(items).block(block), area);
}

fn draw_detail(frame: &mut Frame, area: Rect, state: &State, theme: &Theme) {
    let lib = &state.library;
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(theme.dim());
    let lines: Vec<Line> = match lib.current() {
        Some(f) => {
            let tags = lib
                .tags_by_file
                .get(&f.id)
                .map(|t| t.join(", "))
                .unwrap_or_default();
            vec![
                Line::from(vec![
                    Span::styled(format!("{} ", state_glyph(f.state)), theme.accent()),
                    Span::raw(format!(
                        "{}  ·  {} chunks  ·  {} tokens  ·  {}",
                        f.state.as_str(),
                        f.chunk_count,
                        f.token_count,
                        tags
                    )),
                ]),
                Line::styled(
                    f.error
                        .clone()
                        .map(|e| format!("error: {e}"))
                        .unwrap_or_else(|| {
                            f.preview.clone().unwrap_or_default().replace('\n', " ")
                        }),
                    theme.dim(),
                ),
            ]
        }
        None => vec![Line::styled(
            "Select a file to see its details.",
            theme.dim(),
        )],
    };
    frame.render_widget(
        Paragraph::new(lines).block(block).wrap(Wrap { trim: true }),
        area,
    );
}

fn draw_record(frame: &mut Frame, area: Rect, rec: &RecordView, theme: &Theme) {
    let width = area.width.saturating_sub(4).clamp(30, 110);
    let height = area.height.saturating_sub(2).max(6);
    let rect = Rect {
        x: area.x + (area.width.saturating_sub(width)) / 2,
        y: area.y + 1,
        width,
        height,
    };
    frame.render_widget(Clear, rect);
    let f = &rec.file;
    let mut lines: Vec<Line> = vec![
        Line::from(vec![
            Span::styled("state     ", theme.dim()),
            Span::raw(format!("{} {}", state_glyph(f.state), f.state.as_str())),
        ]),
        Line::from(vec![
            Span::styled("size      ", theme.dim()),
            Span::raw(
                f.size_bytes
                    .map(|b| georag_core::doctor::fmt_gib(b as u64))
                    .unwrap_or_default(),
            ),
        ]),
        Line::from(vec![
            Span::styled("chunks    ", theme.dim()),
            Span::raw(format!("{} ({} tokens)", f.chunk_count, f.token_count)),
        ]),
    ];
    for t in &rec.tags {
        lines.push(Line::from(vec![
            Span::styled("tag       ", theme.dim()),
            Span::styled(t.tag.clone(), theme.accent()),
            Span::raw(format!(
                "  {}{}",
                t.source,
                t.confidence.map(|c| format!(" {c:.1}")).unwrap_or_default()
            )),
        ]));
    }
    if let Some(e) = &f.error {
        lines.push(Line::from(vec![
            Span::styled("error     ", theme.dim()),
            Span::styled(e.clone(), Style::default().fg(theme.warn)),
        ]));
        if e == "no_text" {
            lines.push(Line::styled(
                "This PDF has no text layer. OCR arrives in phase P4 (`georag ocr`).",
                theme.dim(),
            ));
        }
    }
    lines.push(Line::from(""));
    lines.push(Line::styled(
        format!("Chunks ({})", rec.chunks.len()),
        theme.accent(),
    ));
    for c in &rec.chunks {
        let snippet: String = c
            .text
            .chars()
            .take(90)
            .collect::<String>()
            .replace('\n', " ");
        lines.push(Line::from(vec![
            Span::styled(
                format!("{:<10} ", c.locator.as_deref().unwrap_or("")),
                theme.dim(),
            ),
            Span::raw(format!("{:>5} tok  {snippet}", c.tokens)),
        ]));
    }
    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" {} ", f.rel_path))
        .border_style(theme.accent());
    frame.render_widget(
        Paragraph::new(lines)
            .block(block)
            .wrap(Wrap { trim: false })
            .scroll((rec.scroll, 0)),
        rect,
    );
}

fn truncate_middle(s: &str, max: usize) -> String {
    let n = s.chars().count();
    if n <= max {
        return s.to_string();
    }
    let keep = max.saturating_sub(1);
    let head = keep / 3;
    let tail = keep - head;
    let h: String = s.chars().take(head).collect();
    let t: String = s.chars().skip(n - tail).collect();
    format!("{h}…{t}")
}
