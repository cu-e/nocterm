//! The empty center offers a new connection and a compact saved-server history.
use gpui_kit::{
    Action as _, AnyElement, Context, ObjectFit, SharedString, StyledImage as _,
    TestSupportExt as _,
    component::{
        ActiveTheme as _, Icon, Sizable as _,
        button::{Button, ButtonVariants as _},
        h_flex, v_flex,
    },
    div, img,
    prelude::*,
    px, rems,
};
use nocterm_ui::IconName;

use super::Workspace;
use crate::{ConnectionSummary, NewTab};

const RECENT_LIMIT: usize = 6;

impl Workspace {
    pub(super) fn render_empty_state(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let recent = self
            .connection_directory()
            .map(|directory| directory.recent_connections(cx))
            .unwrap_or_default();
        v_flex()
            .size_full()
            .min_h_0()
            .items_center()
            .justify_center()
            .child(
                v_flex()
                    .w_full()
                    .max_w(rems(20.))
                    .max_h_full()
                    .min_h_0()
                    .p_4()
                    .items_center()
                    .gap_3()
                    .text_color(cx.theme().muted_foreground)
                    .child(Icon::new(IconName::SquareTerminal).large())
                    .child("No open sessions")
                    .child(
                        Button::new("empty-new-tab")
                            .primary()
                            .label("New Connection")
                            .on_click(|_, window, cx| {
                                window.dispatch_action(NewTab.boxed_clone(), cx)
                            }),
                    )
                    .when(!recent.is_empty(), |body| {
                        body.child(
                            v_flex()
                                .id("empty-recent-connections")
                                .test_support()
                                .w_full()
                                .min_h_0()
                                .max_h(rems(12.))
                                .overflow_y_scroll()
                                .children(recent.into_iter().take(RECENT_LIMIT).map(
                                    |connection| self.render_recent_connection(connection, cx),
                                )),
                        )
                    }),
            )
    }

    fn render_recent_connection(
        &self,
        connection: ConnectionSummary,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme();
        let directory = self.connection_directory();
        let workspace = cx.entity().downgrade();
        let id = connection.id.clone();
        h_flex()
            .id(SharedString::from(format!("empty-recent-{id}")))
            .test_support()
            .role(gpui_kit::Role::Button)
            .aria_label(connection.name.clone())
            .w_full()
            .gap_2()
            .px_2()
            .py_1()
            .rounded(theme.radius)
            .cursor_pointer()
            .hover(|row| row.bg(theme.sidebar_accent))
            .text_color(theme.foreground)
            .child(match connection.icon {
                Some(icon) => img(icon).size_4().flex_shrink_0().into_any_element(),
                None => Icon::new(IconName::Server)
                    .small()
                    .text_color(theme.muted_foreground)
                    .into_any_element(),
            })
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_sm()
                    .truncate()
                    .child(connection.name),
            )
            .when_some(connection.flag, |row, flag| {
                row.child(
                    div()
                        .id(SharedString::from(format!("empty-recent-flag-{id}")))
                        .test_support()
                        .flex_shrink_0()
                        .rounded_xs()
                        .overflow_hidden()
                        .border_1()
                        .border_color(theme.border)
                        .child(img(flag).w(px(16.)).h(px(11.)).object_fit(ObjectFit::Cover)),
                )
            })
            .on_click(move |_, window, cx| {
                if let Some(directory) = directory.clone() {
                    let workspace = workspace.clone();
                    let id = id.clone();
                    // Opening a session updates Workspace; wait for this render's borrow to end.
                    window.defer(cx, move |window, cx| {
                        directory.open(&id, &workspace, window, cx);
                    });
                }
            })
            .into_any_element()
    }
}

#[cfg(test)]
mod tests;
