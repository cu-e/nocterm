//! Terminal emulation: turns a byte stream into a grid, and input into bytes.
//!
//! Owns: screen emulation, selection, bounded text/search and negotiated input encoding.
//! Does not own: PTYs, transport I/O, window rendering or application scheduling.
//! Ports used/provided: none. Settings section: none. Globals: none.
//! Invariants: input follows the negotiated terminal modes; snapshots expose this
//! crate's types; searches report typed limits instead of truncating silently,
//! and cancellation never publishes matches from an older generation.
//!
//! This crate is the only place that knows the emulation library
//! (`alacritty_terminal`). Everything it exposes is expressed in its own
//! types, so the renderer and the session code do not change if the engine
//! underneath does.
//!
//! * [`Emulator`] holds the screen: feed it output, read a [`Frame`].
//! * [`Emulator::text`] delegates bounded history/screen reads to a borrowed text reader.
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
    MAX_REGEX_LINE_BYTES, MAX_SEARCH_QUERY, RegexWork, SearchBatch, SearchDirection, SearchError,
    SearchMatch, SearchOptions, SearchPoint, SearchProgress, SearchResult, SearchScan,
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
