//! The frame a plugin cannot style: marker, label and the fault chip.

use gpui::prelude::FluentBuilder as _;
use gpui::{AnyElement, Div, InteractiveElement, ParentElement, SharedString, Styled, div, px};

use crate::ViewStyle;

/// Where the view sits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrameKind {
    /// A status-bar widget: the marker leads the content, a developer session adds a DEV chip,
    /// and the application attaches the label to the widget as a tooltip.
    Bar,
    /// A panel: a header line carries the marker and the label above the content.
    Panel,
}

/// The width of the fault chip, in pixels.
pub(crate) const FAULT_WIDTH: f32 = 72.;
const MARKER_SIZE: f32 = 8.;

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

/// The chip that tells a bar item from a developer session apart, which has no header line to
/// carry the label's DEV suffix.
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
/// plugin: a bar item gets a DEV chip, and a panel's `label` already ends in DEV.
pub(crate) fn framed(
    kind: FrameKind,
    style: &ViewStyle,
    label: SharedString,
    dev: bool,
    body: AnyElement,
) -> Div {
    match kind {
        FrameKind::Bar => div()
            .flex()
            .flex_row()
            .items_center()
            .gap_1()
            .min_w_0()
            .child(marker(style))
            .when(dev, |frame| frame.child(dev_chip(style)))
            .child(div().min_w_0().overflow_hidden().child(body)),
        FrameKind::Panel => div()
            .flex()
            .flex_col()
            .size_full()
            .gap_2()
            .p_2()
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_1()
                    .text_xs()
                    .text_color(style.muted)
                    .child(marker(style))
                    .child(
                        div()
                            .truncate()
                            .child(label)
                            .debug_selector(|| "plugin-label".to_string()),
                    ),
            )
            .child(div().flex_1().min_h_0().child(body)),
    }
}
