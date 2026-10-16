//! Terminal display policy and settings, without native sessions or views.
//! Owns: terminal schema, defaults, range and sanitization rules.
//! Does not own: native transport, clipboard access, emulation or rendering.
//! Ports: SettingsSection. Settings: terminal. Globals: none.
//! Invariant: deserialization and sanitization retain the established terminal format.
use nocterm_session::{Charset, validate_term};
use nocterm_settings::{SettingsSection, clamp_f32, trim_unset};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::ops::RangeInclusive;
/// Terminal font sizes a user may pick, in pixels.
pub const FONT_SIZE_RANGE: RangeInclusive<f32> = 6.0..=72.0;
/// Terminal line heights a user may pick, as multiples of the font size.
pub const LINE_HEIGHT_RANGE: RangeInclusive<f32> = 1.0..=3.0;
/// Scrollback lengths a user may pick, in lines.
pub const SCROLLBACK_RANGE: RangeInclusive<u32> = 0..=1_000_000;
/// How terminals look and behave.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TerminalSettings {
    /// Text encoding used when a session has no override.
    pub charset: Charset,
    /// Font family. Unset: the design tokens' terminal font.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font_family: Option<String>,
    /// Font size in pixels. Unset: the design tokens' terminal size.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font_size: Option<f32>,
    /// Line height as a multiple of the font size. Unset: the design tokens' value.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line_height: Option<f32>,
    /// Shape of the cursor.
    pub cursor_shape: CursorShape,
    /// Blink the cursor while the terminal has focus.
    pub cursor_blink: bool,
    /// Lines of history kept above the visible screen.
    pub scrollback_lines: u32,
    /// Copy text to the clipboard as soon as it is selected.
    pub copy_on_select: bool,
    /// Allow programs to write the clipboard only while their terminal screen has focus.
    /// Denied by default; clipboard reads are always denied.
    pub clipboard_write: ClipboardWritePolicy,
    /// Terminal type announced to the remote host (its `TERM` variable).
    pub term: String,
    /// Show the timestamp of the first output on each logical line.
    /// Show each logical line's first output time in UTC (this machine's clock).
    pub show_timestamps: bool,
    /// Show sequential logical line numbers outside the terminal grid.
    pub show_line_numbers: bool,
    /// Highlight dates, addresses and important messages in otherwise unstyled output.
    pub semantic_highlighting: bool,
}

impl Default for TerminalSettings {
    fn default() -> Self {
        Self {
            charset: Charset::default(),
            font_family: None,
            font_size: None,
            line_height: None,
            cursor_shape: CursorShape::default(),
            cursor_blink: true,
            scrollback_lines: 10_000,
            copy_on_select: false,
            clipboard_write: ClipboardWritePolicy::default(),
            term: "xterm-256color".to_owned(),
            show_timestamps: false,
            show_line_numbers: false,
            semantic_highlighting: true,
        }
    }
}

/// Permission for terminal output (OSC 52) to change the system clipboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClipboardWritePolicy {
    #[default]
    Deny,
    FocusedTerminal,
}

/// Shape of the terminal cursor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CursorShape {
    #[default]
    Block,
    Bar,
    Underline,
}

impl SettingsSection for TerminalSettings {
    const KEY: &'static str = "terminal";

    fn sanitize(&mut self) {
        trim_unset(&mut self.font_family);
        self.font_size = self
            .font_size
            .and_then(|size| clamp_f32(size, FONT_SIZE_RANGE));
        self.line_height = self
            .line_height
            .and_then(|height| clamp_f32(height, LINE_HEIGHT_RANGE));
        self.scrollback_lines = self
            .scrollback_lines
            .clamp(*SCROLLBACK_RANGE.start(), *SCROLLBACK_RANGE.end());
        self.term = self.term.trim().to_owned();
        if validate_term(&self.term).is_err() {
            self.term = Self::default().term;
        }
    }
}

mod schema;
