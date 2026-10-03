//! Terminal emulation: turns a byte stream into a grid, and input into bytes.
//!
//! This crate is the only place that knows the emulation library
//! (`alacritty_terminal`). Everything it exposes is expressed in its own
//! types, so the renderer and the session code do not change if the engine
//! underneath does.
//!
//! * [`Emulator`] holds the screen: feed it output, read a [`Frame`].
//! * [`encode_key`], [`encode_mouse`], [`encode_paste`] and [`encode_focus`]
//!   produce what to send for user input, given the [`Modes`] the running
//!   program has set.

mod emulator;
mod keys;
mod mouse;
mod paste;

pub use emulator::{
    Cell, CellPoint, Color, Cursor, CursorShape, Effect, Emulator, EmulatorOptions, Frame,
    LineMetadata, Modes, Palette, Rgb, Scroll, SelectionKind, Side, Style, TermSize,
};
pub use keys::{KeyPress, Modifiers, encode_key};
pub use mouse::{MouseButton, MouseEvent, MouseEventKind, encode_mouse};
pub use paste::{encode_focus, encode_paste};
