//! Design tokens: the single source of truth for how nocterm looks.
//!
//! Owns: typed visual tokens, color validation and partial token overrides.
//! Does not own: filesystem reads, user settings, toolkit themes or rendering.
//! Ports used/provided: none. Settings section: none. Globals: none.
//! Invariants: overrides validate against the token schema; built-in tokens remain
//! the fallback for unspecified values.
//!
//! [`tokens/default.toml`](DesignTokens::BUILTIN_SOURCE) is the product's
//! visual language written down once. This crate gives that file a type,
//! groups resize bounds and ANSI intensity banks while retaining its flat wire schema,
//! validates it, and layers a user's partial `theme.toml` over it. It knows
//! nothing about the UI toolkit: `nocterm-ui` projects the tokens onto the
//! component theme, so a toolkit change never touches the tokens.

mod color;
mod tokens;

pub use color::{Color, ParseColorError};
pub use tokens::{
    AnsiColors, DesignTokens, Layout, Palette, Shape, TerminalColors, TokenError, Typography,
    UiColors, WidthBounds,
};
