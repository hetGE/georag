//! UI state, messages and commands. Nothing here touches the terminal.

use std::sync::Arc;

use crossterm::event::{KeyEvent, MouseEvent};
use georag_core::app::App;
use georag_core::chat::{ChatEvent, RunOutcome};
use georag_core::conversations::ConversationView;
use georag_core::models::PullManager;
use georag_core::models::ops::PullPlan;
use georag_core::status::Snapshot;
use georag_core::update::CheckReport;

use crate::screens::chat::ChatState;
use crate::screens::library::LibraryState;
use crate::screens::models::ModelsState;
use crate::screens::settings::SettingsState;
use crate::screens::update::UpdateState;
use crate::theme::Theme;

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
            Screen::Home
            | Screen::Chat
            | Screen::Models
            | Screen::Update
            | Screen::Library
            | Screen::Settings => None,
            Screen::Import => Some("P2 (import wizard)"),
            Screen::Status => Some("P3"),
        }
    }

    pub fn tab_index(self) -> Option<usize> {
        Screen::TABS.iter().position(|s| *s == self)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputPurpose {
    ChatTags,
    ConversationFilter,
    CatalogQuery,
    LinkModel,
    SetRole { id: String },
    LibraryFilter,
    LibraryTag { ids: Vec<i64> },
    LibraryUntag { ids: Vec<i64> },
    AddRoot,
    SettingValue { key: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfirmAction {
    DeleteConversation { id: i64 },
    RemoveModel { id: String },
    CombineDone { message_id: i64 },
    ApplyUpdate,
    RollbackUpdate,
    Reprocess { ids: Vec<i64> },
    RemoveRoot { name: String },
    Quit,
}

/// Modal layers drawn over the current screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Overlay {
    Help {
        scroll: u16,
    },
    Confirm {
        text: String,
        on_yes: ConfirmAction,
    },
    TextInput {
        title: String,
        value: String,
        purpose: InputPurpose,
    },
    Chunk {
        title: String,
        text: String,
        scroll: u16,
    },
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

/// Engine state for the status bar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EngineState {
    NotLoaded,
    Loading,
    Ready,
    Generating,
    Failed(String),
}

pub struct State {
    pub screen: Screen,
    pub overlay: Option<Overlay>,
    pub snapshot: Snapshot,
    pub theme: Theme,
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
    pub app: Option<Arc<App>>,
    pub engine_state: EngineState,
    pub chat: ChatState,
    pub models: ModelsState,
    pub pulls: Arc<PullManager>,
    pub pending_pulls: Vec<PullPlan>,
    pub update: UpdateState,
    /// Right-aligned status bar notice when a newer release is known.
    pub update_notice: Option<String>,
    pub library: LibraryState,
    pub settings: SettingsState,
    /// Set while the indexing worker runs; cleared to stop it.
    pub indexing: Option<Arc<std::sync::atomic::AtomicBool>>,
}

pub const TOAST_TICKS: u32 = 20;

pub enum Msg {
    Key(KeyEvent),
    Mouse(MouseEvent),
    Resize(u16, u16),
    Tick,
    Chat(ChatEvent),
    ChatFinished(Result<RunOutcome, String>),
    AppReady(Arc<App>),
    AppFailed(String),
    ConversationLoaded(ConversationView),
    ConversationsListed(Vec<georag_core::Conversation>),
    ModelsChanged,
    Toast(String),
    UpdateChecked(Result<(CheckReport, String), String>),
    UpdateProgress {
        bytes: u64,
        total: Option<u64>,
    },
    UpdateStaged(Result<std::path::PathBuf, String>),
    UpdateApplied(Result<String, String>),
    UpdateNotice(String),
    LibraryLoaded {
        files: Vec<georag_core::File>,
        tags: std::collections::HashMap<i64, Vec<String>>,
        facets: georag_core::FacetCounts,
    },
    RecordLoaded(crate::screens::library::RecordView),
    SettingsLoaded {
        entries: Vec<(String, String)>,
        roots: Vec<georag_core::Root>,
    },
    Pipeline(georag_core::pipeline::PipelineEvent),
    IndexingFinished(Result<georag_core::pipeline::Report, String>),
    DoctorText(String),
    StartIndexing,
}

/// Side effects requested by `update`; the loop performs them.
#[derive(Debug, Clone, PartialEq)]
pub enum Cmd {
    Quit,
    Ask {
        message: String,
    },
    OpenConversation {
        id: i64,
    },
    DeleteConversation {
        id: i64,
    },
    ExportConversation,
    ViewChunk {
        chunk_id: i64,
    },
    OpenFile {
        file_id: i64,
    },
    CombineDone {
        message_id: i64,
    },
    PullModel {
        spec: String,
    },
    PausePull {
        id: String,
    },
    UseModel {
        id: String,
    },
    RemoveModel {
        id: String,
    },
    VerifyModel {
        id: String,
    },
    LinkModel {
        spec: String,
    },
    SetRole {
        id: String,
        role: String,
    },
    RefreshModels,
    RefreshConversations,
    ReloadApp,
    UpdateCheck,
    UpdateDownload,
    UpdateApply,
    UpdateRollback,
    RefreshLibrary,
    OpenRecord {
        file_id: i64,
    },
    Reprocess {
        ids: Vec<i64>,
    },
    Rescan,
    AddRoot {
        path: String,
    },
    RemoveRoot {
        name: String,
    },
    TagFiles {
        ids: Vec<i64>,
        tag: String,
        remove: bool,
    },
    StartIndexing,
    RefreshSettings,
    SetSetting {
        key: String,
        value: String,
    },
    GotoUpdate,
    RunDoctor,
}

impl State {
    pub fn new(snapshot: Snapshot, mouse: bool, theme: Theme) -> State {
        let home_focus = if snapshot.is_first_run() {
            HomeFocus::Checklist
        } else {
            HomeFocus::Menu
        };
        State {
            screen: Screen::Home,
            overlay: None,
            snapshot,
            theme,
            home_focus,
            menu_cursor: 0,
            checklist_cursor: 0,
            should_quit: false,
            mouse,
            width: 0,
            height: 0,
            tick: 0,
            toast: None,
            app: None,
            engine_state: EngineState::NotLoaded,
            chat: ChatState::default(),
            models: ModelsState::default(),
            pulls: Arc::new(PullManager::new()),
            pending_pulls: Vec::new(),
            update: UpdateState::default(),
            update_notice: None,
            library: LibraryState::default(),
            settings: SettingsState::default(),
            indexing: None,
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

    /// True when a pass or job is running.
    pub fn busy(&self) -> bool {
        self.chat.running || self.indexing.is_some()
    }
}
