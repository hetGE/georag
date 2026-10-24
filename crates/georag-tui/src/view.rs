//! `view(state) -> frame`: pure rendering. Layout rules from handoff 9.1:
//! status bar, tab row, body, key bar; panes collapse below 110 columns and
//! one pane shows below 80; text is wrapped, never clipped.

use georag_core::doctor::fmt_gib;
use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap};
use unicode_width::UnicodeWidthStr;

use crate::screens::{chat, models};
use crate::state::{EngineState, HOME_MENU, HomeFocus, Overlay, Screen, State};
use crate::theme::Theme;

pub const NARROW: u16 = 80;
pub const COMPACT: u16 = 110;

pub fn draw(frame: &mut Frame, state: &State, theme: &Theme) {
    let area = frame.area();
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .split(area);

    draw_status_bar(frame, rows[0], state, theme);
    draw_tab_row(frame, rows[1], state, theme);
    match state.screen {
        Screen::Home => draw_home(frame, rows[2], state, theme),
        Screen::Chat => chat::draw(frame, rows[2], state, theme),
        Screen::Models => models::draw(frame, rows[2], state, theme),
        other => draw_placeholder(frame, rows[2], other, theme),
    }
    draw_key_bar(frame, rows[3], state, theme);

    if let Some(overlay) = &state.overlay {
        draw_overlay(frame, area, overlay, theme);
    }
    if let Some((text, _)) = &state.toast {
        draw_toast(frame, area, text, theme);
    }
}

fn draw_status_bar(frame: &mut Frame, area: Rect, state: &State, theme: &Theme) {
    let s = &state.snapshot;
    let (used, total) = match &state.app {
        Some(app) => {
            let st = app.engine.stats();
            (
                st.memory_used_bytes,
                st.memory_total_bytes.max(
                    s.backend
                        .as_ref()
                        .map(|b| b.device.memory_total)
                        .unwrap_or(0),
                ),
            )
        }
        None => (
            0,
            s.backend
                .as_ref()
                .map(|b| b.device.memory_total)
                .unwrap_or(0),
        ),
    };
    let backend = s
        .backend
        .as_ref()
        .map(|b| b.backend.to_string())
        .unwrap_or_else(|| "no backend".into());
    let engine = match &state.engine_state {
        EngineState::NotLoaded => "no model".to_string(),
        EngineState::Loading => "loading".to_string(),
        EngineState::Ready => match &state.app {
            Some(app) => {
                let st = app.engine.stats();
                format!("ready, ctx {}", st.n_ctx)
            }
            None => "ready".to_string(),
        },
        EngineState::Generating => match &state.app {
            Some(app) => format!("generating {:.0} t/s", app.engine.stats().tokens_per_second),
            None => "generating".to_string(),
        },
        EngineState::Failed(_) => "engine failed".to_string(),
    };
    let segments = [
        format!("georag {}", s.version),
        s.home.display().to_string(),
        engine,
        format!("{backend} {} / {}", fmt_gib(used), fmt_gib(total)),
    ];
    let left = fit_segments(&segments, area.width as usize);
    frame.render_widget(Paragraph::new(left).style(theme.bar()), area);
    // Right-aligned update notice goes here once the updater reports one.
}

/// Join segments with a separator, dropping from the right until it fits.
fn fit_segments(segments: &[String], width: usize) -> String {
    let mut n = segments.len();
    while n > 0 {
        let text = segments[..n].join(" · ");
        if text.width() <= width {
            return text;
        }
        n -= 1;
    }
    String::new()
}

/// Column where each tab's label starts, shared with mouse handling.
fn tab_spans() -> Vec<(Screen, u16, u16)> {
    let mut spans = Vec::new();
    let mut col: u16 = 1;
    for (i, screen) in Screen::TABS.iter().enumerate() {
        let label = format!("{} {}", i + 1, screen.title());
        let w = label.width() as u16;
        spans.push((*screen, col, col + w));
        col += w + 3;
    }
    spans
}

pub fn tab_column(screen: Screen) -> u16 {
    tab_spans()
        .iter()
        .find(|(s, _, _)| *s == screen)
        .map(|(_, start, _)| *start)
        .unwrap_or(0)
}

