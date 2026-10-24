//! Small pieces the parts of the editor share.

use gpui_kit::{
    AnyElement, App, ElementId, SharedString, TestSupportExt as _, Window,
    component::{ActiveTheme as _, Disableable as _, Sizable as _, button::Button},
    div,
    prelude::*,
};

/// A small outlined button. A disabled one ignores clicks.
pub(crate) fn action(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    enabled: bool,
    on_click: impl Fn(&mut Window, &mut App) + 'static,
) -> Button {
    Button::new(id)
        .small()
        .outline()
        .label(label)
        .disabled(!enabled)
        .on_click(move |_, window, cx| on_click(window, cx))
}

/// A small label in a rounded outline, for states such as "Disabled".
pub(crate) fn tag(id: impl Into<ElementId>, text: impl Into<SharedString>, cx: &App) -> AnyElement {
    let text = text.into();
    div()
        .id(id)
        .aria_label(text.clone())
        .test_support()
        .px_1p5()
        .rounded(cx.theme().radius)
        .border_1()
        .border_color(cx.theme().border)
        .text_xs()
        .text_color(cx.theme().muted_foreground)
        .child(text)
        .into_any_element()
}
