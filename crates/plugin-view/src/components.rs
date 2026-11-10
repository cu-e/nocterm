//! The measures of the components, as the application's own panels draw theirs: a section
//! heading, a list row, a small icon button, a compact button, a badge, a caption and a field.
//!
//! These functions only style; the painter decides what is a control and what it reports.

use gpui::{
    AnyElement, Div, FontWeight, Hsla, IntoElement, ParentElement, SharedString, Styled, div, px,
};

use crate::ViewStyle;

/// The corner radius of rows and controls.
pub(crate) const RADIUS: f32 = 6.;
/// The side of a small icon button.
pub(crate) const ICON_BUTTON: f32 = 20.;
/// The height of a compact button.
pub(crate) const BUTTON_HEIGHT: f32 = 22.;
/// The size of the icon that leads a row.
pub(crate) const ROW_ICON: f32 = 14.;
/// The size of the chevron of a foldable section and of an icon button's icon.
pub(crate) const SMALL_ICON: f32 = 12.;

/// A box of a fixed size that centres an icon drawn at the text size around it.
pub(crate) fn icon_box(size: f32, color: Hsla, icon: AnyElement) -> Div {
    div()
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .size(px(size))
        .text_size(px(size))
        .text_color(color)
        .child(icon)
}

/// The heading of a section: the title in capitals, an optional chevron before it and an
/// optional count at the end, in the small semibold muted type of the application's headings.
pub(crate) fn section_heading(
    style: &ViewStyle,
    title: &str,
    count: Option<&str>,
    chevron: Option<AnyElement>,
) -> Div {
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap_1()
        .px_2()
        .py_1()
        .rounded(px(RADIUS))
        .text_xs()
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(style.muted)
        .children(chevron.map(|chevron| icon_box(SMALL_ICON, style.muted, chevron)))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .child(SharedString::from(title.to_uppercase())),
        )
        .children(count.map(|count| {
            div()
                .flex_none()
                .child(SharedString::from(count.to_owned()))
        }))
}

/// The body of a list row: the leading icon, the title over the muted detail line, then the
/// row's own children at the end.
pub(crate) fn item_body(
    style: &ViewStyle,
    title: &str,
    detail: Option<&str>,
    icon: Option<(AnyElement, Hsla)>,
    trailing: Vec<AnyElement>,
) -> Div {
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap_2()
        .w_full()
        .min_w_0()
        .children(icon.map(|(icon, color)| icon_box(ROW_ICON, color, icon)))
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .child(
                    div()
                        .text_sm()
                        .text_color(style.text)
                        .truncate()
                        .child(SharedString::from(title.to_owned())),
                )
                .children(detail.map(|detail| {
                    div()
                        .text_xs()
                        .text_color(style.muted)
                        .truncate()
                        .child(SharedString::from(detail.to_owned()))
                })),
        )
        .when_children(trailing)
}

/// Extends a row with its trailing children, if it has any.
trait WhenChildren {
    fn when_children(self, children: Vec<AnyElement>) -> Self;
}

impl WhenChildren for Div {
    fn when_children(self, children: Vec<AnyElement>) -> Self {
        if children.is_empty() {
            self
        } else {
            self.child(
                div()
                    .flex()
                    .flex_row()
                    .flex_none()
                    .items_center()
                    .gap_0p5()
                    .children(children),
            )
        }
    }
}

/// A label and its value on one line.
pub(crate) fn field(style: &ViewStyle, label: &str, value: &str, color: Hsla) -> AnyElement {
    div()
        .flex()
        .flex_row()
        .items_baseline()
        .gap_2()
        .min_w_0()
        .text_xs()
        .child(
            div()
                .flex_none()
                .text_color(style.muted)
                .child(SharedString::from(label.to_owned())),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_color(color)
                .child(SharedString::from(value.to_owned())),
        )
        .into_any_element()
}

/// A line of small muted text.
pub(crate) fn caption(style: &ViewStyle, text: &str) -> AnyElement {
    div()
        .min_w_0()
        .text_xs()
        .text_color(style.muted)
        .child(SharedString::from(text.to_owned()))
        .into_any_element()
}

/// A compact badge: the tone's colour on a faint fill of it.
pub(crate) fn badge(text: &str, color: Hsla) -> AnyElement {
    div()
        .flex_none()
        .px_1p5()
        .rounded(px(4.))
        .bg(color.opacity(0.15))
        .text_color(color)
        .text_xs()
        .font_weight(FontWeight::MEDIUM)
        .child(SharedString::from(text.to_owned()))
        .into_any_element()
}
