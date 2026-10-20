//! The chat history: search, pinned chats first, and each chat's actions —
//! rename, fork, pin and delete.

use gpui_kit::{
    Anchor, App, Context, Entity, TestSupportExt as _,
    component::{
        ActiveTheme as _, Sizable as _,
        button::{Button, ButtonVariants as _},
        h_flex,
        input::Input,
        v_flex,
    },
    div,
    prelude::*,
    px,
};
use nocterm_agent_directory::AgentsHandle;
use nocterm_ai::agents::AgentProfile;
use nocterm_ui::IconName;
use std::time::Duration;

use super::{
    AgentPanel,
    widgets::{relative_prompt_time, single_line_label},
};
use crate::thread::AgentThread;
use gpui_kit::{
    EntityId, MouseButton,
    component::menu::{DropdownMenu as _, PopupMenuItem},
};

/// The order chats are listed in: pinned first, then the most recently
/// changed. `chats` holds each chat's pin and change time.
pub(super) fn order(chats: &[(bool, u64)]) -> Vec<usize> {
    let mut order: Vec<usize> = (0..chats.len()).collect();
    // Stable, so chats changed in the same second keep the order they were
    // made in, newest first.
    order.reverse();
    order.sort_by_key(|&index| std::cmp::Reverse(chats[index]));
    order
}

struct HistoryRow {
    id: EntityId,
    title: String,
    pinned: bool,
    selected: bool,
    when: String,
    model: String,
    generating: bool,
    agent: Option<AgentProfile>,
    rename: Option<Input>,
}
struct HistoryRowRenderer;
impl HistoryRowRenderer {
    fn name(
        title: &str,
        input: Option<Input>,
        cx: &mut Context<AgentPanel>,
    ) -> gpui_kit::AnyElement {
        match input {
            Some(input) => div()
                .flex_1()
                .min_w_0()
                .key_context("AgentChatRename")
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_key_down(
                    cx.listener(|panel, event: &gpui_kit::KeyDownEvent, window, cx| {
                        panel.on_history_rename_key(event, window, cx)
                    }),
                )
                .child(input)
                .into_any_element(),
            None => div()
                .flex_1()
                .min_w_0()
                .truncate()
                .child(single_line_label(title))
                .into_any_element(),
        }
    }
    fn details(row: &HistoryRow, name: gpui_kit::AnyElement, cx: &gpui_kit::App) -> gpui_kit::Div {
        let icon = if row.generating {
            gpui_kit::component::spinner::Spinner::new()
                .small()
                .color(cx.theme().muted_foreground)
                .into_any_element()
        } else {
            crate::agent_icon::agent_icon(row.agent)
                .small()
                .into_any_element()
        };
        v_flex()
            .flex_1()
            .min_w_0()
            .gap_0p5()
            .child(
                h_flex()
                    .w_full()
                    .min_w_0()
                    .gap_1()
                    .child(icon)
                    .when(row.pinned, |line| {
                        line.child(
                            gpui_kit::component::Icon::new(IconName::Pin)
                                .xsmall()
                                .flex_shrink_0()
                                .text_color(cx.theme().muted_foreground),
                        )
                    })
                    .child(name),
            )
            .child(
                h_flex()
                    .w_full()
                    .min_w_0()
                    .gap_2()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(
                        div()
                            .id(("history-when", row.id.as_u64()))
                            .test_support()
                            .aria_label(row.when.clone())
                            .flex_shrink_0()
                            .child(row.when.clone()),
                    )
                    .child(div().flex_1())
                    .child(div().min_w_0().truncate().child(row.model.clone())),
            )
    }
    pub(super) fn render(
        mut row: HistoryRow,
        cx: &mut Context<AgentPanel>,
    ) -> gpui_kit::AnyElement {
        let id = row.id;
        let title = row.title.clone();
        let hover = cx.theme().muted.blend(cx.theme().foreground.opacity(0.04));
        let name = Self::name(&row.title, row.rename.take(), cx);
        let details = Self::details(&row, name, cx);
        let actions = Self::actions(row.id, row.pinned, cx);
        h_flex()
            .id(("history-thread", id.as_u64()))
            .test_support()
            .role(gpui_kit::Role::Button)
            .aria_label(title.clone())
            .w_full()
            .min_w_0()
            .gap_1()
            .pl_2()
            .pr_1()
            .py_1p5()
            .rounded(px(8.))
            .cursor_pointer()
            .text_sm()
            .when(row.selected, |row| row.bg(cx.theme().muted))
            .when(!row.selected, |row| row.hover(move |style| style.bg(hover)))
            .tooltip(move |_, cx| {
                cx.new(|_| gpui_kit::component::tooltip::Tooltip::new(title.clone()))
                    .into()
            })
            .on_click(cx.listener(move |panel, _, _, cx| panel.on_history_selected(id, cx)))
            .child(details)
            .child(actions)
            .into_any_element()
    }
    fn actions(id: EntityId, pinned: bool, cx: &mut Context<AgentPanel>) -> gpui_kit::AnyElement {
        let panel = cx.weak_entity();
        h_flex()
            .flex_shrink_0()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                Button::new(("history-actions", id.as_u64()))
                    .ghost()
                    .xsmall()
                    .icon(IconName::Ellipsis)
                    .tooltip("Chat actions")
                    .dropdown_menu_with_anchor(Anchor::TopRight, move |menu, _, _| {
                        Self::menu(menu, id, pinned, panel.clone())
                    }),
            )
            .child(
                Button::new(("delete-thread", id.as_u64()))
                    .ghost()
                    .xsmall()
                    .icon(IconName::Trash)
                    .tooltip("Delete chat")
                    .on_click(
                        cx.listener(move |panel, _, window, cx| {
                            panel.delete_thread(id, window, cx)
                        }),
                    ),
            )
            .into_any_element()
    }
    fn menu(
        menu: gpui_kit::component::menu::PopupMenu,
        id: EntityId,
        pinned: bool,
        panel: gpui_kit::WeakEntity<AgentPanel>,
    ) -> gpui_kit::component::menu::PopupMenu {
        let rename = panel.clone();
        let fork = panel.clone();
        let pin = panel.clone();
        let release = panel;
        menu.item(
            PopupMenuItem::new("Rename")
                .icon(IconName::Pencil)
                .on_click(move |_, window, cx| {
                    let _ = rename.update(cx, |panel, cx| panel.start_rename(id, window, cx));
                }),
        )
        .item(
            PopupMenuItem::new("Fork")
                .icon(IconName::GitFork)
                .on_click(move |_, window, cx| {
                    let _ = fork.update(cx, |panel, cx| panel.fork_thread(id, window, cx));
                }),
        )
        .item(
            PopupMenuItem::new("Release agent resources").on_click(move |_, _, cx| {
                let _ = release.update(cx, |panel, cx| panel.release_history_resources(id, cx));
            }),
        )
        .item(
            PopupMenuItem::new(if pinned { "Unpin" } else { "Pin" })
                .icon(if pinned {
                    IconName::PinOff
                } else {
                    IconName::Pin
                })
                .on_click(move |_, _, cx| {
                    let _ = pin.update(cx, |panel, cx| panel.set_pinned(id, !pinned, cx));
                }),
        )
    }
}

