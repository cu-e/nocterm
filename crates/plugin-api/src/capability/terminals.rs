//! `terminals.open`, `terminals.write` and `terminals.bar`: terminal tabs a plugin drives.
//!
//! A plugin terminal is a tab like any other terminal of the window: the user reads and selects
//! its text, and attaches it to an AI chat, whose agent reads it and types into it. The host
//! echoes what is typed and hands each finished line to the plugin as a `terminal-line` event;
//! the plugin writes text back. Nothing runs on this computer. A tab may carry a bar of the
//! plugin along one of its edges: the contribution the tab reports to draws it, told which
//! terminal it draws for.

use alloc::string::String;
use serde::{Deserialize, Serialize};

use super::{Capability, sealed};
use crate::{Edge, LimitError, LocalId, error::key_syntax, sanitize_display};
use nocterm_plugin_model::sanitize_text;

/// Longest terminal key.
pub const MAX_TERMINAL_KEY: usize = 32;
/// Longest terminal title, in characters.
pub const MAX_TERMINAL_TITLE: usize = 48;
/// Most bytes one write carries.
pub const MAX_WRITE: usize = 64 * 1024;
/// Most terminals one plugin keeps open.
pub const MAX_TERMINALS: usize = 4;
/// Longest line the host hands the plugin, in characters; typing stops there.
pub const MAX_LINE: usize = 1024;

/// Opens a terminal tab, or shows it if it is open. Lines typed into it are reported to `local`,
/// one of the plugin's own contributions. With `files` the plugin serves the tab's files, which
/// needs `terminals.files` (see [`super::files`]).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct Open {
    pub local: LocalId,
    pub terminal: String,
    pub title: String,
    #[serde(default)]
    pub files: bool,
}

impl Open {
    pub fn check(&self) -> Result<(), LimitError> {
        key_syntax(&self.terminal, MAX_TERMINAL_KEY, "terminal")?;
        if self.title.chars().count() > MAX_TERMINAL_TITLE {
            return Err(LimitError::TooLong {
                field: "title",
                max: MAX_TERMINAL_TITLE,
            });
        }
        if sanitize_display(&self.title, MAX_TERMINAL_TITLE) != self.title {
            return Err(LimitError::Unsanitized { field: "title" });
        }
        Ok(())
    }
}

/// Writes text to an open terminal of the plugin. The host keeps printable text, line breaks,
/// tabs and colours, and drops every other control sequence.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct Write {
    pub terminal: String,
    pub text: String,
}

impl Write {
    pub fn check(&self) -> Result<(), LimitError> {
        key_syntax(&self.terminal, MAX_TERMINAL_KEY, "terminal")?;
        if self.text.len() > MAX_WRITE {
            return Err(LimitError::TooLong {
                field: "text",
                max: MAX_WRITE,
            });
        }
        Ok(())
    }
}

/// Takes the plugin's toolbar for its own tabs off one of its open terminal tabs (`edge: None`)
/// or puts it back (any edge). Where the toolbar runs is the window layout's: the manifest's
/// `edge` and `shown` suggest it and the user moves or hides it, so the edge named here is not
/// read. The toolbar draws the bar: the host asks it for a view with the snapshot's `terminal`
/// set, and reports what the user does in the bar to it the same way.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct Bar {
    pub terminal: String,
    #[serde(default)]
    pub edge: Option<Edge>,
}

impl Bar {
    pub fn check(&self) -> Result<(), LimitError> {
        key_syntax(&self.terminal, MAX_TERMINAL_KEY, "terminal")
    }
}

/// Marker of `terminals.open`.
pub struct OpenTerminal;

impl sealed::Sealed for OpenTerminal {}

impl Capability for OpenTerminal {
    const ID: &'static str = "terminals.open";
    type Value = Open;
}

/// Marker of `terminals.write`.
pub struct WriteTerminal;

impl sealed::Sealed for WriteTerminal {}

impl Capability for WriteTerminal {
    const ID: &'static str = "terminals.write";
    type Value = Write;
}

/// Marker of `terminals.bar`.
pub struct ShowTerminalBar;

impl sealed::Sealed for ShowTerminalBar {}

impl Capability for ShowTerminalBar {
    const ID: &'static str = "terminals.bar";
    type Value = Bar;
}

/// The bytes a terminal shows for `text`: printable characters, `\n` as a line break, `\t`,
/// and SGR colour sequences (`ESC [ digits ; … m`). Every other control character and escape
/// sequence is dropped whole, so a plugin cannot set the title, move the cursor, write the
/// clipboard, open a link or answer queries through the screen.
pub fn terminal_output(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + text.len() / 16);
    let mut run = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if !c.is_control() {
            run.push(c);
            continue;
        }
        // A run of plain text loses invisible and direction characters as view text does.
        out.push_str(&sanitize_text(&run));
        run.clear();
        match c {
            '\n' => out.push_str("\r\n"),
            '\t' => out.push('\t'),
            '\u{1b}' => match chars.next() {
                Some('[') => keep_colour(&mut chars, &mut out),
                Some(']' | 'P' | '_' | '^' | 'X') => skip_string(&mut chars),
                _ => {}
            },
            '\u{9b}' => {
                let _ = control_sequence(&mut chars);
            }
            '\u{90}' | '\u{98}' | '\u{9d}' | '\u{9e}' | '\u{9f}' => skip_string(&mut chars),
            _ => {}
        }
    }
    out.push_str(&sanitize_text(&run));
    out
}

/// Reads a control sequence after its introducer: parameters, intermediates and the final byte.
/// Returns the parameters and the final byte when it is well formed.
fn control_sequence(
    chars: &mut core::iter::Peekable<core::str::Chars<'_>>,
) -> Option<(String, char)> {
    let mut params = String::new();
    let mut plain = true;
    for next in chars.by_ref() {
        match next {
            '0'..='9' | ';' if plain => {
                if params.len() < 32 {
                    params.push(next);
                }
            }
            '\u{30}'..='\u{3f}' | '\u{20}'..='\u{2f}' => plain = false,
            '\u{40}'..='\u{7e}' => return plain.then_some((params, next)),
            _ => return None,
        }
    }
    None
}

fn keep_colour(chars: &mut core::iter::Peekable<core::str::Chars<'_>>, out: &mut String) {
    if let Some((params, 'm')) = control_sequence(chars) {
        out.push_str("\u{1b}[");
        out.push_str(&params);
        out.push('m');
    }
}

/// Skips a string sequence (OSC, DCS, APC, PM, SOS) up to its BEL or string terminator.
fn skip_string(chars: &mut core::iter::Peekable<core::str::Chars<'_>>) {
    while let Some(next) = chars.next() {
        match next {
            '\u{7}' | '\u{9c}' => return,
            '\u{1b}' if chars.peek() == Some(&'\\') => {
                chars.next();
                return;
            }
            _ => {}
        }
    }
}
