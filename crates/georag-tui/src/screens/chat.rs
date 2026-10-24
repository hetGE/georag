//! Chat screen: conversations list, transcript with the pass rail, sources
//! pane and input box (handoff 9.2).

use std::sync::Arc;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use georag_core::Source;
use georag_core::chat::{ChatEvent, Controls, Mode, PassStatus, PlanSummary, RunControl, Tier};
use georag_core::conversations::ConversationView;
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap};
use unicode_width::UnicodeWidthStr;

use crate::state::{Cmd, Msg, Overlay, State};
use crate::theme::Theme;

#[derive(Debug, Clone, PartialEq)]
pub struct PassView {
    pub k: u16,
    pub sources_from: u32,
    pub sources_to: u32,
    pub status: Option<PassStatus>,
    pub text: String,
    pub cut_off: bool,
}

/// A run in progress or just finished, as shown in the transcript.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RunView {
    pub question: String,
    pub plan: Option<PlanSummary>,
    pub passes: Vec<PassView>,
    pub final_text: String,
    pub final_started: bool,
    pub done: bool,
    pub sources: Vec<Source>,
    pub cut_off: bool,
    pub error: Option<String>,
}

impl RunView {
    pub fn current_pass(&self) -> Option<u16> {
        if self.final_started && !self.done {
            return Some(0);
        }
        self.passes
            .iter()
            .rev()
            .find(|p| p.status.is_none())
            .map(|p| p.k)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChatFocus {
    Conversations,
    Transcript,
    Sources,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Depth {
    Auto,
    Manual(Tier),
}

impl Depth {
    pub fn next(self) -> Depth {
        match self {
            Depth::Auto => Depth::Manual(Tier::Quick),
            Depth::Manual(Tier::Quick) => Depth::Manual(Tier::Optimal),
            Depth::Manual(Tier::Optimal) => Depth::Manual(Tier::Deep),
            Depth::Manual(Tier::Deep) => Depth::Manual(Tier::Deeper),
            Depth::Manual(Tier::Deeper) => Depth::Manual(Tier::Ludicrous),
            Depth::Manual(Tier::Ludicrous) => Depth::Auto,
        }
    }

    pub fn label(self) -> String {
        match self {
            Depth::Auto => "auto".into(),
            Depth::Manual(t) => format!("manual {t}"),
        }
    }
}

#[derive(Debug)]
pub struct ChatState {
    pub conversations: Vec<georag_core::Conversation>,
    pub conv_cursor: usize,
    pub conv_filter: String,
    pub show_conversations: bool,
    pub show_sources: bool,
    pub focus: ChatFocus,
    pub current: Option<ConversationView>,
    pub run: Option<RunView>,
    pub running: bool,
    pub controls: Option<Controls>,
    pub input: String,
    pub tags: Vec<String>,
    pub depth: Depth,
    pub transcript_scroll: u16,
    pub source_cursor: usize,
    /// Which pass's sources the pane shows; None = all.
    pub source_pass: Option<u16>,
    pub follow: bool,
    /// The background run, joined on quit so the engine is torn down cleanly.
    pub thread: Option<std::thread::JoinHandle<()>>,
}

impl Default for ChatState {
    fn default() -> Self {
        Self {
            conversations: Vec::new(),
            conv_cursor: 0,
            conv_filter: String::new(),
            show_conversations: true,
            show_sources: true,
            focus: ChatFocus::Transcript,
            current: None,
            run: None,
            running: false,
            controls: None,
            input: String::new(),
            tags: Vec::new(),
            depth: Depth::Auto,
            transcript_scroll: 0,
            source_cursor: 0,
            source_pass: None,
            follow: true,
            thread: None,
        }
    }
}

impl ChatState {
    pub fn apply_event(&mut self, ev: ChatEvent) {
        let Some(run) = self.run.as_mut() else { return };
        match ev {
            ChatEvent::Plan { summary } => run.plan = Some(summary),
            ChatEvent::PassStart {
                k,
                sources_from,
                sources_to,
            } => {
                run.passes.push(PassView {
                    k,
                    sources_from,
                    sources_to,
                    status: None,
                    text: String::new(),
                    cut_off: false,
                });
            }
            ChatEvent::Token { k, text } => match k {
                Some(0) => run.final_text.push_str(&text),
                Some(k) => {
                    if let Some(p) = run.passes.iter_mut().find(|p| p.k == k) {
                        p.text.push_str(&text);
                    }
                }
                None => run.final_text.push_str(&text),
            },
            ChatEvent::PassEnd { k, status, cut_off } => {
                if let Some(p) = run.passes.iter_mut().find(|p| p.k == k) {
                    p.status = Some(status);
                    p.cut_off = cut_off;
                }
            }
            ChatEvent::FinalStart => run.final_started = true,
            ChatEvent::Done {
                sources, cut_off, ..
            } => {
                run.done = true;
                run.sources = sources;
                run.cut_off = cut_off;
            }
            ChatEvent::Error { message } => {
                run.error = Some(message);
                run.done = true;
            }
        }
    }

    pub fn visible_sources(&self) -> Vec<Source> {
        let Some(run) = &self.run else {
            return Vec::new();
        };
        match self.source_pass {
            None => run.sources.clone(),
            Some(k) => {
                let Some(p) = run.passes.iter().find(|p| p.k == k) else {
                    return Vec::new();
                };
                run.sources
                    .iter()
                    .filter(|s| s.tag >= p.sources_from && s.tag <= p.sources_to)
                    .cloned()
                    .collect()
            }
        }
    }

    pub fn filtered_conversations(&self) -> Vec<&georag_core::Conversation> {
        let f = self.conv_filter.to_ascii_lowercase();
        self.conversations
            .iter()
            .filter(|c| {
                f.is_empty()
                    || c.title
                        .as_deref()
                        .unwrap_or("")
                        .to_ascii_lowercase()
                        .contains(&f)
            })
            .collect()
    }
}

/// Keys on the Chat screen. Returns commands for the loop.
pub fn on_key(state: &mut State, key: KeyEvent) -> Vec<Cmd> {
    let running = state.chat.running;
    // Run controls first.
    if running {
        match key.code {
            KeyCode::Esc => {
                if let Some(c) = &state.chat.controls {
                    c.request(RunControl::StopAll);
                }
                state.show_toast("stopping the run");
                return Vec::new();
            }
            KeyCode::Char('s')
                if state.chat.focus != ChatFocus::Transcript || state.chat.input.is_empty() =>
            {
                if let Some(c) = &state.chat.controls {
                    c.request(RunControl::SkipPass);
                }
                return Vec::new();
            }
            KeyCode::Char('a')
                if state.chat.focus != ChatFocus::Transcript || state.chat.input.is_empty() =>
            {
                if let Some(c) = &state.chat.controls {
                    c.request(RunControl::AcceptPass);
                }
                return Vec::new();
            }
            _ => {}
        }
    }
    match state.chat.focus {
        ChatFocus::Transcript => on_transcript_key(state, key),
        ChatFocus::Conversations => on_conversations_key(state, key),
        ChatFocus::Sources => on_sources_key(state, key),
    }
}

fn on_transcript_key(state: &mut State, key: KeyEvent) -> Vec<Cmd> {
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    match key.code {
        KeyCode::Enter if alt || (ctrl && key.code == KeyCode::Char('j')) => {
            state.chat.input.push('\n');
            Vec::new()
        }
        KeyCode::Char('j') if ctrl => {
            state.chat.input.push('\n');
            Vec::new()
        }
        KeyCode::Enter => {
            let text = state.chat.input.trim().to_string();
            if text.is_empty() || state.chat.running {
                return Vec::new();
            }
            state.chat.input.clear();
            vec![Cmd::Ask { message: text }]
        }
        KeyCode::Backspace => {
            state.chat.input.pop();
            Vec::new()
        }
        KeyCode::Char('c') if ctrl => Vec::new(),
        KeyCode::Char(ch) if !ctrl && !alt => {
            state.chat.input.push(ch);
            Vec::new()
        }
        KeyCode::Tab => {
            state.chat.focus = if state.chat.show_sources {
                ChatFocus::Sources
            } else if state.chat.show_conversations {
                ChatFocus::Conversations
            } else {
                ChatFocus::Transcript
            };
            Vec::new()
        }
        KeyCode::BackTab => {
            state.chat.focus = if state.chat.show_conversations {
                ChatFocus::Conversations
            } else {
                ChatFocus::Transcript
            };
            Vec::new()
        }
        KeyCode::Up | KeyCode::PageUp => {
            state.chat.follow = false;
            state.chat.transcript_scroll = state
                .chat
                .transcript_scroll
                .saturating_sub(if key.code == KeyCode::PageUp { 10 } else { 1 });
            Vec::new()
        }
        KeyCode::Down | KeyCode::PageDown => {
            state.chat.transcript_scroll = state
                .chat
                .transcript_scroll
                .saturating_add(if key.code == KeyCode::PageDown { 10 } else { 1 });
            Vec::new()
        }
        KeyCode::End => {
            state.chat.follow = true;
            Vec::new()
        }
        KeyCode::F(2) => {
            state.chat.depth = state.chat.depth.next();
            Vec::new()
        }
        _ => Vec::new(),
    }
}

/// Keys that act on the screen rather than the input box: handled by the
/// global dispatcher when the input is empty.
pub fn on_screen_key(state: &mut State, key: KeyEvent) -> Option<Vec<Cmd>> {
    if state.chat.focus == ChatFocus::Transcript && !state.chat.input.is_empty() {
        return None;
    }
    match key.code {
        KeyCode::Char('n') => {
            state.chat.current = None;
            state.chat.run = None;
            state.chat.focus = ChatFocus::Transcript;
            Some(Vec::new())
        }
        KeyCode::Char('c') => {
            state.chat.show_conversations = !state.chat.show_conversations;
            if !state.chat.show_conversations && state.chat.focus == ChatFocus::Conversations {
                state.chat.focus = ChatFocus::Transcript;
            }
            Some(Vec::new())
        }
        KeyCode::Char('S') => {
            state.chat.show_sources = !state.chat.show_sources;
            if !state.chat.show_sources && state.chat.focus == ChatFocus::Sources {
                state.chat.focus = ChatFocus::Transcript;
            }
            Some(Vec::new())
        }
        KeyCode::Char('d') => {
            state.chat.depth = state.chat.depth.next();
            state.show_toast(format!("depth: {}", state.chat.depth.label()));
            Some(Vec::new())
        }
        KeyCode::Char('t') => {
            state.overlay = Some(Overlay::TextInput {
                title: "Tags (comma separated, empty for all)".into(),
                value: state.chat.tags.join(", "),
                purpose: crate::state::InputPurpose::ChatTags,
            });
            Some(Vec::new())
        }
        KeyCode::Char('e') => Some(vec![Cmd::ExportConversation]),
        KeyCode::Char('/') if state.chat.focus == ChatFocus::Conversations => {
            state.overlay = Some(Overlay::TextInput {
                title: "Find conversation".into(),
                value: state.chat.conv_filter.clone(),
                purpose: crate::state::InputPurpose::ConversationFilter,
            });
            Some(Vec::new())
        }
        KeyCode::Char(c @ '1'..='4') if state.chat.focus == ChatFocus::Sources => {
            state.chat.source_pass = Some((c as u8 - b'0') as u16);
            state.chat.source_cursor = 0;
            Some(Vec::new())
        }
        KeyCode::Char('0') if state.chat.focus == ChatFocus::Sources => {
            state.chat.source_pass = None;
            Some(Vec::new())
        }
        _ => None,
    }
}

fn on_conversations_key(state: &mut State, key: KeyEvent) -> Vec<Cmd> {
    let n = state.chat.filtered_conversations().len();
    match key.code {
        KeyCode::Char('j') | KeyCode::Down => {
            if n > 0 {
                state.chat.conv_cursor = (state.chat.conv_cursor + 1).min(n - 1);
            }
            Vec::new()
        }
        KeyCode::Char('k') | KeyCode::Up => {
            state.chat.conv_cursor = state.chat.conv_cursor.saturating_sub(1);
            Vec::new()
        }
        KeyCode::Char('g') => {
            state.chat.conv_cursor = 0;
            Vec::new()
        }
        KeyCode::Char('G') => {
            state.chat.conv_cursor = n.saturating_sub(1);
            Vec::new()
        }
        KeyCode::Enter => {
            let id = state
                .chat
                .filtered_conversations()
                .get(state.chat.conv_cursor)
                .map(|c| c.id);
            match id {
                Some(id) => vec![Cmd::OpenConversation { id }],
                None => Vec::new(),
            }
        }
        KeyCode::Char('x') => {
            let id = state
                .chat
                .filtered_conversations()
                .get(state.chat.conv_cursor)
                .map(|c| c.id);
            match id {
                Some(id) => {
                    state.overlay = Some(Overlay::Confirm {
                        text: format!("Move conversation {id} to trash?"),
                        on_yes: crate::state::ConfirmAction::DeleteConversation { id },
                    });
                    Vec::new()
                }
                None => Vec::new(),
            }
        }
        KeyCode::Tab => {
            state.chat.focus = ChatFocus::Transcript;
            Vec::new()
        }
        KeyCode::BackTab => {
            state.chat.focus = if state.chat.show_sources {
                ChatFocus::Sources
            } else {
                ChatFocus::Transcript
            };
            Vec::new()
        }
        _ => Vec::new(),
    }
}

fn on_sources_key(state: &mut State, key: KeyEvent) -> Vec<Cmd> {
    let n = state.chat.visible_sources().len();
    match key.code {
        KeyCode::Char('j') | KeyCode::Down => {
            if n > 0 {
                state.chat.source_cursor = (state.chat.source_cursor + 1).min(n - 1);
            }
            Vec::new()
        }
        KeyCode::Char('k') | KeyCode::Up => {
            state.chat.source_cursor = state.chat.source_cursor.saturating_sub(1);
            Vec::new()
        }
        KeyCode::Enter => match state.chat.visible_sources().get(state.chat.source_cursor) {
            Some(s) => vec![Cmd::ViewChunk {
                chunk_id: s.chunk_id,
            }],
            None => Vec::new(),
        },
        KeyCode::Char('o') => match state.chat.visible_sources().get(state.chat.source_cursor) {
            Some(s) => vec![Cmd::OpenFile { file_id: s.file_id }],
            None => Vec::new(),
        },
        KeyCode::Tab => {
            state.chat.focus = if state.chat.show_conversations {
                ChatFocus::Conversations
            } else {
                ChatFocus::Transcript
            };
            Vec::new()
        }
        KeyCode::BackTab => {
            state.chat.focus = ChatFocus::Transcript;
            Vec::new()
        }
        _ => Vec::new(),
    }
}

pub fn key_bar(state: &State) -> Vec<(&'static str, &'static str)> {
    if state.chat.running {
        return vec![
            ("esc", "stop all"),
            ("s", "skip pass"),
            ("a", "accept pass"),
            ("tab", "pane"),
            ("?", "help"),
        ];
    }
    match state.chat.focus {
        ChatFocus::Transcript => vec![
            ("enter", "send"),
            ("alt-enter", "newline"),
            ("d", "depth"),
            ("t", "tags"),
            ("n", "new"),
            ("c S", "panes"),
            ("e", "export"),
            ("q", "back"),
        ],
        ChatFocus::Conversations => vec![
            ("enter", "open"),
            ("n", "new"),
            ("x", "delete"),
            ("/", "find"),
            ("tab", "pane"),
            ("q", "back"),
        ],
        ChatFocus::Sources => vec![
            ("enter", "view chunk"),
            ("o", "open file"),
            ("1-4", "pass"),
            ("0", "all"),
            ("tab", "pane"),
            ("q", "back"),
        ],
    }
}

pub fn draw(frame: &mut Frame, area: Rect, state: &State, theme: &Theme) {
    let chat = &state.chat;
    let narrow = area.width < crate::view::NARROW;
    let compact = area.width < crate::view::COMPACT;
    let show_conv = chat.show_conversations
        && !narrow
        && !(compact && chat.show_sources && chat.focus != ChatFocus::Conversations);
    let show_src = chat.show_sources
        && !narrow
        && !(compact && show_conv && chat.focus == ChatFocus::Conversations);
    let mut constraints = Vec::new();
    if show_conv {
        constraints.push(Constraint::Length(26));
    }
    constraints.push(Constraint::Min(30));
    if show_src {
        constraints.push(Constraint::Length(34));
    }
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(constraints)
        .split(area);
    let mut i = 0;
    if narrow {
        match chat.focus {
            ChatFocus::Conversations => draw_conversations(frame, area, state, theme),
            ChatFocus::Sources => draw_sources(frame, area, state, theme),
            ChatFocus::Transcript => draw_main(frame, area, state, theme),
        }
        return;
    }
    if show_conv {
        draw_conversations(frame, cols[i], state, theme);
        i += 1;
    }
    draw_main(frame, cols[i], state, theme);
    i += 1;
    if show_src {
        draw_sources(frame, cols[i], state, theme);
    }
}

fn draw_conversations(frame: &mut Frame, area: Rect, state: &State, theme: &Theme) {
    let chat = &state.chat;
    let focused = chat.focus == ChatFocus::Conversations;
    let items: Vec<ListItem> = chat
        .filtered_conversations()
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let marker = if focused && i == chat.conv_cursor {
                ">"
            } else {
                " "
            };
            let title = c.title.as_deref().unwrap_or("(untitled)");
            let current = chat
                .current
                .as_ref()
                .map(|v| v.conversation.id == c.id)
                .unwrap_or(false);
            let line = Line::from(vec![
                Span::raw(format!("{marker} ")),
                Span::styled(
                    title.to_string(),
                    if current {
                        theme.accent()
                    } else {
                        Style::default()
                    },
                ),
            ]);
            if focused && i == chat.conv_cursor {
                ListItem::new(line).style(theme.selected())
            } else {
                ListItem::new(line)
            }
        })
        .collect();
    let title = if chat.conv_filter.is_empty() {
        " Conversations ".to_string()
    } else {
        format!(" Conversations /{} ", chat.conv_filter)
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .border_style(if focused { theme.accent() } else { theme.dim() });
    frame.render_widget(List::new(items).block(block), area);
}

fn draw_main(frame: &mut Frame, area: Rect, state: &State, theme: &Theme) {
    let chat = &state.chat;
    let input_lines = chat.input.lines().count().max(1) as u16 + 2;
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(3),
            Constraint::Length(input_lines.min(6)),
        ])
        .split(area);

