// Added by Nocterm contributors; see NOCTERM.md. Licensed under Apache-2.0.
//! Floating cards: a window's regions drawn as rounded, raised surfaces on a
//! canvas, with a gap between them.
//!
//! The application decides whether its window floats and with what geometry
//! by installing [`FloatingCards`] as a global; without it every component
//! keeps its edge-to-edge appearance. The dock skin, tab bars and resize
//! handles read the same global, so one setting reframes the whole window.
//!
//! Every card keeps half the gap clear around itself ([`FloatingCards::margin`]),
//! so two neighbours are a full gap apart without either knowing about the
//! other, and the divider a split draws between them lands in the middle of
//! the gap. A container that holds cards pads its edge by the same half gap.
//!
//! GPUI clips descendants to rectangles, so a card cannot rely on its
//! rounded corners to hide what its content paints there. A card therefore
//! ends with a [`FloatingCards::corner_mask`]: the canvas painted back over
//! whatever spilled past the curve, and the card's outline drawn again on
//! top. The mask paints only; it takes no part in hit testing.

use gpui::{
    AnyElement, App, BoxShadow, Div, Global, Hsla, IntoElement, ParentElement as _, Pixels,
    Styled as _, div, prelude::FluentBuilder as _, px,
};

/// How far a card's shadow may reach past its edge. It must stay inside the
/// card's own margin: the containers around a card clip to their bounds, and
/// a shadow cut off halfway across a gap reads as a rendering fault.
const SHADOW_REACH: Pixels = px(3.);

/// The geometry and colours of a floating window, as an application global.
///
/// Absent: the classic edge-to-edge layout. See the [module docs](self).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FloatingCards {
    /// Space between two cards, and between a card and the window's edge.
    pub gap: Pixels,
    /// Corner radius of a card.
    pub radius: Pixels,
    /// The backdrop the cards float on.
    pub canvas: Hsla,
    /// The fill of a card whose content does not paint its own.
    pub surface: Hsla,
    /// The hairline around each card.
    pub border: Hsla,
    /// Ink of the shadow under each card; `None` draws cards flat.
    pub shadow: Option<Hsla>,
}

impl Global for FloatingCards {}

impl FloatingCards {
    /// The floating geometry in force, or `None` for the classic layout.
    pub fn get(cx: &App) -> Option<Self> {
        cx.try_global::<Self>().copied()
    }

    /// The space a card keeps clear on each side: half the gap.
    pub fn margin(&self) -> Pixels {
        self.gap / 2.
    }

    /// The radius of something inset by `inset` inside a card, so that its
    /// corners stay concentric with the card's. Never below `floor`.
    pub fn inner_radius(&self, inset: Pixels, floor: Pixels) -> Pixels {
        (self.radius - inset).max(floor)
    }

    /// Styles `surface` as a card: its fill, outline, corners and shadow.
    ///
    /// The outline is one pixel and part of the box, so content laid out
    /// inside starts one pixel in on every side, clear of it.
    pub fn surface(&self, surface: Div, fill: Hsla) -> Div {
        surface
            .rounded(self.radius)
            .bg(fill)
            .border_1()
            .border_color(self.border)
            .when_some(self.shadow(), |surface, shadow| surface.shadow(shadow))
    }

    /// The shadow under a card, when one fits inside its margin.
    fn shadow(&self) -> Option<Vec<BoxShadow>> {
        let ink = self.shadow?;
        // One pixel down and two of blur: three pixels at most.
        (self.margin() >= SHADOW_REACH)
            .then(|| vec![BoxShadow::new(px(0.), px(1.), ink).blur_radius(px(2.))])
    }

    /// A whole card around `content`: its margin, its surface, and the mask
    /// that keeps `content` inside the rounded corners. Fills its parent.
    pub fn card(&self, content: impl IntoElement) -> Div {
        div().size_full().p(self.margin()).child(
            self.surface(div(), self.surface)
                .relative()
                .size_full()
                .overflow_hidden()
                .child(div().size_full().child(content))
                .child(self.corner_mask(true)),
        )
    }

    /// The overlay that rounds a card's corners over its content.
    ///
    /// Place it last, absolutely, in an element whose box is the inside of a
    /// card's outline and which clips to its bounds (as a card from
    /// [`Self::card`] does, and as a dock's content region does). With `top`
    /// unset only the bottom corners are masked, for a region that starts
    /// below the top of its card.
    pub fn corner_mask(&self, top: bool) -> AnyElement {
        if self.radius <= px(0.) {
            return div().into_any_element();
        }
        // The canvas is a ring around the card whose inner edge is the card's
        // own curve: GPUI rounds a border's inner edge to the outer radius
        // less the border width. The ring reaches a radius past the card, which
        // the clip around the mask then trims back to the corners.
        let reach = self.radius;
        // A region that starts below the card's top pushes the mask's own top
        // corners above it, where the clip removes them.
        let above = if top { px(1.) } else { self.radius + px(2.) };
        div()
            .absolute()
            .top(-above)
            .left(-px(1.))
            .right(-px(1.))
            .bottom(-px(1.))
            .child(
                div()
                    .absolute()
                    .top(-reach)
                    .left(-reach)
                    .right(-reach)
                    .bottom(-reach)
                    .rounded(self.radius + reach)
                    .border(reach)
                    .border_color(self.canvas),
            )
            .child(
                div()
                    .absolute()
                    .size_full()
                    .rounded(self.radius)
                    .border_1()
                    .border_color(self.border),
            )
            .into_any_element()
    }
}
