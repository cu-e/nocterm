//! The chat history: search, pinned chats first, and each chat's actions —
//! rename, fork, pin and delete — beside the Cloud chats of the window's hub in one order by
//! last activity, drawn by the same row renderer with only the actions their port supports.

use gpui_kit::{
    Anchor, App, Context, Entity, SharedString, TestSupportExt as _,
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
use nocterm_ai::agents::{AgentProfile, AvailableAgents};
use nocterm_cloud_agents_model::ChatSummary;
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

/// What one row of the history is: a local chat of the roster, or a Cloud chat the hub lists.
#[derive(Clone)]
enum RowTarget {
    Local(EntityId),
    Cloud(String),
}
impl RowTarget {
    /// The element id of the row's `part`.
    fn element(&self, part: &'static str) -> gpui_kit::ElementId {
        match self {
            Self::Local(id) => (part, id.as_u64()).into(),
            Self::Cloud(key) => SharedString::from(format!("{part}-cloud-{key}")).into(),
        }
    }
}
/// What a history row shows, the same for both kinds of chat.
struct HistoryRow {
    target: RowTarget,
    title: String,
    pinned: bool,
    selected: bool,
    when: String,
    model: String,
    generating: bool,
    icon: RowIcon,
    rename: Option<Input>,
}
enum RowIcon {
    Agent(Option<AgentProfile>),
    Cloud,
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
        let icon = match (&row.icon, row.generating) {
            (_, true) => gpui_kit::component::spinner::Spinner::new()
                .small()
                .color(cx.theme().muted_foreground)
                .into_any_element(),
            (RowIcon::Agent(agent), false) => crate::agent_icon::agent_icon(*agent)
                .small()
                .into_any_element(),
            (RowIcon::Cloud, false) => gpui_kit::component::Icon::new(IconName::Cloud)
                .small()
                .text_color(cx.theme().muted_foreground)
                .into_any_element(),
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
                            .id(row.target.element("history-when"))
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
        let target = row.target.clone();
        let title = row.title.clone();
        let hover = cx.theme().muted.blend(cx.theme().foreground.opacity(0.04));
        let name = Self::name(&row.title, row.rename.take(), cx);
        let details = Self::details(&row, name, cx);
        // A Cloud chat offers only what the hub's port supports: opening it.
        let actions = match &target {
            RowTarget::Local(id) => Some(Self::actions(*id, row.pinned, cx)),
            RowTarget::Cloud(_) => None,
        };
        let open = target.clone();
        h_flex()
            .id(target.element("history-thread"))
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
            .on_click(cx.listener(move |panel, _, window, cx| match &open {
                RowTarget::Local(id) => panel.on_history_selected(*id, cx),
                RowTarget::Cloud(key) => panel.open_cloud_history(key, window, cx),
            }))
            .child(details)
            .children(actions)
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
        chats: &crate::roster::ChatRoster,
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
    /// What the listed rows show, so a chat's change that none of them shows (a draft
    /// keystroke, a streamed chunk) leaves the history as it is. A search matches
    /// transcripts, so while one is typed there is no stamp.
    pub(super) fn stamp(
        chats: &crate::roster::ChatRoster,
        history: &super::history_column::HistoryColumn,
        open_cloud: Option<&str>,
        cx: &App,
    ) -> Option<u64> {
        use std::hash::{Hash as _, Hasher as _};
        if !history.query(cx).is_empty() {
            return None;
        }
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        chats
            .current()
            .map(|current| current.entity_id())
            .hash(&mut hasher);
        (history.cloud().revision(), open_cloud).hash(&mut hasher);
        for thread in chats.iter() {
            let chat = thread.read(cx);
            (
                thread.entity_id(),
                chat.title(),
                chat.document().pinned(),
                chat.document().updated(),
                chat.model(),
                chat.session_generating(),
                chat.document().agent_id(),
                chat.draft_text().is_some(),
                chat.history_entries().is_empty(),
                chat.archive_summary().map(|row| row.has_content),
            )
                .hash(&mut hasher);
        }
        Some(hasher.finish())
    }
    pub(super) fn render(
        sources: HistorySources<'_>,
        agents: &AgentsHandle,
        now: u64,
        cx: &mut Context<AgentPanel>,
    ) -> gpui_kit::AnyElement {
        let HistorySources {
            chats,
            column,
            open_cloud,
        } = sources;
        let mut history = v_flex()
            .id("agent-history-list")
            .test_support()
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .p_2()
            .gap_1();
        if chats.is_empty() && column.cloud().is_empty(cx) {
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
                column
                    .query_input()
                    .prefix(gpui_kit::component::Icon::new(IconName::Search).small())
                    .cleanable(true),
            ),
        );
        let listed = Self::listed_threads(chats, column, cx);
        let cloud = column.cloud().matching(&column.query(cx), cx);
        if listed.is_empty() && cloud.is_empty() {
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
        // Resolved once: resolving builds the whole agent list from the catalog and settings.
        let available = agents.available(cx);
        // Local and Cloud chats share one order: pinned first, then the most recently changed.
        // A Cloud chat's time is its runner's last activity; one without a summary goes last.
        let mut rows: Vec<((bool, u64), HistoryRow)> = Vec::with_capacity(listed.len());
        for thread in listed {
            let document = thread.read(cx).document();
            let key = (document.pinned(), document.updated());
            let row = Self::local_row(&thread, chats, column, (agents, &available), now, cx);
            rows.push((key, row));
        }
        for summary in cloud {
            let key = (false, summary.updated_at.unwrap_or(0));
            rows.push((key, Self::cloud_row(summary, open_cloud.as_deref(), now)));
        }
        rows.sort_by_key(|(key, _)| std::cmp::Reverse(*key));
        for (_, row) in rows {
            history = history.child(HistoryRowRenderer::render(row, cx));
        }
        history.into_any_element()
    }
    fn local_row(
        thread: &Entity<AgentThread>,
        chats: &crate::roster::ChatRoster,
        history: &super::history_column::HistoryColumn,
        (agents, available): (&AgentsHandle, &AvailableAgents),
        now: u64,
        cx: &App,
    ) -> HistoryRow {
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
        HistoryRow {
            target: RowTarget::Local(id),
            title: chat.title().to_string(),
            pinned: chat.document().pinned(),
            selected: chats
                .current()
                .is_some_and(|current| current.entity_id() == id),
            when,
            model: chat.model().unwrap_or_default(),
            generating: chat.session_generating(),
            icon: RowIcon::Agent(agents.profile_in(available, chat.document().agent_id())),
            rename: history.rename_input(id),
        }
    }
    /// A Cloud chat's row: its runner's last activity and what the chat is doing, else who
    /// hosts it. A row the runner did not confirm says "as of".
    fn cloud_row(summary: ChatSummary, open: Option<&str>, now: u64) -> HistoryRow {
        use nocterm_cloud_agents_model::{ChatActivity, ChatListing};
        let mut when = match summary.updated_at {
            Some(updated) => {
                let ago =
                    relative_prompt_time(Some(Duration::from_secs(now.saturating_sub(updated))));
                match summary.listing {
                    ChatListing::AsOf => format!("as of {ago}"),
                    _ => ago,
                }
            }
            None => summary.host.clone(),
        };
        let state = match summary.activity {
            Some(ChatActivity::Running) => Some("running"),
            Some(ChatActivity::WaitingApproval) => Some("waiting for approval"),
            Some(ChatActivity::NeedsGrant) => Some("waiting for permission"),
            Some(ChatActivity::NeedsProvider) => Some("needs provider access"),
            Some(ChatActivity::NeedsSsh) => Some("needs server access"),
            Some(ChatActivity::Failed) => Some("failed"),
            Some(ChatActivity::Idle) | None => None,
        };
        if let Some(state) = state {
            when.push_str(" · ");
            when.push_str(state);
        }
        // The servers the chat is bound to, as its summary lists them (ADR 0147).
        if !summary.servers.is_empty() {
            when.push_str(" · on ");
            when.push_str(&summary.servers.join(", "));
        }
        HistoryRow {
            selected: open == Some(summary.key.as_str()),
            target: RowTarget::Cloud(summary.key),
            title: summary.title,
            pinned: false,
            when,
            model: summary.model.unwrap_or_default(),
            generating: false,
            icon: RowIcon::Cloud,
            rename: None,
        }
    }
}

/// What the history lists: the roster's local chats, and the Cloud chats of the history
/// column with the key of the one that is open.
pub(super) struct HistorySources<'a> {
    chats: &'a crate::roster::ChatRoster,
    column: &'a super::history_column::HistoryColumn,
    open_cloud: Option<String>,
}
impl<'a> HistorySources<'a> {
    pub(super) fn new(
        chats: &'a crate::roster::ChatRoster,
        column: &'a super::history_column::HistoryColumn,
        open_cloud: Option<&str>,
    ) -> Self {
        Self {
            chats,
            column,
            open_cloud: open_cloud.map(str::to_owned),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::order;

    #[test]
    fn a_cloud_row_names_the_servers_its_chat_is_bound_to() {
        let mut summary = nocterm_cloud_agents_model::ChatSummary::plain(
            "k".into(),
            "Deploy".into(),
            "Scribe".into(),
            None,
        );
        summary.updated_at = Some(100);
        summary.servers = vec!["deploy@db".into(), "root@web".into()];
        let row = super::HistoryProjection::cloud_row(summary, None, 160);
        assert!(
            row.when.ends_with(" · on deploy@db, root@web"),
            "{}",
            row.when
        );
    }

    #[test]
    fn pinned_chats_come_first_then_the_most_recent() {
        // (pinned, updated), in the order the chats were made.
        let chats = [(false, 10), (true, 5), (false, 30), (true, 7), (false, 30)];
        assert_eq!(order(&chats), [3, 1, 4, 2, 0]);
    }
}
