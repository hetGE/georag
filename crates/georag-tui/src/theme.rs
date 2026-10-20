//! Colours. Meaning is never carried by colour alone; glyphs and text do that.

use ratatui::style::{Color, Modifier, Style};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Depth {
    TrueColor,
    Indexed256,
    Basic16,
}

#[derive(Debug, Clone, Copy)]
pub struct Theme {
    pub depth: Depth,
    pub accent: Color,
    pub dim: Color,
    pub bar_bg: Color,
    pub bar_fg: Color,
    pub ok: Color,
    pub warn: Color,
}

impl Theme {
    /// `tui.color`: auto | truecolor | 256 | 16. Auto reads COLORTERM and TERM.
    pub fn from_setting(setting: &str) -> Theme {
        let depth = match setting.trim().to_ascii_lowercase().as_str() {
            "truecolor" => Depth::TrueColor,
            "256" => Depth::Indexed256,
            "16" => Depth::Basic16,
            _ => detect_depth(),
        };
        Theme::with_depth(depth)
    }

    pub fn with_depth(depth: Depth) -> Theme {
        match depth {
            Depth::TrueColor => Theme {
                depth,
                accent: Color::Rgb(94, 176, 230),
                dim: Color::Rgb(128, 136, 148),
                bar_bg: Color::Rgb(36, 42, 52),
                bar_fg: Color::Rgb(224, 228, 234),
                ok: Color::Rgb(120, 200, 120),
                warn: Color::Rgb(230, 180, 80),
            },
            Depth::Indexed256 => Theme {
                depth,
                accent: Color::Indexed(75),
                dim: Color::Indexed(245),
                bar_bg: Color::Indexed(236),
                bar_fg: Color::Indexed(254),
                ok: Color::Indexed(114),
                warn: Color::Indexed(179),
            },
            Depth::Basic16 => Theme {
                depth,
                accent: Color::Cyan,
                dim: Color::DarkGray,
                bar_bg: Color::Blue,
                bar_fg: Color::White,
                ok: Color::Green,
                warn: Color::Yellow,
            },
        }
    }

    pub fn bar(&self) -> Style {
        Style::default().bg(self.bar_bg).fg(self.bar_fg)
    }
    pub fn accent(&self) -> Style {
        Style::default()
            .fg(self.accent)
            .add_modifier(Modifier::BOLD)
    }
    pub fn dim(&self) -> Style {
        Style::default().fg(self.dim)
    }
    pub fn selected(&self) -> Style {
        Style::default()
            .fg(self.accent)
            .add_modifier(Modifier::BOLD | Modifier::REVERSED)
    }
}

fn detect_depth() -> Depth {
    let colorterm = std::env::var("COLORTERM")
        .unwrap_or_default()
        .to_ascii_lowercase();
    if colorterm == "truecolor" || colorterm == "24bit" {
        return Depth::TrueColor;
    }
    let term = std::env::var("TERM").unwrap_or_default();
    if term.contains("256color") {
        Depth::Indexed256
    } else {
        Depth::Basic16
    }
}