pub fn tab_at_column(column: u16) -> Option<Screen> {
    tab_spans()
        .into_iter()
        .find(|(_, start, end)| column >= *start && column < *end)
        .map(|(s, _, _)| s)
}

fn draw_tab_row(frame: &mut Frame, area: Rect, state: &State, theme: &Theme) {
    let mut spans: Vec<Span> = vec![Span::raw(" ")];
    for (i, screen) in Screen::TABS.iter().enumerate() {
        let label = format!("{} {}", i + 1, screen.title());
        let style = if state.screen == *screen {
            theme.selected()
        } else {
            theme.dim()
        };
        spans.push(Span::styled(label, style));
        if i + 1 < Screen::TABS.len() {
            spans.push(Span::raw("   "));
        }
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn draw_key_bar(frame: &mut Frame, area: Rect, state: &State, theme: &Theme) {
    let keys: Vec<(&str, &str)> = match (&state.overlay, state.screen) {
        (Some(Overlay::Help { .. }), _) | (Some(Overlay::Chunk { .. }), _) => {
            vec![("j k", "scroll"), ("esc", "close")]
        }
        (Some(Overlay::Confirm { .. }), _) => vec![("y", "yes"), ("n", "no")],
        (Some(Overlay::TextInput { .. }), _) => vec![("enter", "apply"), ("esc", "cancel")],
        (None, Screen::Home) => vec![
            ("1-5", "screens"),
            ("j k", "move"),
            ("enter", "open"),
            ("i", "import"),
            ("u", "update"),
            ("?", "help"),
            ("q", "quit"),
        ],
        (None, Screen::Chat) => chat::key_bar(state),
        (None, Screen::Models) => models::key_bar(state),
        (None, _) => vec![
            ("1-5", "screens"),
            ("tab", "next"),
            ("?", "help"),
            ("q", "back"),
        ],
    };
    let mut spans: Vec<Span> = Vec::new();
    for (i, (key, what)) in keys.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled("  ", theme.bar()));
        }
        spans.push(Span::styled(
            format!(" {key} "),
            theme.bar().add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::styled(what.to_string(), theme.bar()));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)).style(theme.bar()), area);
}

fn draw_home(frame: &mut Frame, area: Rect, state: &State, theme: &Theme) {
    let narrow = area.width < NARROW;
    if narrow {
        match state.home_focus {
            HomeFocus::Menu => draw_menu(frame, area, state, theme),
            HomeFocus::Checklist => draw_right_pane(frame, area, state, theme),
        }
        return;
    }
    let menu_width = if area.width < COMPACT { 24 } else { 30 };
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(menu_width), Constraint::Min(20)])
        .split(area);
    draw_menu(frame, cols[0], state, theme);
    draw_right_pane(frame, cols[1], state, theme);
}

fn draw_menu(frame: &mut Frame, area: Rect, state: &State, theme: &Theme) {
    let focused = state.home_focus == HomeFocus::Menu;
    let items: Vec<ListItem> = HOME_MENU
        .iter()
        .enumerate()
        .map(|(i, (key, label, _))| {
            let marker = if focused && i == state.menu_cursor {
                ">"
            } else {
                " "
            };
            let line = Line::from(vec![
                Span::raw(format!("{marker} ")),
                Span::styled(format!("{key}"), theme.accent()),
                Span::raw(format!("  {label}")),
            ]);
            if focused && i == state.menu_cursor {
                ListItem::new(line).style(theme.selected())
            } else {
                ListItem::new(line)
            }
        })
        .collect();
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" GeoRAG ")
        .border_style(if focused { theme.accent() } else { theme.dim() });
    frame.render_widget(List::new(items).block(block), area);
}

fn draw_right_pane(frame: &mut Frame, area: Rect, state: &State, theme: &Theme) {
    if state.snapshot.is_first_run() {
        draw_checklist(frame, area, state, theme);
    } else {
        draw_glance(frame, area, state, theme);
    }
}

