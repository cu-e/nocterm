//! Small pieces the steps of the sign-in dialog are built from.

use gpui_kit::{
    App, IntoElement, SharedString,
    component::{ActiveTheme as _, StyledExt as _, v_flex},
    div,
    prelude::*,
};
use nocterm_hubs_model::Fingerprint;

/// A labelled field.
pub(crate) fn field(label: &'static str, input: impl IntoElement) -> impl IntoElement {
    v_flex()
        .gap_1()
        .child(div().text_xs().font_semibold().child(label))
        .child(input)
}

pub(crate) fn title(text: String, _: &App) -> impl IntoElement {
    div().font_semibold().child(text)
}

/// A quiet sentence.
pub(crate) fn note(text: impl Into<SharedString>, cx: &App) -> impl IntoElement {
    div()
        .text_sm()
        .text_color(cx.theme().muted_foreground)
        .child(text.into())
}

/// A sentence about what went wrong.
pub(crate) fn problem(text: String, cx: &App) -> impl IntoElement {
    div().text_sm().text_color(cx.theme().danger).child(text)
}

/// A fingerprint in groups of four bytes, which are easier to compare by eye.
pub(crate) fn fingerprint_block(
    label: &'static str,
    fingerprint: &Fingerprint,
    cx: &App,
) -> impl IntoElement {
    let groups = fingerprint.groups();
    v_flex()
        .gap_1()
        .child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(label),
        )
        .child(
            div()
                .font_family(cx.theme().mono_font_family.clone())
                .text_sm()
                .child(format!(
                    "{}\n{}",
                    groups[..4].join(" "),
                    groups[4..].join(" ")
                )),
        )
}
