//! The chat history: search, pinned chats first, and each chat's actions —
//! rename, fork, pin and delete.

use gpui_kit::{
    Anchor, App, Context, Entity, TestSupportExt as _, Window,
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
use nocterm_ui::IconName;
use std::time::Duration;

use super::{
    AgentPanel,
    widgets::{relative_prompt_time, single_line_label},
};
use crate::{runtime::Runtime, thread::AgentThread};
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

impl AgentPanel {
    fn index_of(&self, id: EntityId) -> Option<usize> {
        self.threads
            .iter()
            .position(|thread| thread.entity_id() == id)
    }

    /// Keeps the chats `keep` accepts, and the open chat where it is kept.
    pub(super) fn retain_threads(
        &mut self,
        keep: impl Fn(&Entity<AgentThread>, &App) -> bool,
        cx: &mut Context<Self>,
    ) {
        let active = self.current().map(|thread| thread.entity_id());
        let proposed: Vec<_> = self
            .threads
            .iter()
            .filter(|thread| !keep(thread, cx))
            .map(|thread| thread.entity_id())
            .collect();
        let removed = proposed
            .into_iter()
            .filter(|id| self.admit_before_removal(*id, cx))
            .collect::<Vec<_>>();
        if self
            .composer
            .editing_chat()
            .is_some_and(|chat| removed.contains(&chat))
        {
            self.leave_composer_with(true, cx);
        }
        self.composer.removed_chats(&removed);
        self.threads
            .retain(|thread| !removed.contains(&thread.entity_id()));
        self.active = active.and_then(|id| self.index_of(id));
        if active.is_some_and(|id| removed.contains(&id)) {
            self.reset_chat_view(cx);
        }
        if !removed.is_empty() {
            self.on_history_catalog_changed(cx);
        }
    }

    /// Drops empty, unnamed chats other than `keep`: a chat nobody wrote in
    /// is not history.
    pub(super) fn discard_drafts(&mut self, keep: Option<EntityId>, cx: &mut Context<Self>) {
        self.retain_threads(
            |thread, cx| Some(thread.entity_id()) == keep || !thread.read(cx).is_draft(),
            cx,
        );
    }

    /// Shows chat `id`, connecting it if it came from history.
    pub(super) fn open_thread(&mut self, id: EntityId, cx: &mut Context<Self>) {
        self.leave_composer(cx);
        self.discard_drafts(Some(id), cx);
        if self.index_of(id).is_none() {
            return;
        }
        self.select_thread(id, cx);
    }

    pub(super) fn delete_thread(
        &mut self,
        id: EntityId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(index) = self.index_of(id) {
            let thread = self.threads[index].clone();
            let chat = thread.read(cx).document().chat_id().to_owned();
            thread.update(cx, |thread, cx| thread.release_resources(cx));
            Runtime::global(cx).update(cx, |runtime, cx| runtime.delete_chat(chat, cx));
            self.retain_threads(|thread, _| thread.entity_id() != id, cx);
            if self.current().is_none() {
                self.composer.set_text("", window, cx);
            }
        }
        self.sync_approval_attention(window, cx);
        cx.notify();
    }

    /// Copies chat `id` into a new chat and opens the copy.
    pub(super) fn fork_thread(
        &mut self,
        id: EntityId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.fork_thread_at(id, None, window, cx);
    }

    /// Copies chat `id` up to and including entry `last` (all of it, with
    /// `None`) into a new chat and opens the copy.
    pub(super) fn fork_thread_at(
        &mut self,
        id: EntityId,
        last: Option<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(source) = self.index_of(id).map(|index| self.threads[index].clone()) else {
            return;
        };
        if source.read(cx).is_archived() {
            let panel = cx.weak_entity();
            let handle = window.window_handle();
            source.update(cx, |thread, cx| {
                thread.load_archive(
                    crate::thread::archive::ArchiveIntent::Fork {
                        panel,
                        source: id,
                        window: handle,
                        last,
                    },
                    cx,
                )
            });
            return;
        }
        let fork = match last {
            Some(last) => source.read(cx).fork_at(last + 1),
            None => source.read(cx).fork(),
        };
        let host = self.host.clone();
        let thread = cx.new(|cx| AgentThread::forked(fork, host, cx));
        thread.update(cx, |thread, cx| thread.persist(cx));
        self.track(&thread, window, cx);
        let id = thread.entity_id();
        self.threads.push(thread);
        self.on_history_catalog_changed(cx);
        self.open_thread(id, cx);
    }

    pub(crate) fn fork_loaded_archive(
        &mut self,
        source: EntityId,
        last: Option<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.index_of(source).is_some() {
            self.fork_thread_at(source, last, window, cx);
        }
    }

    pub(super) fn set_pinned(&mut self, id: EntityId, pinned: bool, cx: &mut Context<Self>) {
        if let Some(index) = self.index_of(id) {
            self.threads[index].update(cx, |thread, cx| thread.set_pinned(pinned, cx));
        }
    }

    pub(super) fn start_rename(
        &mut self,
        id: EntityId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self.index_of(id) else {
            return;
        };
        let title = self.threads[index].read(cx).title();
        self.history.begin_rename(id, title.to_string(), window, cx);
        cx.notify();
    }
    pub(super) fn finish_rename(
        &mut self,
        accept: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if accept {
            if let Some(binding) = self.history.rename_binding() {
                self.commit_history_rename(binding, window, cx);
            }
        } else {
            self.history.cancel_rename();
            window.focus(&self.focus, cx);
            cx.notify();
        }
    }
    pub(super) fn commit_history_rename(
        &mut self,
        binding: super::history_column::RenameBinding,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(request) = self.history.rename_submission(binding, cx) else {
            return;
        };
        let (chat, title) = request.into_parts();
        if let Some(index) = self.index_of(chat) {
            self.threads[index].update(cx, |thread, cx| thread.rename(Some(title), cx));
        }
        window.focus(&self.focus, cx);
        cx.notify();
    }

    /// The chats to list, in order, matching the search.
    fn listed_threads(&self, cx: &App) -> Vec<Entity<AgentThread>> {
        let query = self.history.query(cx);
        let chats: Vec<(bool, u64)> = self
            .threads
            .iter()
            .map(|thread| {
                (
                    thread.read(cx).document().pinned(),
                    thread.read(cx).document().updated(),
                )
            })
            .collect();
        order(&chats)
            .into_iter()
            .map(|index| &self.threads[index])
            .filter(|thread| {
                let thread = thread.read(cx);
                query.is_empty()
                    || thread.title().to_lowercase().contains(&query)
                    || thread.transcript().mentions(&query)
                    || self.history.matches(thread.document().chat_id())
            })
            .cloned()
            .collect()
    }

    pub(super) fn render_history(&mut self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let mut history = v_flex()
            .id("agent-history-list")
            .test_support()
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .p_2()
            .gap_1();
        if self.threads.is_empty() {
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
                self.history
                    .query_input()
                    .prefix(gpui_kit::component::Icon::new(IconName::Search).small())
                    .cleanable(true),
            ),
        );
        let listed = self.listed_threads(cx);
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
            history = history.child(self.render_history_row(&thread, cx));
        }
        history.into_any_element()
    }

    fn render_history_row(
        &self,
        thread: &Entity<AgentThread>,
        cx: &mut Context<Self>,
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
                self.host.timestamp().saturating_sub(chat.updated_at()),
            )))
        };
        let row = HistoryRow {
            id,
            title: chat.title().to_string(),
            pinned: chat.document().pinned(),
            selected: self
                .current()
                .is_some_and(|current| current.entity_id() == id),
            when,
            model: chat.model().unwrap_or_default(),
            generating: chat.session_generating(),
            agent: chat.document().agent_id().to_owned(),
            rename: self.history.rename_input(id),
        };
        HistoryRowRenderer::render(row, cx)
    }
    fn on_history_selected(&mut self, id: EntityId, cx: &mut Context<Self>) {
        if !self.history.renaming() {
            self.open_thread(id, cx);
        }
    }
    fn on_history_rename_key(
        &mut self,
        event: &gpui_kit::KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.keystroke.key == "escape" {
            cx.stop_propagation();
            self.finish_rename(false, window, cx);
        }
    }
    fn release_history_resources(&mut self, id: EntityId, cx: &mut Context<Self>) {
        if let Some(index) = self.index_of(id) {
            self.threads[index].update(cx, |thread, cx| thread.release_resources(cx));
        }
    }
}

struct HistoryRow {
    id: EntityId,
    title: String,
    pinned: bool,
    selected: bool,
    when: String,
    model: String,
    generating: bool,
    agent: String,
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
            nocterm_ui::agent_icon(&row.agent)
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
    fn render(mut row: HistoryRow, cx: &mut Context<AgentPanel>) -> gpui_kit::AnyElement {
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
