//! `[appearance]` and `[terminal]`: how the interface and terminals look.
//!
//! The doc comment of every field doubles as its entry in the generated
//! reference (`docs/reference/settings.md`) and as its description in the
//! settings tab, so write it for the person changing the setting.

use std::ops::RangeInclusive;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use nocterm_session::{Charset, validate_term};
use nocterm_settings::{SettingsSection, clamp_f32, trim_unset};

/// Terminal font sizes a user may pick, in pixels.
pub const FONT_SIZE_RANGE: RangeInclusive<f32> = 6.0..=72.0;
/// Terminal line heights a user may pick, as multiples of the font size.
pub const LINE_HEIGHT_RANGE: RangeInclusive<f32> = 1.0..=3.0;
/// Scrollback lengths a user may pick, in lines.
pub const SCROLLBACK_RANGE: RangeInclusive<u32> = 0..=1_000_000;
/// Space between floating cards a user may pick, in pixels.
pub const CARD_GAP_RANGE: RangeInclusive<f32> = 0.0..=24.0;
/// Corner radii of floating cards a user may pick, in pixels.
pub const CARD_RADIUS_RANGE: RangeInclusive<f32> = 0.0..=24.0;

/// How the interface looks.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct AppearanceSettings {
    /// Which palette to use: follow the operating system, or always light or dark.
    pub mode: AppearanceMode,
    /// Imported light theme name. Unset uses Nocterm Default.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub light_theme: Option<String>,
    /// Imported dark theme name. Unset uses Nocterm Default.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dark_theme: Option<String>,
    /// Find out which country each saved server is in and show its flag. Sends
    /// the server's public IP address (never its name) to a GeoIP service on
    /// connecting; private addresses are never sent. A country set on the
    /// connection is always shown.
    pub detect_server_country: bool,
    /// How the window's regions are framed: edge to edge, or as floating
    /// cards with rounded corners on a darker canvas.
    pub layout: UiLayout,
    /// Space between floating cards and around the window's edge, in pixels.
    #[schemars(extend("minimum" = CARD_GAP_RANGE.start(), "maximum" = CARD_GAP_RANGE.end()))]
    pub card_gap: f32,
    /// Corner radius of floating cards, in pixels.
    #[schemars(extend("minimum" = CARD_RADIUS_RANGE.start(), "maximum" = CARD_RADIUS_RANGE.end()))]
    pub card_radius: f32,
}

impl AppearanceSettings {
    /// Default space between floating cards, in pixels.
    pub const DEFAULT_CARD_GAP: f32 = 4.0;
    /// Default corner radius of floating cards, in pixels.
    pub const DEFAULT_CARD_RADIUS: f32 = 10.0;
}

impl Default for AppearanceSettings {
    fn default() -> Self {
        Self {
            mode: AppearanceMode::default(),
            light_theme: None,
            dark_theme: None,
            detect_server_country: true,
            layout: UiLayout::default(),
            card_gap: Self::DEFAULT_CARD_GAP,
            card_radius: Self::DEFAULT_CARD_RADIUS,
        }
    }
}

/// How the window's regions are framed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum UiLayout {
    /// Sidebar, tabs and panels as rounded cards floating on a darker canvas.
    #[default]
    Floating,
    /// Regions edge to edge, divided by hairlines.
    Classic,
}

/// Which palette the interface uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AppearanceMode {
    /// Follow the operating system.
    #[default]
    System,
    Light,
    Dark,
}

/// How terminals look and behave.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct TerminalSettings {
    /// Text encoding used when a session has no override.
    pub charset: Charset,
    /// Font family. Unset: the design tokens' terminal font.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font_family: Option<String>,
    /// Font size in pixels. Unset: the design tokens' terminal size.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(extend("minimum" = FONT_SIZE_RANGE.start(), "maximum" = FONT_SIZE_RANGE.end()))]
    pub font_size: Option<f32>,
    /// Line height as a multiple of the font size. Unset: the design tokens' value.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(extend("minimum" = LINE_HEIGHT_RANGE.start(), "maximum" = LINE_HEIGHT_RANGE.end()))]
    pub line_height: Option<f32>,
    /// Shape of the cursor.
    pub cursor_shape: CursorShape,
    /// Blink the cursor while the terminal has focus.
    pub cursor_blink: bool,
    /// Lines of history kept above the visible screen.
    #[schemars(extend("minimum" = SCROLLBACK_RANGE.start(), "maximum" = SCROLLBACK_RANGE.end()))]
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ClipboardWritePolicy {
    #[default]
    Deny,
    FocusedTerminal,
}

