//! User settings: their schema and their file.
//!
//! Settings are what a *user* chooses (which font, which cursor); design
//! tokens (`nocterm-design`) are what the *product* looks like by default.
//! A setting left unset falls back to the matching token.

mod schema;
mod session_options;
pub use session_options::{
    Charset, LoggingOptions, ProxyConfig, SessionOptions, TERM_PRESETS, validate_term,
};
mod store;

pub use schema::{
    Appearance, AppearanceMode, CONNECT_TIMEOUT_RANGE, CursorShape, FONT_SIZE_RANGE,
    KEEPALIVE_RANGE, LINE_HEIGHT_RANGE, SCROLLBACK_RANGE, Settings, ShellSettings, SshSettings,
    TerminalSettings, VaultSettings,
};
pub use store::SettingsFile;