    // Mode line.
    let tags = if chat.tags.is_empty() {
        "all".to_string()
    } else {
        chat.tags.join(",")
    };
    let model = state
        .snapshot
        .models
        .chat
        .clone()
        .unwrap_or_else(|| "no model".into());
    let mode = Line::from(vec![
        Span::styled(" tags ", theme.dim()),
        Span::raw(tags),
        Span::styled(" [t]  depth ", theme.dim()),
        Span::raw(chat.depth.label()),
        Span::styled(" [d]  model ", theme.dim()),
        Span::raw(model),
    ]);
    frame.render_widget(Paragraph::new(mode), rows[0]);

    // Transcript.
    let mut lines: Vec<Line> = Vec::new();
    if let Some(view) = &chat.current {
        for m in &view.messages {
            push_message(
                &mut lines,
                &m.message.role,
                &m.message.content,
                &m.message.sources,
                theme,
            );
        }
    }
    if let Some(run) = &chat.run {
        push_message(&mut lines, "user", &run.question, &[], theme);
        if let Some(plan) = &run.plan {
            lines.push(rail_line(run, plan, theme));
        }
        if run.passes.len() > 1 || run.final_started {
            for p in &run.passes {
                let status = match p.status {
                    Some(s) => s.as_str().to_string(),
                    None => "running".to_string(),
                };
                lines.push(Line::styled(
                    format!(
                        "Pass {} (sources {} to {}): {status}{}",
                        p.k,
                        p.sources_from,
                        p.sources_to,
                        if p.cut_off { ", cut off" } else { "" }
                    ),
                    theme.accent(),
                ));
                push_markdown(&mut lines, &p.text, &run.sources, theme);
                lines.push(Line::from(""));
            }
            if run.final_started {
                lines.push(Line::styled("Final answer", theme.accent()));
            }
        } else if let Some(p) = run.passes.first() {
            push_markdown(&mut lines, &p.text, &run.sources, theme);
        }
        if run.final_started || run.passes.is_empty() {
            push_markdown(&mut lines, &run.final_text, &run.sources, theme);
        }
        if run.cut_off {
            lines.push(Line::styled(
                "[The answer was cut off at the context limit.]",
                Style::default().fg(theme.warn),
            ));
        }
        if let Some(e) = &run.error {
            lines.push(Line::styled(
                format!("error: {e}"),
                Style::default().fg(theme.warn),
            ));
        }
        if !run.done && chat.running {
            lines.push(Line::styled("…", theme.dim()));
        }
    }
    if lines.is_empty() {
        lines.push(Line::styled("Ask a question below. Auto depth reads every source above the relevance threshold, in as many passes as memory needs.", theme.dim()));
    }
    let focused = chat.focus == ChatFocus::Transcript;
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Chat ")
        .border_style(if focused { theme.accent() } else { theme.dim() });
    let inner_height = rows[1].height.saturating_sub(2) as usize;
    let inner_width = rows[1].width.saturating_sub(2) as usize;
    let total: usize = lines.iter().map(|l| wrapped_height(l, inner_width)).sum();
    let scroll = if chat.follow {
        total.saturating_sub(inner_height) as u16
    } else {
        chat.transcript_scroll
            .min(total.saturating_sub(inner_height) as u16)
    };
    frame.render_widget(
        Paragraph::new(lines)
            .block(block)
            .wrap(Wrap { trim: false })
            .scroll((scroll, 0)),
        rows[1],
    );