fn draw_checklist(frame: &mut Frame, area: Rect, state: &State, theme: &Theme) {
    let focused = state.home_focus == HomeFocus::Checklist;
    let mut lines: Vec<Line> = vec![
        Line::from("Welcome. Three steps before the first question:"),
        Line::from(""),
    ];
    for (i, item) in state.checklist().iter().enumerate() {
        let glyph = if item.done { "✓" } else { "○" };
        let glyph_style = if item.done {
            Style::default().fg(theme.ok)
        } else {
            theme.dim()
        };
        let marker = if focused && i == state.checklist_cursor {
            ">"
        } else {
            " "
        };
        let suffix = if item.optional { "  (optional)" } else { "" };
        let mut line = Line::from(vec![
            Span::raw(format!("{marker} ")),
            Span::styled(glyph.to_string(), glyph_style),
            Span::raw(format!("  {}{suffix}", item.label)),
        ]);
        if focused && i == state.checklist_cursor {
            line = line.style(theme.selected());
        }
        lines.push(line);
    }
    lines.push(Line::from(""));
    lines.push(Line::styled(
        "enter opens the screen for the highlighted step",
        theme.dim(),
    ));
    lines.push(Line::from(""));
    lines.extend(glance_lines(state, theme));
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" First run ")
        .border_style(if focused { theme.accent() } else { theme.dim() });
    frame.render_widget(
        Paragraph::new(lines)
            .block(block)
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn draw_glance(frame: &mut Frame, area: Rect, state: &State, theme: &Theme) {
    let mut lines = glance_lines(state, theme);
    lines.push(Line::from(""));
    lines.push(Line::styled("Recent conversations", theme.accent()));
    if state.chat.conversations.is_empty() {
        lines.push(Line::styled("  none yet", theme.dim()));
    } else {
        for c in state.chat.conversations.iter().take(8) {
            lines.push(Line::from(format!(
                "  {}  {}",
                &c.updated_at[..10.min(c.updated_at.len())],
                c.title.as_deref().unwrap_or("(untitled)")
            )));
        }
    }
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" At a glance ")
        .border_style(theme.dim());
    frame.render_widget(
        Paragraph::new(lines)
            .block(block)
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn glance_lines(state: &State, theme: &Theme) -> Vec<Line<'static>> {
    let s = &state.snapshot;
    let engine = match (&s.backend, &s.backend_error) {
        (Some(b), _) => format!(
            "{} on {}, {} free of {}",
            b.backend,
            b.device.description,
            fmt_gib(b.device.memory_free),
            fmt_gib(b.device.memory_total)
        ),
        (None, Some(e)) => format!("unavailable: {e}"),
        (None, None) => "not probed".to_string(),
    };
    let row = |k: &str, v: String| {
        Line::from(vec![
            Span::styled(format!("{k:<10}"), theme.dim()),
            Span::raw(v),
        ])
    };
    let models = match &state.app {
        Some(app) => format!(
            "chat {}, embed {}",
            app.plan.chat_id.as_deref().unwrap_or("none"),
            app.plan.embed_id.as_deref().unwrap_or("none")
        ),
        None => format!(
            "chat {}, embed {}",
            s.models.chat.as_deref().unwrap_or("none"),
            s.models.embed.as_deref().unwrap_or("none")
        ),
    };
    vec![
        Line::styled("At a glance", theme.accent()),
        row("flavour", s.flavour.to_string()),
        row("engine", engine),
        row("models", models),
        row("roots", s.roots.len().to_string()),
        row(
            "files",
            format!(
                "{} total, {} indexed, {} failed",
                s.files.total(),
                s.files.indexed,
                s.files.failed
            ),
        ),
        row(
            "daemon",
            if s.daemon_running {
                "running"
            } else {
                "not running"
            }
            .to_string(),
        ),
        row(
            "jobs",
            format!("{} queued, {} running", s.jobs_queued, s.jobs_running),
        ),
    ]
}

fn draw_placeholder(frame: &mut Frame, area: Rect, screen: Screen, theme: &Theme) {
    let phase = screen.arrives_in().unwrap_or("a later phase");
    let lines = vec![
        Line::from(""),
        Line::styled(screen.title(), theme.accent()),
        Line::from(""),
        Line::from(format!("This screen arrives in phase {phase}.")),
        Line::styled("Press q to go back to Home.", theme.dim()),
    ];
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(theme.dim());
    frame.render_widget(
        Paragraph::new(lines)
            .block(block)
            .alignment(Alignment::Center)
            .wrap(Wrap { trim: false }),
        area,
    );
}

const HELP: &[(&str, &str)] = &[
    ("1 to 5", "jump to Chat, Library, Models, Status, Settings"),
    ("tab, shift-tab", "next pane, or next tab"),
    ("j k, arrows", "move the cursor"),
    ("pgup pgdn", "page"),
    ("g G", "first and last row"),
    ("/ n N", "filter, next and previous match"),
    ("enter", "open the highlighted item, or send a question"),
    (
        "alt-enter",
        "new line in the question box (ctrl-j on legacy consoles)",
    ),
    ("?", "this help"),
    ("esc", "close an overlay, or stop the current run"),
    ("s a", "skip or accept the current pass during a run"),
    ("q", "back to Home, then quit"),
    ("ctrl-c", "quit now; the daemon keeps running"),
    ("d t", "chat: cycle depth, set tags"),
    ("c S", "chat: toggle the conversations and sources panes"),
    ("e", "chat: export the conversation to markdown in exports/"),
    ("i", "import from v1 (Home)"),
    ("u", "update (Home)"),
];

fn draw_overlay(frame: &mut Frame, area: Rect, overlay: &Overlay, theme: &Theme) {
    match overlay {
        Overlay::Help { scroll } => {
            let width = area.width.clamp(20, 76);
            let height = area.height.min(HELP.len() as u16 + 4).max(5);
            let rect = centred(area, width, height);
            frame.render_widget(Clear, rect);
            let lines: Vec<Line> = HELP
                .iter()
                .map(|(k, v)| {
                    Line::from(vec![
                        Span::styled(format!("{k:<16}"), theme.accent()),
                        Span::raw(*v),
                    ])
                })
                .collect();
            let block = Block::default()
                .borders(Borders::ALL)
                .title(" Keys ")
                .border_style(theme.accent());
            frame.render_widget(
                Paragraph::new(lines)
                    .block(block)
                    .scroll((*scroll, 0))
                    .wrap(Wrap { trim: false }),
                rect,
            );
        }
        Overlay::Confirm { text, .. } => {
            let width = (text.width() as u16 + 6).clamp(24, area.width);
            let rect = centred(area, width, 5);
            frame.render_widget(Clear, rect);
            let block = Block::default()
                .borders(Borders::ALL)
                .title(" Confirm ")
                .border_style(Style::default().fg(theme.warn));
            let lines = vec![
                Line::raw(text.clone()),
                Line::styled("y yes · n no", theme.dim()),
            ];
            frame.render_widget(
                Paragraph::new(lines)
                    .block(block)
                    .wrap(Wrap { trim: false }),
                rect,
            );
        }
        Overlay::TextInput { title, value, .. } => {
            let width = area.width.clamp(30, 90);
            let rect = centred(area, width, 5);
            frame.render_widget(Clear, rect);
            let block = Block::default()
                .borders(Borders::ALL)
                .title(format!(" {title} "))
                .border_style(theme.accent());
            frame.render_widget(
                Paragraph::new(Line::raw(format!("{value}▏")))
                    .block(block)
                    .wrap(Wrap { trim: false }),
                rect,
            );
        }
        Overlay::Chunk {
            title,
            text,
            scroll,
        } => chat::draw_chunk_overlay(frame, area, title, text, *scroll, theme),
    }
}

fn draw_toast(frame: &mut Frame, area: Rect, text: &str, theme: &Theme) {
    let width = (text.width() as u16 + 4).min(area.width);
    let rect = Rect {
        x: area.width.saturating_sub(width),
        y: area.height.saturating_sub(3),
        width,
        height: 3,
    };
    frame.render_widget(Clear, rect);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.warn));
    frame.render_widget(Paragraph::new(text).block(block), rect);
}

