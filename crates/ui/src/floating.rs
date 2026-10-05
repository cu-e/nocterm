//! The floating layout: the appearance settings and design tokens projected
//! onto the toolkit's [`FloatingCards`] global.
//!
//! The dock, its tab bars and resize handles, and the workspace frame all
//! read that one global, so choosing the layout here reframes the window
//! without any view knowing where the choice came from.

use gpui_kit::{
    App, Hsla,
    component::{ActiveTheme as _, floating::FloatingCards},
    hsla as gpui_hsla, px,
};
use nocterm_settings::UiLayout;

use crate::{ActiveDesign as _, ActiveSettings as _, hsla, terminal_style::color};

/// How much darker than the cards the canvas is, as a share of their
/// lightness for a dark surface and as a step down for a light one.
const DARK_CANVAS_SHARE: f32 = 0.55;
const LIGHT_CANVAS_STEP: f32 = 0.07;
/// A dark surface this close to black has no darker shade to recede into;
/// its canvas is a shade lighter instead.
const NEAR_BLACK: f32 = 0.06;

/// Installs the floating geometry the settings ask for, or removes it for the
/// classic layout. Called with the theme already in place.
pub(crate) fn apply_floating(cx: &mut App) {
    match cards(cx) {
        Some(cards) => cx.set_global(cards),
        None if cx.has_global::<FloatingCards>() => {
            cx.remove_global::<FloatingCards>();
        }
        None => {}
    }
}

/// The floating geometry for the current settings, tokens and theme.
fn cards(cx: &App) -> Option<FloatingCards> {
    let appearance = &cx.settings().appearance;
    if appearance.layout != UiLayout::Floating {
        return None;
    }
    let theme = cx.theme();
    let dark = theme.is_dark();
    let palette = cx.design().palette(dark);
    // Cards wear the terminal's background, so a terminal fills its card
    // without a seam between the tab bar and the grid.
    let surface = hsla(
        palette
            .terminal
            .background
            .unwrap_or_else(|| color(theme.background)),
    );
    Some(FloatingCards {
        gap: px(appearance.card_gap),
        radius: px(appearance.card_radius),
        canvas: palette
            .canvas
            .map(hsla)
            .unwrap_or_else(|| canvas_for(surface, dark)),
        surface,
        border: theme.border,
        shadow: theme
            .shadow
            .then(|| gpui_hsla(0., 0., 0., if dark { 0.35 } else { 0.08 })),
    })
}

/// A backdrop that sets cards of `surface` apart: a shade darker, or for a
/// near-black dark surface, a shade lighter.
fn canvas_for(surface: Hsla, dark: bool) -> Hsla {
    let l = surface.l;
    let l = if !dark {
        (l - LIGHT_CANVAS_STEP).max(0.)
    } else if l >= NEAR_BLACK {
        l * DARK_CANVAS_SHARE
    } else {
        l + NEAR_BLACK * 0.5
    };
    Hsla {
        l,
        a: 1.,
        ..surface
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_canvas_recedes_behind_the_cards() {
        let dark = gpui_hsla(0., 0., 0.094, 1.);
        assert!(canvas_for(dark, true).l < dark.l);

        let light = gpui_hsla(0., 0., 1., 1.);
        let canvas = canvas_for(light, false);
        assert!(canvas.l < light.l && canvas.l > 0.9);
    }

    #[test]
    fn a_black_surface_gets_a_lighter_canvas_rather_than_none() {
        let black = gpui_hsla(0., 0., 0., 1.);
        assert!(canvas_for(black, true).l > 0.);
    }

    #[test]
    fn the_canvas_is_opaque() {
        let translucent = gpui_hsla(0.6, 0.2, 0.3, 0.5);
        assert_eq!(canvas_for(translucent, true).a, 1.);
    }
}