    // Input.
    let input_block = Block::default()
        .borders(Borders::ALL)
        .title(if chat.running {
            " Input (run in progress) "
        } else {
            " Question "
        })
        .border_style(if focused { theme.accent() } else { theme.dim() });
    let input_text = if chat.input.is_empty() && !chat.running {
        Line::styled(
            "type a question, enter sends, alt-enter for a new line",
            theme.dim(),
        )
    } else {
        Line::raw(chat.input.clone())
    };
    frame.render_widget(
        Paragraph::new(input_text)
            .block(input_block)
            .wrap(Wrap { trim: false }),
        rows[2],
    );
    if focused && !chat.running {
        let w = chat.input.lines().last().map(|l| l.width()).unwrap_or(0) as u16;
        let y = rows[2].y + 1 + chat.input.lines().count().saturating_sub(1) as u16;
        frame.set_cursor_position((
            rows[2].x + 1 + w.min(rows[2].width.saturating_sub(3)),
            y.min(rows[2].y + rows[2].height.saturating_sub(2)),
        ));
    }
}

fn wrapped_height(line: &Line<'_>, width: usize) -> usize {
    if width == 0 {
        return 1;
    }
    let w = line.width();
    (w / width + usize::from(w % width > 0)).max(1)
}

fn rail_line<'a>(run: &RunView, plan: &PlanSummary, theme: &Theme) -> Line<'a> {
    let mut spans: Vec<Span> = Vec::new();
    for k in 1..=plan.passes as u16 {
        let glyph = match run.passes.iter().find(|p| p.k == k) {
            Some(p) => match p.status {
                Some(PassStatus::Done) => "✓ done".to_string(),
                Some(PassStatus::Skipped) => "– skipped".to_string(),
                Some(PassStatus::Stopped) => "✕ stopped".to_string(),
                Some(PassStatus::Accepted) => "✓ accepted".to_string(),
                None => format!("▶ {} chars", p.text.len()),
            },
            None => "○".to_string(),
        };
        spans.push(Span::styled(
            format!("{k} {glyph}"),
            if run.current_pass() == Some(k) {
                theme.accent()
            } else {
                Style::default()
            },
        ));
        spans.push(Span::styled(" · ", theme.dim()));
    }
    let final_glyph = if run.done && run.final_started {
        "✓"
    } else if run.final_started {
        "▶"
    } else {
        "○"
    };
    spans.push(Span::styled(
        format!("final {final_glyph}"),
        if run.final_started && !run.done {
            theme.accent()
        } else {
            Style::default()
        },
    ));
    spans.push(Span::styled(
        format!(
            "    {} sources ≥ {:.2} · {} tokens{}{}",
            plan.kept,
            plan.min_relevance,
            plan.total_tokens,
            if plan.dropped > 0 {
                format!(" · {} dropped", plan.dropped)
            } else {
                String::new()
            },
            if plan.fallback { " · fallback" } else { "" }
        ),
        theme.dim(),
    ));
    Line::from(spans)
}

