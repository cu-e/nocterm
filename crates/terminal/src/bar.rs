//! A bar along one edge of an attached terminal's tab, drawn by whoever opened the tab.
//!
//! The opener keeps a [`TerminalBar`] and fills, moves or empties it while the tab is open; the
//! tab draws what it holds around the screen and redraws when it changes. The tab owns the bar,
//! so the bar and the view in it go when the tab closes.
use gpui_kit::{
    AnyElement, AnyView, App, Context, IntoElement,
    component::{ActiveTheme as _, h_flex, v_flex},
    div,
    prelude::*,
    px,
};

/// The height of a bar along the top or the bottom of a tab.
const BAR_HEIGHT: f32 = 28.;
/// The narrowest and the widest a bar along a side of a tab is.
const RAIL_WIDTH: (f32, f32) = (36., 240.);

/// The edge of a tab a bar runs along.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BarEdge {
    Top,
    Bottom,
    Left,
    Right,
}

impl BarEdge {
    fn is_side(self) -> bool {
        matches!(self, Self::Left | Self::Right)
    }
}

/// What an attached terminal's tab shows along one of its edges, if anything.
#[derive(Default)]
pub struct TerminalBar {
    shown: Option<(BarEdge, AnyView)>,
}

impl TerminalBar {
    /// Shows `view` along `edge`, in place of what the bar showed.
    pub fn show(&mut self, edge: BarEdge, view: AnyView, cx: &mut Context<Self>) {
        self.shown = Some((edge, view));
        cx.notify();
    }

    /// Takes the bar away.
    pub fn hide(&mut self, cx: &mut Context<Self>) {
        if self.shown.take().is_some() {
            cx.notify();
        }
    }

    /// The edge the bar runs along, while it shows.
    pub fn edge(&self) -> Option<BarEdge> {
        self.shown.as_ref().map(|(edge, _)| *edge)
    }

    /// The view the bar shows, so an opener can move it to another edge as it is.
    pub fn view(&self) -> Option<&AnyView> {
        self.shown.as_ref().map(|(_, view)| view)
    }
}

/// `screen` with the bar along its edge, or `screen` alone while the bar shows nothing.
pub(crate) fn around(screen: AnyElement, bar: &TerminalBar, cx: &App) -> AnyElement {
    let Some((edge, view)) = bar.shown.clone() else {
        return screen;
    };
    let theme = cx.theme();
    let strip = if edge.is_side() {
        v_flex()
            .h_full()
            .min_w(px(RAIL_WIDTH.0))
            .max_w(px(RAIL_WIDTH.1))
            .min_h_0()
    } else {
        h_flex().w_full().h(px(BAR_HEIGHT)).min_w_0()
    };
    let strip = match edge {
        BarEdge::Top => strip.border_b_1(),
        BarEdge::Bottom => strip.border_t_1(),
        BarEdge::Left => strip.border_r_1(),
        BarEdge::Right => strip.border_l_1(),
    }
    .id("terminal-bar")
    .debug_selector(|| "terminal-bar".to_string())
    .flex_none()
    .overflow_hidden()
    .border_color(theme.border)
    .bg(theme.tab_bar)
    .child(view);
    let frame = if edge.is_side() {
        h_flex().size_full().min_h_0()
    } else {
        v_flex().size_full().min_w_0()
    };
    let screen = div().flex_1().min_w_0().min_h_0().size_full().child(screen);
    match edge {
        BarEdge::Top | BarEdge::Left => frame.child(strip).child(screen),
        BarEdge::Bottom | BarEdge::Right => frame.child(screen).child(strip),
    }
    .into_any_element()
}