pub(super) struct HistoryProjection;
impl HistoryProjection {
    /// The chats to list, in order, matching the search.
    fn listed_threads(
        chats: &super::chat_list::ChatList,
        history: &super::history_column::HistoryColumn,
        cx: &App,
    ) -> Vec<Entity<AgentThread>> {
        let query = history.query(cx);
        let chats_owner = chats;
        let chats: Vec<(bool, u64)> = chats
            .iter()
            .map(|thread| {
                (
                    thread.read(cx).document().pinned(),
                    thread.read(cx).document().updated(),
                )
            })
            .collect();
        let threads = chats_owner.iter().collect::<Vec<_>>();
        order(&chats)
            .into_iter()
            .map(|index| threads[index])
            .filter(|thread| {
                let thread = thread.read(cx);
                query.is_empty()
                    || thread.title().to_lowercase().contains(&query)
                    || thread.transcript().mentions(&query)
                    || history.matches(thread.document().chat_id())
            })
            .cloned()
            .collect()
    }
    pub(super) fn render(
        chats: &super::chat_list::ChatList,
        history_owner: &super::history_column::HistoryColumn,
        agents: &AgentsHandle,
        now: u64,
        cx: &mut Context<AgentPanel>,
    ) -> gpui_kit::AnyElement {
        let mut history = v_flex()
            .id("agent-history-list")
            .test_support()
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .p_2()
            .gap_1();
        if chats.is_empty() {
            return history
                .child(
                    div()
                        .id("agent-history-empty")
                        .test_support()
                        .px_1()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child("History is empty"),
                )
                .into_any_element();
        }
        history = history.child(
            div().pb_1().child(
                history_owner
                    .query_input()
                    .prefix(gpui_kit::component::Icon::new(IconName::Search).small())
                    .cleanable(true),
            ),
        );
        let listed = Self::listed_threads(chats, history_owner, cx);
        if listed.is_empty() {
            history = history.child(
                div()
                    .id("agent-history-no-match")
                    .test_support()
                    .px_1()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child("No chats match"),
            );
        }
        for thread in listed {
            history = history.child(Self::render_history_row(
                &thread,
                chats,
                history_owner,
                agents,
                now,
                cx,
            ));
        }
        history.into_any_element()
    }
    fn render_history_row(
        thread: &Entity<AgentThread>,
        chats: &super::chat_list::ChatList,
        history: &super::history_column::HistoryColumn,
        agents: &AgentsHandle,
        now: u64,
        cx: &mut Context<AgentPanel>,
    ) -> gpui_kit::AnyElement {
        let id = thread.entity_id();
        let chat = thread.read(cx);
        let when = if chat.archive_summary().is_none_or(|row| !row.has_content)
            && chat.history_entries().is_empty()
            && chat.draft_text().is_none()
        {
            "No requests yet".to_owned()
        } else {
            relative_prompt_time(Some(Duration::from_secs(
                now.saturating_sub(chat.updated_at()),
            )))
        };
        let row = HistoryRow {
            id,
            title: chat.title().to_string(),
            pinned: chat.document().pinned(),
            selected: chats
                .current()
                .is_some_and(|current| current.entity_id() == id),
            when,
            model: chat.model().unwrap_or_default(),
            generating: chat.session_generating(),
            agent: agents.profile(chat.document().agent_id(), cx),
            rename: history.rename_input(id),
        };
        HistoryRowRenderer::render(row, cx)
    }
}

#[cfg(test)]
mod tests {
    use super::order;

    #[test]
    fn pinned_chats_come_first_then_the_most_recent() {
        // (pinned, updated), in the order the chats were made.
        let chats = [(false, 10), (true, 5), (false, 30), (true, 7), (false, 30)];
        assert_eq!(order(&chats), [3, 1, 4, 2, 0]);
    }
}
