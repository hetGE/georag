//! Import from v1 wizard (handoff 9.2 and 10): locate, map root, preview,
//! options, run, summary.

use std::path::PathBuf;

use crossterm::event::{KeyCode, KeyEvent};
use georag_core::import_v1::{ImportReport, Preview};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

use crate::state::{Cmd, InputPurpose, Overlay, State};
use crate::theme::Theme;

#[derive(Debug, Clone, PartialEq)]
pub enum Step {
    Locate,
    MapRoot,
    Preview,
    Options,
    Running,
    Summary(ImportReport),
    Failed(String),
}

#[derive(Debug, Clone)]
pub struct ImportState {
    pub step: Step,
    pub source: Option<PathBuf>,
    pub root: Option<PathBuf>,
    pub preview: Option<Preview>,
    pub with_chats: bool,
    pub retag: bool,
    pub option_cursor: usize,
    pub busy: bool,
}

impl Default for ImportState {
    fn default() -> Self {
        Self {
            step: Step::Locate,
            source: None,
            root: None,
            preview: None,
            with_chats: false,
            retag: false,
            option_cursor: 0,
            busy: false,
        }
    }
}

pub fn on_key(state: &mut State, key: KeyEvent) -> Vec<Cmd> {
    let imp = &mut state.import;
    match (&imp.step, key.code) {
        (_, KeyCode::Char('r')) if matches!(imp.step, Step::Summary(_) | Step::Failed(_)) => {
            state.import = ImportState::default();
            Vec::new()
        }
        (Step::Locate, KeyCode::Enter) | (Step::Locate, KeyCode::Char('l')) => {
            state.overlay = Some(Overlay::TextInput {
                title: "Path to the v1 _0RAG folder, its metadata.db, or a .georag archive".into(),
                value: imp
                    .source
                    .as_ref()
                    .map(|p| p.display().to_string())
                    .unwrap_or_default(),
                purpose: InputPurpose::ImportSource,
            });
            Vec::new()
        }
        (Step::MapRoot, KeyCode::Enter) | (Step::MapRoot, KeyCode::Char('m')) => {
            state.overlay = Some(Overlay::TextInput {
                title: "Folder the documents live in (the v1 relative paths are based here)".into(),
                value: imp
                    .root
                    .as_ref()
                    .map(|p| p.display().to_string())
                    .unwrap_or_default(),
                purpose: InputPurpose::ImportRoot,
            });
            Vec::new()
        }
        (Step::Preview, KeyCode::Enter) => {
            imp.step = Step::Options;
            Vec::new()
        }
        (Step::Preview, KeyCode::Backspace) => {
            imp.step = Step::MapRoot;
            Vec::new()
        }
        (Step::Options, KeyCode::Char('j') | KeyCode::Down) => {
            imp.option_cursor = (imp.option_cursor + 1).min(2);
            Vec::new()
        }
        (Step::Options, KeyCode::Char('k') | KeyCode::Up) => {
            imp.option_cursor = imp.option_cursor.saturating_sub(1);
            Vec::new()
        }
        (Step::Options, KeyCode::Char(' ')) => {
            match imp.option_cursor {
                0 => imp.with_chats = !imp.with_chats,
                1 => imp.retag = !imp.retag,
                _ => {}
            }
            Vec::new()
        }
        (Step::Options, KeyCode::Enter) => {
            imp.step = Step::Running;
            imp.busy = true;
            vec![Cmd::ImportRun]
        }
        (Step::Options, KeyCode::Backspace) => {
            imp.step = Step::Preview;
            Vec::new()
        }
        (Step::Summary(_), KeyCode::Enter) => vec![Cmd::StartIndexing, Cmd::GotoLibrary],
        _ => Vec::new(),
    }
}

pub fn key_bar(state: &State) -> Vec<(&'static str, &'static str)> {
    match state.import.step {
        Step::Locate => vec![("enter", "choose source"), ("q", "back")],
        Step::MapRoot => vec![("enter", "set root"), ("q", "back")],
        Step::Preview => vec![
            ("enter", "continue"),
            ("backspace", "change root"),
            ("q", "back"),
        ],
        Step::Options => vec![
            ("space", "toggle"),
            ("enter", "run import"),
            ("backspace", "back"),
            ("q", "cancel"),
        ],
        Step::Running => vec![("", "importing")],
        Step::Summary(_) => vec![
            ("enter", "start indexing now"),
            ("r", "import another"),
            ("q", "back"),
        ],
        Step::Failed(_) => vec![("r", "try again"), ("q", "back")],
    }
}

