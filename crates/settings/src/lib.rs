//! User settings: their schema and their file.
//!
//! Settings are what a *user* chooses (which font, which cursor); design
//! tokens (`nocterm-design`) are what the *product* looks like by default.
//! A setting left unset falls back to the matching token.

mod ai;
mod explorer;
mod monitor;
mod schema;
mod session_options;
pub use session_options::{
    Charset, LoggingOptions, ProxyConfig, SessionOptions, TERM_PRESETS, validate_term,
};
mod store;

pub use ai::lifecycle::{AgentResourceSettings, AgentSessionSettings};
pub use ai::{AgentServerSettings, AiSettings, ApprovalPolicy, ApprovalSettings, SandboxMode};
pub use explorer::{
    ExplorerSettings, FILE_PLACEHOLDER, INDEXING_ENTRIES_RANGE, IndexingSettings, OpenRule,
    OpenSettings, Opener,
};
pub use monitor::{
    MONITOR_DETAIL_INTERVAL_RANGE, MONITOR_HISTORY_RANGE, MONITOR_INTERVAL_RANGE, MonitorMetric,
    MonitorSettings,
};
pub use schema::{
    Appearance, AppearanceMode, CARD_GAP_RANGE, CARD_RADIUS_RANGE, CONNECT_TIMEOUT_RANGE,
    ClipboardWritePolicy, CursorShape, FONT_SIZE_RANGE, KEEPALIVE_RANGE, LINE_HEIGHT_RANGE,
    SCROLLBACK_RANGE, Settings, ShellSettings, SshSettings, TerminalSettings, UiLayout,
    VaultSettings,
};
pub use store::{LoadedSettings, SectionError, SettingsFile};
