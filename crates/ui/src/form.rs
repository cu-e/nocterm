//! The look shared by settings pages: titled sections of rows on the
//! terminal's background, without cards or outlines.
use gpui_kit::{
    AnyElement, App, Hsla, SharedString,
    component::{ActiveTheme as _, Sizable as _, StyledExt as _, h_flex, v_flex},
    div,
    prelude::*,
    px, rems,
};

use crate::{ActiveDesign as _, TerminalStyle, hsla};

/// The background of settings pages: the terminal's, so they read as part of
/// the same surface.
pub fn page_background(cx: &App) -> Hsla {
    if cx.has_global::<crate::SettingsStore>() {
        hsla(TerminalStyle::current(cx).background)
    } else {
        cx.theme().background
    }
}

/// The width a settings form keeps to on wide windows.
pub fn page_width(cx: &App) -> gpui_kit::Rems {
    rems(cx.design().layout.settings_width)
}

/// An entry of a page list beside a form: icon and title, highlighted when
/// `selected`. The caller adds the click handler.
pub fn nav_item(
    id: impl Into<gpui_kit::ElementId>,
    icon: crate::IconName,
    title: impl Into<SharedString>,
    selected: bool,
    cx: &App,
) -> gpui_kit::Stateful<gpui_kit::Div> {
    let theme = cx.theme();
    h_flex()
        .id(id)
        .w_full()
        .gap_2()
        .px_3()
        .py_1p5()
        .rounded(theme.radius)
        .text_sm()
        .cursor_pointer()
        .text_color(if selected {
            theme.foreground
        } else {
            theme.muted_foreground
        })
        .when(selected, |item| item.bg(theme.foreground.opacity(0.08)))
        .hover(|item| item.bg(theme.foreground.opacity(0.05)))
        .child(gpui_kit::component::Icon::new(icon).small())
        .child(title.into())
}

/// A page heading with an optional line under it.
pub fn page_header(
    title: impl Into<SharedString>,
    description: Option<SharedString>,
    cx: &App,
) -> AnyElement {
    v_flex()
        .gap_1()
        .pb_2()
        .child(div().text_xl().font_semibold().child(title.into()))
        .when_some(description, |header, description| {
            header.child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(description),
            )
        })
        .into_any_element()
}

/// A group of rows under a heading; rows are separated by hairlines.
pub fn section(
    title: impl Into<SharedString>,
    rows: impl IntoIterator<Item = AnyElement>,
    cx: &App,
) -> AnyElement {
    let divider = cx.theme().border.opacity(0.5);
    let mut list = v_flex().w_full();
    for (index, row) in rows.into_iter().enumerate() {
        list = list.child(
            div()
                .w_full()
                .when(index > 0, |row| row.border_t_1().border_color(divider))
                .child(row),
        );
    }
    v_flex()
        .w_full()
        .pt_4()
        .child(
            div()
                .pb_1()
                .text_xs()
                .font_semibold()
                .text_color(cx.theme().muted_foreground)
                .child(title.into().to_uppercase()),
        )
        .child(list)
        .into_any_element()
}

/// Label and description on the left, the control on the right.
pub fn row(
    label: impl Into<SharedString>,
    description: impl Into<SharedString>,
    control: impl IntoElement,
    cx: &App,
) -> AnyElement {
    h_flex()
        .w_full()
        .py_3()
        .gap_4()
        .justify_between()
        .items_center()
        .child(text(label.into(), description.into(), cx))
        .child(div().flex_shrink_0().child(control))
        .into_any_element()
}

/// Label and description above a full-width control, with its error under it.
pub fn stacked_row(
    label: impl Into<SharedString>,
    description: impl Into<SharedString>,
    control: impl IntoElement,
    error: Option<SharedString>,
    cx: &App,
) -> AnyElement {
    v_flex()
        .w_full()
        .py_3()
        .gap_2()
        .child(text(label.into(), description.into(), cx))
        .child(control)
        .when_some(error, |row, error| row.child(error_text(error, cx)))
        .into_any_element()
}

/// A validation or operation error under a control.
pub fn error_text(error: impl Into<SharedString>, cx: &App) -> AnyElement {
    div()
        .text_xs()
        .text_color(cx.theme().danger)
        .child(error.into())
        .into_any_element()
}

/// A muted note, for status lines and empty states.
pub fn note(text: impl Into<SharedString>, cx: &App) -> AnyElement {
    div()
        .py_2()
        .text_sm()
        .text_color(cx.theme().muted_foreground)
        .child(text.into())
        .into_any_element()
}

fn text(label: SharedString, description: SharedString, cx: &App) -> impl IntoElement {
    v_flex()
        .flex_1()
        .min_w_0()
        .gap(px(2.))
        .child(div().text_sm().font_medium().child(label))
        .when(!description.is_empty(), |text| {
            text.child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(description),
            )
        })
}
