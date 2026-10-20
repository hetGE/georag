//! UI state, messages and commands. Nothing here touches the terminal.

use crossterm::event::{KeyEvent, MouseEvent};
use georag_core::status::Snapshot;

/// Top-level screens. Chat to Settings are the five tabs; Import and Update
/// are reached from Home.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Home,
    Chat,
    Library,
    Models,
    Status,
    Settings,
    Import,
    Update,
}

impl Screen {
    pub const TABS: [Screen; 5] = [
        Screen::Chat,
        Screen::Library,
        Screen::Models,
        Screen::Status,
        Screen::Settings,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Screen::Home => "Home",
            Screen::Chat => "Chat",
            Screen::Library => "Library",
            Screen::Models => "Models",
            Screen::Status => "Status",
            Screen::Settings => "Settings",
            Screen::Import => "Import from v1",
            Screen::Update => "Update",
        }
    }

    /// Which phase delivers the screen; `None` once it exists.
    pub fn arrives_in(self) -> Option<&'static str> {
        match self {
            Screen::Home => None,
            Screen::Chat | Screen::Models | Screen::Update => Some("P1"),
            Screen::Library | Screen::Settings | Screen::Import => Some("P2"),
            Screen::Status => Some("P3"),
        }
    }

    pub fn tab_index(self) -> Option<usize> {
        Screen::TABS.iter().position(|s| *s == self)
    }
}

/// Modal layers drawn over the current screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Overlay {
    Help { scroll: u16 },
}

/// One row of the Home menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuItem {
    Screen(Screen),
    Help,
    Quit,
}

pub const HOME_MENU: [(char, &str, MenuItem); 9] = [
    ('1', "Chat", MenuItem::Screen(Screen::Chat)),
    ('2', "Library", MenuItem::Screen(Screen::Library)),
    ('3', "Models", MenuItem::Screen(Screen::Models)),
    ('4', "Status", MenuItem::Screen(Screen::Status)),
    ('5', "Settings", MenuItem::Screen(Screen::Settings)),
    ('i', "Import from v1", MenuItem::Screen(Screen::Import)),
    ('u', "Update", MenuItem::Screen(Screen::Update)),
    ('?', "Help", MenuItem::Help),
    ('q', "Quit", MenuItem::Quit),
];

/// One line of the first-run checklist.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChecklistItem {
    pub label: &'static str,
    pub done: bool,
    pub optional: bool,
    pub opens: Screen,
}

/// Which Home pane has the cursor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HomeFocus {
    Menu,
    Checklist,
}

#[derive(Debug, Clone)]
pub struct State {
    pub screen: Screen,
    pub overlay: Option<Overlay>,
    pub snapshot: Snapshot,
    pub home_focus: HomeFocus,
    pub menu_cursor: usize,
    pub checklist_cursor: usize,
    pub should_quit: bool,
    pub mouse: bool,
    pub width: u16,
    pub height: u16,
    pub tick: u64,
    /// Toast text and ticks left (5 s at 250 ms per tick).
    pub toast: Option<(String, u32)>,
}

pub const TOAST_TICKS: u32 = 20;

#[derive(Debug, Clone)]
pub enum Msg {
    Key(KeyEvent),
    Mouse(MouseEvent),
    Resize(u16, u16),
    Tick,
}

/// Side effects requested by `update`; the loop performs them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Cmd {
    Quit,
}

impl State {
    pub fn new(snapshot: Snapshot, mouse: bool) -> State {
        let home_focus = if snapshot.is_first_run() {
            HomeFocus::Checklist
        } else {
            HomeFocus::Menu
        };
        State {
            screen: Screen::Home,
            overlay: None,
            snapshot,
            home_focus,
            menu_cursor: 0,
            checklist_cursor: 0,
            should_quit: false,
            mouse,
            width: 0,
            height: 0,
            tick: 0,
            toast: None,
        }
    }

    pub fn checklist(&self) -> Vec<ChecklistItem> {
        let s = &self.snapshot;
        vec![
            ChecklistItem {
                label: "Home folder created",
                done: s.home_initialised,
                optional: false,
                opens: Screen::Settings,
            },
            ChecklistItem {
                label: "Pull a chat model and an embedding model",
                done: s.models_ready(),
                optional: false,
                opens: Screen::Models,
            },
            ChecklistItem {
                label: "Add a library root",
                done: !s.roots.is_empty(),
                optional: false,
                opens: Screen::Library,
            },
            ChecklistItem {
                label: "Import tags and files from v1",
                done: false,
                optional: true,
                opens: Screen::Import,
            },
        ]
    }

    pub fn show_toast(&mut self, text: impl Into<String>) {
        let text = text.into();
        tracing::info!(toast = %text);
        self.toast = Some((text, TOAST_TICKS));
    }

    /// True when a pass or job is running; nothing runs in P0.
    pub fn busy(&self) -> bool {
        false
    }
}
