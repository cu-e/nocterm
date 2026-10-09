//! How a terminal grid looks, resolved from tokens, settings and theme.

use gpui_kit::{App, Hsla, Pixels, Rems, Rgba, SharedString, component::ActiveTheme, px, rems};
use nocterm_design::Color;

use crate::{ActiveDesign, SettingsExt};

/// Everything a terminal view needs to paint, already resolved: a setting
/// wins over a design token, which wins over the component theme.
#[derive(Clone, Debug, PartialEq)]
pub struct TerminalStyle {
    pub font_family: SharedString,
    pub font_size: Pixels,
    /// Line height as a multiple of the font size.
    pub line_height: f32,
    pub padding: Rems,
    pub show_timestamps: bool,
    pub show_line_numbers: bool,
    pub semantic_highlighting: bool,
    pub gutter_foreground: Color,
    pub foreground: Color,
    pub background: Color,
    pub cursor: Color,
    pub selection: Color,
    /// The sixteen ANSI colours, in index order.
    pub ansi: [Color; 16],
}

impl TerminalStyle {
    pub fn current(cx: &App) -> Self {
        let tokens = cx.design();
        let settings = cx.setting::<nocterm_settings::TerminalSettings>();
        let theme = cx.theme();
        let colors = &tokens.palette(theme.is_dark()).terminal;

        let foreground = colors.foreground.unwrap_or_else(|| color(theme.foreground));
        Self {
            font_family: settings
                .font_family
                .clone()
                .or_else(|| tokens.typography.mono_font.clone())
                .map_or_else(|| theme.mono_font_family.clone(), SharedString::from),
            font_size: px(settings
                .font_size
                .unwrap_or_else(|| tokens.typography.mono_size())),
            line_height: settings
                .line_height
                .unwrap_or(tokens.typography.terminal_line_height),
            padding: rems(tokens.layout.terminal_padding),
            show_timestamps: settings.show_timestamps,
            show_line_numbers: settings.show_line_numbers,
            semantic_highlighting: settings.semantic_highlighting,
            gutter_foreground: color(theme.muted_foreground),
            foreground,
            background: colors.background.unwrap_or_else(|| color(theme.background)),
            cursor: colors.cursor.unwrap_or(foreground),
            selection: colors.selection.unwrap_or_else(|| color(theme.selection)),
            ansi: colors.ansi(),
        }
    }
}

/// A token colour as GPUI paints it.
pub fn hsla(color: Color) -> Hsla {
    Rgba {
        r: f32::from(color.r) / 255.0,
        g: f32::from(color.g) / 255.0,
        b: f32::from(color.b) / 255.0,
        a: f32::from(color.a) / 255.0,
    }
    .into()
}

pub(crate) fn color(hsla: Hsla) -> Color {
    let rgba = Rgba::from(hsla);
    let channel = |value: f32| (value.clamp(0.0, 1.0) * 255.0).round() as u8;
    Color {
        r: channel(rgba.r),
        g: channel(rgba.g),
        b: channel(rgba.b),
        a: channel(rgba.a),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colours_survive_the_round_trip_through_gpui() {
        for value in [
            Color::rgb(0, 0, 0),
            Color::rgb(255, 255, 255),
            Color::rgb(0xcd, 0x31, 0x31),
            Color {
                r: 9,
                g: 105,
                b: 218,
                a: 0x40,
            },
        ] {
            assert_eq!(color(hsla(value)), value);
        }
    }
}
