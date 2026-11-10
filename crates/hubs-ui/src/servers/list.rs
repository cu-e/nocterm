//! The compact list of known servers on the left of the Servers page.

use gpui_kit::{
    App, ClickEvent, Context, TestSupportExt as _, WeakEntity, Window,
    component::{
        ActiveTheme as _, Icon,
        button::{Button, ButtonVariants as _},
        h_flex, v_flex,
    },
    div,
    prelude::*,
    px,
};
use nocterm_hubs::HubSummary;
use nocterm_hubs_model::HubId;

use super::{ServersPage, item};
use crate::SignInToServer;
use crate::color::tint;
use crate::status::{status_chip, status_icon};

/// The servers, the one in `selected` marked, and the button that signs in to another.
pub(super) fn server_list(
    summaries: &[HubSummary],
    selected: Option<&HubId>,
    cx: &mut Context<ServersPage>,
) -> gpui_kit::AnyElement {
    let page = cx.weak_entity();
    let rows: Vec<_> = summaries
        .iter()
        .map(|summary| entry(summary, selected == Some(&summary.id), page.clone(), cx))
        .collect();
    v_flex()
        .id("servers-list")
        .test_support()
        .w(px(240.))
        .flex_none()
        .gap_2()
        .child(v_flex().gap_1().children(rows))
        .child(
            Button::new("servers-add")
                .primary()
                .label("Sign in to a server…")
                .on_click(|_: &ClickEvent, window: &mut Window, cx: &mut App| {
                    window.dispatch_action(Box::new(SignInToServer), cx);
                }),
        )
        .into_any_element()
}

/// One server: its colour mark, its name and a dot in the colour of its status.
fn entry(
    summary: &HubSummary,
    selected: bool,
    page: WeakEntity<ServersPage>,
    cx: &App,
) -> impl IntoElement {
    let key = summary.address.to_string();
    let theme = cx.theme();
    let (_, status_color) = status_chip(&summary.status, cx);
    let mark = summary
        .color
        .map_or(theme.muted_foreground, |color| tint(color, cx));
    let id = summary.id.clone();
    h_flex()
        .id(item("server-select", &key))
        .test_support()
        .gap_2()
        .px_2()
        .py_1p5()
        .items_center()
        .rounded(theme.radius)
        .cursor_pointer()
        .when(selected, |entry| entry.bg(theme.accent))
        .child(Icon::new(status_icon(&summary.status)).text_color(mark))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .child(summary.label.clone()),
        )
        .child(
            div()
                .id(item("server-dot", &key))
                .test_support()
                .size_2()
                .flex_none()
                .rounded_full()
                .bg(status_color),
        )
        .on_click(move |_, _, cx| {
            let _ = page.update(cx, |page, cx| page.select_hub(id.clone(), cx));
        })
}