fn push_message<'a>(
    lines: &mut Vec<Line<'a>>,
    role: &str,
    content: &str,
    sources: &[Source],
    theme: &Theme,
) {
    if role == "user" {
        lines.push(Line::from(vec![
            Span::styled("You ", theme.accent().add_modifier(Modifier::BOLD)),
            Span::raw(content.to_string()),
        ]));
    } else {
        lines.push(Line::styled(
            "GeoRAG",
            theme.accent().add_modifier(Modifier::BOLD),
        ));
        push_markdown(lines, content, sources, theme);
    }
    lines.push(Line::from(""));
}

/// Markdown-lite: headings, bold, lists, code fences; citations become
/// `[file locator]` in the accent colour.
pub fn push_markdown<'a>(lines: &mut Vec<Line<'a>>, text: &str, sources: &[Source], theme: &Theme) {
    let mut in_code = false;
    for raw in text.lines() {
        if raw.trim_start().starts_with("```") {
            in_code = !in_code;
            continue;
        }
        if in_code {
            lines.push(Line::styled(format!("    {raw}"), theme.dim()));
            continue;
        }
        let (prefix, body, style) = if let Some(h) = raw.strip_prefix("### ") {
            (
                "",
                h.to_string(),
                Style::default().add_modifier(Modifier::BOLD),
            )
        } else if let Some(h) = raw.strip_prefix("## ") {
            (
                "",
                h.to_string(),
                Style::default().add_modifier(Modifier::BOLD),
            )
        } else if let Some(h) = raw.strip_prefix("# ") {
            (
                "",
                h.to_string(),
                Style::default().add_modifier(Modifier::BOLD),
            )
        } else if let Some(item) = raw
            .trim_start()
            .strip_prefix("- ")
            .or_else(|| raw.trim_start().strip_prefix("* "))
        {
            ("  • ", item.to_string(), Style::default())
        } else {
            ("", raw.to_string(), Style::default())
        };
        let mut spans: Vec<Span> = Vec::new();
        if !prefix.is_empty() {
            spans.push(Span::raw(prefix.to_string()));
        }
        let mut rest = body.as_str();
        while let Some(open) = rest.find('[') {
            if let Some(close) = rest[open..].find(']') {
                let inner = &rest[open + 1..open + close];
                let tags = georag_core::chat::prompts::cited_tags(&rest[open..=open + close]);
                if !tags.is_empty() && inner.starts_with('S') {
                    push_bold(&mut spans, &rest[..open], style);
                    let label = tags
                        .iter()
                        .map(|t| match sources.iter().find(|s| s.tag == *t) {
                            Some(s) => format!(
                                "[{} {}]",
                                short_name(&s.rel_path),
                                s.locator.as_deref().unwrap_or("")
                            ),
                            None => format!("[S{t}]"),
                        })
                        .collect::<Vec<_>>()
                        .join("");
                    spans.push(Span::styled(label, theme.accent()));
                    rest = &rest[open + close + 1..];
                    continue;
                }
            }
            push_bold(&mut spans, &rest[..=open], style);
            rest = &rest[open + 1..];
        }
        push_bold(&mut spans, rest, style);
        lines.push(Line::from(spans));
    }
}

