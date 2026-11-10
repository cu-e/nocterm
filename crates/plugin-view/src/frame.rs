//! The frame a plugin cannot style: the marker of a bar item or a terminal's bar, the label and
//! the fault chip.

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Div, InteractiveElement, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, div, px,
};

use crate::ViewStyle;

/// Where the view sits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrameKind {
    /// A status-bar widget: the marker leads the content, a developer session adds a DEV chip,
    /// and the application attaches the label to the widget as a tooltip.
    Bar,
    /// A panel: the shell writes the label beside the panel's title; the frame scrolls the
    /// content.
    Panel,
    /// The bar of a plugin's terminal tab along its top or bottom: the marker leads and the
    /// content fills the width, so a spacer moves what follows it to the end.
    Strip,
    /// The bar of a plugin's terminal tab along a side: the marker on top, the content one under
    /// the other.
    Rail,
}

impl FrameKind {
    /// Whether the view is drawn as a bar: compact rows, sections without headings, text cut
    /// to one line.
    pub(crate) fn is_bar(self) -> bool {
        !matches!(self, Self::Panel)
    }

    /// Whether the roots of the view run one under the other.
    pub(crate) fn is_column(self) -> bool {
        matches!(self, Self::Panel | Self::Rail)
    }
}

/// The width of the fault chip, in pixels.
pub(crate) const FAULT_WIDTH: f32 = 72.;
const MARKER_SIZE: f32 = 8.;
/// The length and the thickness of the line that marks a strip or a rail.
const LINE_MARKER: (f32, f32) = (14., 2.);

/// "Plugin · ⟨name⟩", the label of every user plugin item; a developer session adds " · DEV".
pub(crate) fn label(name: &str, dev: bool) -> SharedString {
    if dev {
        format!("Plugin \u{b7} {name} \u{b7} DEV").into()
    } else {
        format!("Plugin \u{b7} {name}").into()
    }
}

fn marker(style: &ViewStyle) -> Div {
    div()
        .flex_none()
        .size(px(MARKER_SIZE))
        .rounded_sm()
        .border_2()
        .border_color(style.marker)
        .debug_selector(|| "plugin-marker".to_string())
}

/// The quiet mark of a strip (a short upright line) or a rail (a short flat line): it tells the
/// plugin's bar from the application's without drawing the eye.
fn line_marker(style: &ViewStyle, upright: bool) -> Div {
    let (long, thin) = LINE_MARKER;
    let line = div()
        .flex_none()
        .rounded_full()
        .bg(style.muted.opacity(0.55));
    let line = if upright {
        line.w(px(thin)).h(px(long))
    } else {
        line.w(px(long)).h(px(thin))
    };
    line.debug_selector(|| "plugin-marker".to_string())
}

/// The chip that tells a bar item from a developer session apart, which has no title to carry
/// the label's DEV suffix.
fn dev_chip(style: &ViewStyle) -> Div {
    div()
        .flex_none()
        .px_1()
        .rounded_sm()
        .border_1()
        .border_color(style.caution)
        .text_color(style.caution)
        .text_xs()
        .child("DEV")
        .debug_selector(|| "plugin-dev".to_string())
}

/// A chip of fixed width in place of a view the host refused to draw.
pub(crate) fn fault_chip(style: &ViewStyle) -> Div {
    div()
        .flex_none()
        .w(px(FAULT_WIDTH))
        .px_1()
        .rounded_sm()
        .border_1()
        .border_color(style.caution)
        .text_color(style.caution)
        .text_xs()
        .truncate()
        .child("plugin fault")
        .debug_selector(|| "plugin-fault".to_string())
}

/// Wraps the body in the frame of its kind. `dev` marks the frame of a developer session's
/// plugin: a bar item gets a DEV chip. A panel's provenance (the label) is written by the shell
/// beside the panel's title, outside the view, so the panel frame only scrolls the body.
pub(crate) fn framed(
    kind: FrameKind,
    style: &ViewStyle,
    dev: bool,
    body: AnyElement,
) -> AnyElement {
    match kind {
        FrameKind::Bar => div()
            .flex()
            .flex_row()
            .items_center()
            .gap_1()
            .min_w_0()
            .child(marker(style))
            .when(dev, |frame| frame.child(dev_chip(style)))
            .child(div().min_w_0().overflow_hidden().child(body))
            .into_any_element(),
        FrameKind::Strip => div()
            .flex()
            .flex_row()
            .items_center()
            .gap_2()
            .size_full()
            .min_w_0()
            .px_2()
            .text_xs()
            .debug_selector(|| "plugin-strip".to_string())
            .child(line_marker(style, true))
            .when(dev, |frame| frame.child(dev_chip(style)))
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .child(body),
            )
            .into_any_element(),
        FrameKind::Rail => div()
            .flex()
            .flex_col()
            .items_center()
            .gap_1()
            .size_full()
            .min_h_0()
            .py_2()
            .text_xs()
            .debug_selector(|| "plugin-rail".to_string())
            .child(line_marker(style, false))
            .when(dev, |frame| frame.child(dev_chip(style)))
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .overflow_hidden()
                    .child(body),
            )
            .into_any_element(),
        FrameKind::Panel => div()
            .id("plugin-panel-body")
            .debug_selector(|| "plugin-panel".to_string())
            .flex()
            .flex_col()
            .size_full()
            .overflow_y_scroll()
            .px_1()
            .pb_2()
            .child(body)
            .into_any_element(),
    }
}
