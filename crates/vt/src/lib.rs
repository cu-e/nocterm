//! Terminal emulation: turns a byte stream into a grid, and input into bytes.
//!
//! This crate is the only place that knows the emulation library
//! (`alacritty_terminal`). Everything it exposes is expressed in its own
//! types, so the renderer and the session code do not change if the engine
//! underneath does.
//!
//! * [`Emulator`] holds the screen: feed it output, read a [`Frame`].
//! * [`Emulator::text`] reads the history and screen as bounded plain text.
//! * [`encode_key`], [`encode_mouse`], [`encode_paste`] and [`encode_focus`]
//!   produce what to send for user input, given the [`Modes`] the running
//!   program has set.

mod emulator;
mod keys;
mod mouse;
mod osc_guard;
mod paste;
mod search;
mod text;

pub use search::{
    MAX_REGEX_LINE_BYTES, MAX_SEARCH_QUERY, RegexWork, SearchBatch, SearchDirection, SearchMatch,
    SearchOptions, SearchPoint, SearchProgress, SearchResult, SearchScan,
};

pub use emulator::{
    Cell, CellPoint, Color, Cursor, CursorShape, Effect, Emulator, EmulatorOptions, Frame,
    LineMetadata, Modes, Palette, Rgb, Scroll, SelectionKind, Side, Style, TermSize,
};
pub use keys::{
    KeyEncoding, KeyEvent, KeyEventKind, KeyPress, KeyboardState, Modifiers, ModifyOtherKeys,
    encode_key, encode_key_event, encode_text_commit,
};
pub use mouse::{MouseButton, MouseEvent, MouseEventKind, encode_mouse};
pub use paste::{encode_focus, encode_paste};
pub use text::{TextQuery, TextTail};