/// Shape of the terminal cursor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CursorShape {
    #[default]
    Block,
    Bar,
    Underline,
}

impl SettingsSection for AppearanceSettings {
    const KEY: &'static str = "appearance";

    fn sanitize(&mut self) {
        trim_unset(&mut self.light_theme);
        trim_unset(&mut self.dark_theme);
        self.card_gap = clamp_f32(self.card_gap, CARD_GAP_RANGE).unwrap_or(Self::DEFAULT_CARD_GAP);
        self.card_radius =
            clamp_f32(self.card_radius, CARD_RADIUS_RANGE).unwrap_or(Self::DEFAULT_CARD_RADIUS);
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use nocterm_settings::SettingsDocument;

    fn read<S: SettingsSection>(text: &str) -> S {
        let mut document = SettingsDocument::from_table(toml::from_str(text).unwrap());
        document.register::<S>();
        assert!(document.errors().is_empty(), "{:?}", document.errors());
        document.get::<S>().clone()
    }

    #[test]
    fn partial_table_keeps_the_other_defaults() {
        let terminal: TerminalSettings = read("[terminal]\ncursor_shape = \"bar\"\n");
        assert_eq!(terminal.cursor_shape, CursorShape::Bar);
        assert_eq!(terminal.scrollback_lines, 10_000);
    }

    #[test]
    fn terminal_highlighting_defaults_on_and_explicit_disable_round_trips() {
        let terminal: TerminalSettings = read("[terminal]\nsemantic_highlighting = false\n");
        assert!(!terminal.semantic_highlighting);
        assert!(TerminalSettings::default().semantic_highlighting);
        let saved = toml::to_string(&terminal).unwrap();
        assert_eq!(
            toml::from_str::<TerminalSettings>(&saved).unwrap(),
            terminal
        );
    }

    #[test]
    fn unknown_keys_are_rejected_by_name() {
        let error = toml::from_str::<TerminalSettings>("font_szie = 14\n").unwrap_err();
        assert!(error.to_string().contains("font_szie"), "{error}");
    }

    #[test]
    fn sanitizing_clamps_and_cleans() {
        let mut terminal = TerminalSettings {
            font_size: Some(500.0),
            line_height: Some(f32::NAN),
            font_family: Some("   ".to_owned()),
            term: " ".to_owned(),
            ..TerminalSettings::default()
        };
        terminal.sanitize();
        assert_eq!(terminal.font_size, Some(72.0));
        assert_eq!(terminal.line_height, None);
        assert_eq!(terminal.font_family, None);
        assert_eq!(terminal.term, "xterm-256color");
        let mut appearance = AppearanceSettings {
            card_gap: 100.0,
            card_radius: f32::NAN,
            light_theme: Some("  ".into()),
            dark_theme: Some(" My Dark ".into()),
            ..AppearanceSettings::default()
        };
        appearance.sanitize();
        assert_eq!(appearance.card_gap, 24.0);
        assert_eq!(appearance.card_radius, 10.0);
        assert_eq!(appearance.light_theme, None);
        assert_eq!(appearance.dark_theme.as_deref(), Some("My Dark"));
    }

    #[test]
    fn layout_is_floating_unless_classic_is_chosen() {
        assert_eq!(AppearanceSettings::default().layout, UiLayout::Floating);
        let appearance: AppearanceSettings = read("[appearance]\nlayout = \"classic\"\n");
        assert_eq!(appearance.layout, UiLayout::Classic);
    }

    #[test]
    fn sanitizing_leaves_the_defaults_alone() {
        fn check<S: SettingsSection>() {
            let mut section = S::default();
            section.sanitize();
            assert_eq!(section, S::default(), "{}", S::KEY);
        }
        check::<AppearanceSettings>();
        check::<TerminalSettings>();
    }
}