pub fn draw(frame: &mut Frame, area: Rect, state: &State, theme: &Theme) {
    let imp = &state.import;
    let steps = ["locate", "map root", "preview", "options", "run", "summary"];
    let current = match imp.step {
        Step::Locate => 0,
        Step::MapRoot => 1,
        Step::Preview => 2,
        Step::Options => 3,
        Step::Running => 4,
        Step::Summary(_) | Step::Failed(_) => 5,
    };
    let mut head: Vec<Span> = Vec::new();
    for (i, s) in steps.iter().enumerate() {
        head.push(Span::styled(
            format!(" {} {s} ", i + 1),
            if i == current {
                theme.selected()
            } else if i < current {
                theme.accent()
            } else {
                theme.dim()
            },
        ));
        head.push(Span::raw(" "));
    }
    let mut lines: Vec<Line> = vec![Line::from(head), Line::from("")];
    let row = |k: &str, v: String| {
        Line::from(vec![
            Span::styled(format!("{k:<12}"), theme.dim()),
            Span::raw(v),
        ])
    };
    match &imp.step {
        Step::Locate => {
            lines.push(Line::from("Import the tag assignments and the file list from the Python v1 app. Nothing is written to the v1 folder."));
            lines.push(Line::from(""));
            lines.push(row(
                "source",
                imp.source
                    .as_ref()
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|| {
                        "press enter to choose the _0RAG folder, metadata.db or a .georag archive"
                            .into()
                    }),
            ));
        }
        Step::MapRoot => {
            lines.push(row(
                "source",
                imp.source
                    .as_ref()
                    .map(|p| p.display().to_string())
                    .unwrap_or_default(),
            ));
            lines.push(row(
                "root",
                imp.root
                    .as_ref()
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|| "unknown: press enter to set it".into()),
            ));
            lines.push(Line::from(""));
            lines.push(Line::styled("The root is the folder the v1 relative paths start from, usually the parent of _0RAG. Press enter to confirm or change it.", theme.dim()));
        }
        Step::Preview | Step::Options | Step::Running => {
            if let Some(p) = &imp.preview {
                lines.push(row("root", p.root.display().to_string()));
                lines.push(row(
                    "files",
                    format!(
                        "{} ({} on disk, {} missing, {} failed in v1, {} already present)",
                        p.files,
                        p.files_on_disk,
                        p.files_missing,
                        p.files_failed_in_v1,
                        p.already_present
                    ),
                ));
                lines.push(row(
                    "tags",
                    format!(
                        "{} ({} built in, {} custom)",
                        p.tags,
                        p.tags_builtin,
                        p.tags_custom.len()
                    ),
                ));
                if !p.tags_custom.is_empty() {
                    lines.push(row("", p.tags_custom.join(", ")));
                }
                lines.push(row("file tags", p.file_tags.to_string()));
                lines.push(row(
                    "chats",
                    format!("{} conversations, {} messages", p.conversations, p.messages),
                ));
            }
            if matches!(imp.step, Step::Options | Step::Running) {
                lines.push(Line::from(""));
                let opt = |i: usize, on: bool, label: &str| {
                    let marker = if imp.option_cursor == i { ">" } else { " " };
                    let box_ = if on { "[x]" } else { "[ ]" };
                    let line = Line::from(format!("{marker} {box_} {label}"));
                    if imp.option_cursor == i {
                        line.style(theme.selected())
                    } else {
                        line
                    }
                };
                lines.push(opt(0, imp.with_chats, "import conversations"));
                lines.push(opt(
                    1,
                    imp.retag,
                    "run the tag stage again instead of keeping imported tags",
                ));
                let run_line = Line::from(format!(
                    "{} run the import",
                    if imp.option_cursor == 2 { ">" } else { " " }
                ));
                lines.push(if imp.option_cursor == 2 {
                    run_line.style(theme.selected())
                } else {
                    run_line
                });
            }
            if imp.step == Step::Running {
                lines.push(Line::from(""));
                lines.push(Line::styled("importing…", theme.accent()));
            }
        }
        Step::Summary(r) => {
            lines.push(row("root", r.root.clone()));
            lines.push(row(
                "files",
                format!(
                    "{} imported, {} merged, {} missing on disk",
                    r.files_imported,
                    r.files_merged,
                    r.files_missing.len()
                ),
            ));
            lines.push(row("file tags", r.file_tags.to_string()));
            lines.push(row(
                "new tags",
                if r.tags_added_to_file.is_empty() {
                    "none".into()
                } else {
                    r.tags_added_to_file.join(", ")
                },
            ));
            lines.push(row(
                "chats",
                format!("{} conversations, {} messages", r.conversations, r.messages),
            ));
            lines.push(row(
                "queued",
                format!(
                    "{} files for extraction; the tag stage is skipped for imported files",
                    r.jobs_queued
                ),
            ));
            for m in r.files_missing.iter().take(10) {
                lines.push(Line::styled(format!("  missing: {m}"), theme.dim()));
            }
            lines.push(Line::from(""));
            lines.push(Line::styled(
                "Press enter to start indexing now.",
                theme.accent(),
            ));
        }
        Step::Failed(e) => {
            lines.push(Line::styled(
                format!("import failed: {e}"),
                ratatui::style::Style::default().fg(theme.warn),
            ));
        }
    }
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Import from v1 ")
        .border_style(theme.accent());
    frame.render_widget(
        Paragraph::new(lines)
            .block(block)
            .wrap(Wrap { trim: false }),
        area,
    );
}