fn push_bold<'a>(spans: &mut Vec<Span<'a>>, text: &str, base: Style) {
    let mut parts = text.split("**");
    if let Some(first) = parts.next() {
        if !first.is_empty() {
            spans.push(Span::styled(first.to_string(), base));
        }
    }
    let mut bold = true;
    for part in parts {
        if !part.is_empty() {
            spans.push(Span::styled(
                part.to_string(),
                if bold {
                    base.add_modifier(Modifier::BOLD)
                } else {
                    base
                },
            ));
        }
        bold = !bold;
    }
}

fn short_name(rel_path: &str) -> String {
    rel_path.rsplit('/').next().unwrap_or(rel_path).to_string()
}

fn draw_sources(frame: &mut Frame, area: Rect, state: &State, theme: &Theme) {
    let chat = &state.chat;
    let focused = chat.focus == ChatFocus::Sources;
    let sources = chat.visible_sources();
    let items: Vec<ListItem> = sources
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let marker = if focused && i == chat.source_cursor {
                ">"
            } else {
                " "
            };
            let line = Line::from(vec![
                Span::raw(format!("{marker} ")),
                Span::styled(format!("S{:<3}", s.tag), theme.accent()),
                Span::raw(format!("{:.2} ", s.score)),
                Span::raw(format!(
                    "{} {}",
                    short_name(&s.rel_path),
                    s.locator.as_deref().unwrap_or("")
                )),
            ]);
            if focused && i == chat.source_cursor {
                ListItem::new(line).style(theme.selected())
            } else {
                ListItem::new(line)
            }
        })
        .collect();
    let title = match chat.source_pass {
        Some(k) => format!(" Sources, pass {k} "),
        None => " Sources ".to_string(),
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .border_style(if focused { theme.accent() } else { theme.dim() });
    if items.is_empty() {
        frame.render_widget(
            Paragraph::new(Line::styled("sources appear here after a run", theme.dim()))
                .block(block)
                .wrap(Wrap { trim: false }),
            area,
        );
    } else {
        frame.render_widget(List::new(items).block(block), area);
    }
}