fn centred(area: Rect, width: u16, height: u16) -> Rect {
    Rect {
        x: area.x + (area.width.saturating_sub(width)) / 2,
        y: area.y + (area.height.saturating_sub(height)) / 2,
        width: width.min(area.width),
        height: height.min(area.height),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::Depth;
    use georag_core::status::Snapshot;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use std::path::PathBuf;

    fn render(width: u16, height: u16, state: &State) -> String {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        let theme = Theme::with_depth(Depth::Basic16);
        terminal.draw(|f| draw(f, state, &theme)).unwrap();
        let buffer = terminal.backend().buffer().clone();
        let mut out = String::new();
        for y in 0..height {
            for x in 0..width {
                out.push_str(buffer[(x, y)].symbol());
            }
            out.push('\n');
        }
        out
    }

    fn state() -> State {
        State::new(
            Snapshot::empty(PathBuf::from("/tmp/georag-test")),
            true,
            Theme::with_depth(Depth::Basic16),
        )
    }

    #[test]
    fn chrome_and_checklist_render_at_full_width() {
        let text = render(120, 30, &state());
        assert!(text.contains("georag 2.0.0-dev"));
        assert!(text.contains("1 Chat"));
        assert!(text.contains("5 Settings"));
        assert!(text.contains("First run"));
        assert!(text.contains("Pull a chat model"));
        assert!(text.contains(" q quit"));
    }

    #[test]
    fn narrow_terminal_shows_one_pane_without_panicking() {
        let text = render(60, 20, &state());
        assert!(text.contains("First run"));
        assert!(!text.contains(" GeoRAG "));
        let tiny = render(20, 6, &state());
        assert!(tiny.contains("georag"));
    }

    #[test]
    fn chat_and_models_screens_render() {
        let mut s = state();
        s.screen = Screen::Chat;
        s.chat.run = Some(crate::screens::chat::RunView {
            question: "How deep?".into(),
            plan: Some(georag_core::chat::PlanSummary {
                passes: 3,
                kept: 9,
                total_tokens: 3000,
                dropped: 0,
                fallback: false,
                budget: 1200,
                min_relevance: 0.65,
            }),
            passes: vec![crate::screens::chat::PassView {
                k: 1,
                sources_from: 1,
                sources_to: 3,
                status: Some(georag_core::chat::PassStatus::Done),
                text: "Twelve metres [S1]".into(),
                cut_off: false,
            }],
            ..Default::default()
        });
        let text = render(120, 30, &s);
        assert!(text.contains("1 ✓ done"));
        assert!(text.contains("final ○"));
        assert!(text.contains("Twelve metres"));
        let narrow = render(60, 24, &s);
        assert!(narrow.contains("Chat"));

        s.screen = Screen::Models;
        let text = render(120, 30, &s);
        assert!(text.contains("Catalogue"));
        assert!(text.contains("qwen3.5-9b-q4_k_m"));
        s.models.catalog_cursor = s.models.catalog().len() - 1;
        let text = render(120, 30, &s);
        assert!(
            text.contains("nomic-embed-text-v1.5"),
            "cursor on the last entry scrolls it into view:\n{text}"
        );
        let text = render(40, 12, &s);
        assert!(text.contains("Installed"), "40x12 render:\n{text}");
    }

    #[test]
    fn overlays_render() {
        let mut s = state();
        s.overlay = Some(Overlay::Help { scroll: 0 });
        assert!(render(100, 30, &s).contains("Keys"));
        s.overlay = Some(Overlay::Confirm {
            text: "Sure?".into(),
            on_yes: crate::state::ConfirmAction::Quit,
        });
        assert!(render(100, 30, &s).contains("Sure?"));
        s.overlay = Some(Overlay::TextInput {
            title: "Tags".into(),
            value: "piling".into(),
            purpose: crate::state::InputPurpose::ChatTags,
        });
        assert!(render(100, 30, &s).contains("piling"));
        s.overlay = Some(Overlay::Chunk {
            title: "a.pdf p.3".into(),
            text: "chunk text".into(),
            scroll: 0,
        });
        assert!(render(100, 30, &s).contains("chunk text"));
    }

    #[test]
    fn segments_drop_from_the_right_when_narrow() {
        let segs = vec!["aaaa".to_string(), "bbbb".to_string(), "cccc".to_string()];
        assert_eq!(fit_segments(&segs, 100), "aaaa · bbbb · cccc");
        assert_eq!(fit_segments(&segs, 12), "aaaa · bbbb");
        assert_eq!(fit_segments(&segs, 3), "");
    }

    #[test]
    fn tab_columns_round_trip() {
        for screen in Screen::TABS {
            assert_eq!(tab_at_column(tab_column(screen)), Some(screen));
        }
        assert_eq!(tab_at_column(0), None);
    }
}
