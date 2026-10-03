//! Design tokens: the single source of truth for how nocterm looks.
//!
//! [`tokens/default.toml`](DesignTokens::BUILTIN_SOURCE) is the product's
//! visual language written down once. This crate gives that file a type,
//! validates it, and layers a user's partial `theme.toml` over it. It knows
//! nothing about the UI toolkit: `nocterm-ui` projects the tokens onto the
//! component theme, so a toolkit change never touches the tokens.

mod color;
mod tokens;

pub use color::{Color, ParseColorError};
pub use tokens::{
    DesignTokens, Layout, Palette, Shape, TerminalColors, TokenError, Typography, UiColors,
};