/// Chunk text overlay.
pub fn draw_chunk_overlay(
    frame: &mut Frame,
    area: Rect,
    title: &str,
    text: &str,
    scroll: u16,
    theme: &Theme,
) {
    let width = area.width.saturating_sub(4).clamp(20, 100);
    let height = area.height.saturating_sub(2).max(5);
    let rect = Rect {
        x: area.x + (area.width.saturating_sub(width)) / 2,
        y: area.y + 1,
        width,
        height,
    };
    frame.render_widget(Clear, rect);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" {title} "))
        .border_style(theme.accent());
    frame.render_widget(
        Paragraph::new(text.to_string())
            .block(block)
            .wrap(Wrap { trim: false })
            .scroll((scroll, 0)),
        rect,
    );
}

pub type SharedApp = Arc<georag_core::app::App>;

/// Background chat run: streams events into the loop.
pub fn spawn_run(
    app: SharedApp,
    req: georag_core::chat::ChatRequest,
    controls: Controls,
    tx: tokio::sync::mpsc::Sender<Msg>,
) -> std::thread::JoinHandle<()> {
    std::thread::Builder::new()
        .name("georag-chat-run".into())
        .spawn(move || {
            let ctx = app.chat_context();
            let tx_events = tx.clone();
            let result = georag_core::chat::run(&ctx, &req, &controls, &mut |ev| {
                let _ = tx_events.blocking_send(Msg::Chat(ev));
            });
            let _ = tx.blocking_send(Msg::ChatFinished(result.map_err(|e| e.to_string())));
        })
        .expect("spawning the chat thread")
}

/// Mode and tier for a request from the screen's depth setting.
pub fn request_for(state: &State, message: String) -> georag_core::chat::ChatRequest {
    let (mode, tier) = match state.chat.depth {
        Depth::Auto => (Some(Mode::Auto), None),
        Depth::Manual(t) => (Some(Mode::Manual), Some(t)),
    };
    georag_core::chat::ChatRequest {
        conversation_id: state.chat.current.as_ref().map(|v| v.conversation.id),
        message,
        tags: state.chat.tags.clone(),
        mode,
        tier,
        min_relevance: None,
        max_passes: None,
        synthesis: None,
    }
}
