//! Pure grouped token data; its codec retains the original flat wire format.

use crate::Color;

mod codec;
mod schema;

/// One eight-color ANSI intensity bank, in index order.
#[derive(Debug, Clone, PartialEq)]
pub struct AnsiColors {
    pub black: Color,
    pub red: Color,
    pub green: Color,
    pub yellow: Color,
    pub blue: Color,
    pub magenta: Color,
    pub cyan: Color,
    pub white: Color,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TerminalColors {
    /// Terminal background. Unset: the interface background.
    pub background: Option<Color>,
    /// Default text colour. Unset: the interface foreground.
    pub foreground: Option<Color>,
    /// Cursor colour. Unset: the terminal foreground.
    pub cursor: Option<Color>,
    /// Selection highlight. Unset: the interface selection colour.
    pub selection: Option<Color>,
    pub normal: AnsiColors,
    pub bright: AnsiColors,
}

impl TerminalColors {
    /// The sixteen ANSI colours in index order.
    pub fn ansi(&self) -> [Color; 16] {
        [
            self.normal.black,
            self.normal.red,
            self.normal.green,
            self.normal.yellow,
            self.normal.blue,
            self.normal.magenta,
            self.normal.cyan,
            self.normal.white,
            self.bright.black,
            self.bright.red,
            self.bright.green,
            self.bright.yellow,
            self.bright.blue,
            self.bright.magenta,
            self.bright.cyan,
            self.bright.white,
        ]
    }
}
