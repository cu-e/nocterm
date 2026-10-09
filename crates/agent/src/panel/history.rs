//! The chat history: search, pinned chats first, and each chat's actions —
//! rename, fork, pin and delete.

use gpui_kit::{
    Anchor, App, Context, Entity, Focusable as _, Subscription, TestSupportExt as _, Window,
    component::{
        ActiveTheme as _, Sizable as _,
        button::{Button, ButtonVariants as _},
        h_flex,
        input::{Input, InputEvent, InputState},
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

/// A chat being renamed in place.
pub(super) struct Renaming {
    thread: EntityId,
    pub(super) input: Entity<InputState>,
    _subscriptions: [Subscription; 2],
}

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
        let removed: Vec<_> = self
            .threads
            .iter()
            .filter(|thread| !keep(thread, cx))
            .map(|thread| thread.entity_id())
            .collect();
        if self
            .composer
            .edit
            .as_ref()
            .is_some_and(|edit| removed.contains(&edit.thread))
        {
            self.leave_composer_with(true, cx);
        }
        self.queue_heights.retain(|id, _| !removed.contains(id));
        self.threads
            .retain(|thread| !removed.contains(&thread.entity_id()));
        self.active = active.and_then(|id| self.index_of(id));
        if active.is_some_and(|id| removed.contains(&id)) {
            self.reset_chat_view(cx);
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
            let chat = thread.read(cx).chat_id.clone();
            thread.update(cx, |thread, cx| thread.release_resources(cx));
            Runtime::global(cx).update(cx, |runtime, cx| runtime.delete_chat(chat, cx));
            self.retain_threads(|thread, _| thread.entity_id() != id, cx);
            if self.current().is_none() {
                self.input
                    .update(cx, |input, cx| input.set_value("", window, cx));
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
        if source.read(cx).archive.is_some() {
            let panel = cx.weak_entity();
            let handle = window.window_handle();
            source.update(cx, |thread, cx| {
                thread.load_archive(cx, move |_, cx| {
                    cx.defer(move |cx| {
                        let _ = handle.update(cx, |_, window, cx| {
                            let _ = panel
                                .update(cx, |panel, cx| panel.fork_thread_at(id, last, window, cx));
                        });
                    });
                })
            });
            return;
        }
        let fork = match last {
            Some(last) => source.read(cx).fork_at(last + 1),
            None => source.read(cx).fork(),
        };
        let workspace = self.workspace.clone();
        let thread = cx.new(|cx| AgentThread::forked(fork, workspace, cx));
        thread.update(cx, |thread, cx| thread.persist(cx));
        self.track(&thread, window, cx);
        let id = thread.entity_id();
        self.threads.push(thread);
        self.open_thread(id, cx);
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
        let input = cx.new(|cx| InputState::new(window, cx).default_value(title));
        let enter = cx.subscribe_in(&input, window, |this, _, event, window, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.finish_rename(true, window, cx);
            }
        });
        let focus = input.read(cx).focus_handle(cx);
        let blur = cx.on_blur(&focus, window, |this, window, cx| {
            this.finish_rename(true, window, cx)
        });
        self.renaming = Some(Renaming {
            thread: id,
            input,
            _subscriptions: [enter, blur],
        });
        window.focus(&focus, cx);
        cx.notify();
    }

    pub(super) fn finish_rename(
        &mut self,
        accept: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(renaming) = self.renaming.take() else {
            return;
        };
        if accept && let Some(index) = self.index_of(renaming.thread) {
            let name = renaming.input.read(cx).value().to_string();
            self.threads[index].update(cx, |thread, cx| thread.rename(Some(name), cx));
        }
        window.focus(&self.focus, cx);
        cx.notify();
    }

    /// The chats to list, in order, matching the search.
    fn listed_threads(&self, cx: &App) -> Vec<Entity<AgentThread>> {
        let query = self.history_search.read(cx).value().trim().to_lowercase();
        let chats: Vec<(bool, u64)> = self
            .threads
            .iter()
            .map(|thread| (thread.read(cx).pinned, thread.read(cx).updated))
            .collect();
        order(&chats)
            .into_iter()
            .map(|index| &self.threads[index])
            .filter(|thread| {
                let thread = thread.read(cx);
                query.is_empty()
                    || thread.title().to_lowercase().contains(&query)
                    || thread.state.mentions(&query)
                    || self.archive_matches.contains(&thread.chat_id)
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
                Input::new(&self.history_search)
                    .small()
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

    #[expect(clippy::too_many_lines, reason = "predates the limit")]
    fn render_history_row(
        &self,
        thread: &Entity<AgentThread>,
        cx: &mut Context<Self>,
    ) -> gpui_kit::AnyElement {
        let id = thread.entity_id();
        let key = id.as_u64();
        let chat = thread.read(cx);
        let title = chat.title();
        let pinned = chat.pinned;
        let selected = self
            .current()
            .is_some_and(|current| current.entity_id() == id);
        let when = if chat.archive.as_ref().is_none_or(|row| !row.has_content)
            && chat.state.entries.is_empty()
            && chat.composer.draft.is_none()
        {
            "No requests yet".to_owned()
        } else {
            relative_prompt_time(Some(Duration::from_secs(
                nocterm_ai::time::now().saturating_sub(chat.updated),
            )))
        };
        let model = chat.model().unwrap_or_default();
        let hover = cx.theme().muted.blend(cx.theme().foreground.opacity(0.04));
        let renaming = self
            .renaming
            .as_ref()
            .filter(|renaming| renaming.thread == id)
            .map(|renaming| renaming.input.clone());
        let name: gpui_kit::AnyElement = match renaming {
            Some(input) => div()
                .flex_1()
                .min_w_0()
                .key_context("AgentChatRename")
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_key_down(
                    cx.listener(|this, event: &gpui_kit::KeyDownEvent, window, cx| {
                        if event.keystroke.key == "escape" {
                            cx.stop_propagation();
                            this.finish_rename(false, window, cx);
                        }
                    }),
                )
                .child(Input::new(&input).xsmall())
                .into_any_element(),
            None => div()
                .flex_1()
                .min_w_0()
                .truncate()
                .child(single_line_label(&title))
                .into_any_element(),
        };
        let panel = cx.weak_entity();
        h_flex()
            .id(("history-thread", key))
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
            .when(selected, |row| row.bg(cx.theme().muted))
            .when(!selected, |row| row.hover(move |style| style.bg(hover)))
            .tooltip(move |_, cx| {
                cx.new(|_| gpui_kit::component::tooltip::Tooltip::new(title.clone()))
                    .into()
            })
            .on_click(cx.listener(move |this, _, _, cx| {
                if this.renaming.is_none() {
                    this.open_thread(id, cx);
                }
            }))
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_0p5()
                    .child(
                        h_flex()
                            .w_full()
                            .min_w_0()
                            .gap_1()
                            .child(if chat.generating {
                                gpui_kit::component::spinner::Spinner::new()
                                    .small()
                                    .color(cx.theme().muted_foreground)
                                    .into_any_element()
                            } else {
                                nocterm_ui::agent_icon(&chat.agent_id)
                                    .small()
                                    .into_any_element()
                            })
                            .when(pinned, |row| {
                                row.child(
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
                            .child(div().flex_shrink_0().child(when))
                            .child(div().flex_1())
                            .child(div().min_w_0().truncate().child(model)),
                    ),
            )
            .child(
                // The row's own click must not fire for its buttons.
                h_flex()
                    .flex_shrink_0()
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(
                        Button::new(("history-actions", key))
                            .ghost()
                            .xsmall()
                            .icon(IconName::Ellipsis)
                            .tooltip("Chat actions")
                            .dropdown_menu_with_anchor(Anchor::TopRight, move |menu, _, _| {
                                let rename = panel.clone();
                                let fork = panel.clone();
                                let pin = panel.clone();
                                let release = panel.clone();
                                menu.item(
                                    PopupMenuItem::new("Rename")
                                        .icon(IconName::Pencil)
                                        .on_click(move |_, window, cx| {
                                            let _ = rename.update(cx, |panel, cx| {
                                                panel.start_rename(id, window, cx)
                                            });
                                        }),
                                )
                                .item(PopupMenuItem::new("Fork").icon(IconName::GitFork).on_click(
                                    move |_, window, cx| {
                                        let _ = fork.update(cx, |panel, cx| {
                                            panel.fork_thread(id, window, cx)
                                        });
                                    },
                                ))
                                .item(PopupMenuItem::new("Release agent resources").on_click(
                                    move |_, _, cx| {
                                        let _ = release.update(cx, |panel, cx| {
                                            if let Some(index) = panel.index_of(id) {
                                                panel.threads[index].update(cx, |thread, cx| {
                                                    thread.release_resources(cx)
                                                });
                                            }
                                        });
                                    },
                                ))
                                .item(
                                    PopupMenuItem::new(if pinned { "Unpin" } else { "Pin" })
                                        .icon(if pinned {
                                            IconName::PinOff
                                        } else {
                                            IconName::Pin
                                        })
                                        .on_click(move |_, _, cx| {
                                            let _ = pin.update(cx, |panel, cx| {
                                                panel.set_pinned(id, !pinned, cx)
                                            });
                                        }),
                                )
                            }),
                    )
                    .child(
                        Button::new(("delete-thread", key))
                            .ghost()
                            .xsmall()
                            .icon(IconName::Trash)
                            .tooltip("Delete chat")
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.delete_thread(id, window, cx)
                            })),
                    ),
            )
            .into_any_element()
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
