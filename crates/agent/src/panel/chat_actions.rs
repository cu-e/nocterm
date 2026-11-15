//! Actions bind to the chat shown when its title menu opens, not a later selection.
use super::AgentPanel;
use crate::thread::AgentThread;
use gpui_kit::{
    Anchor, AnyElement, App, ClipboardItem, Context, Entity, IntoElement, WeakEntity,
    component::{
        Sizable as _,
        button::{Button, ButtonVariants as _},
        menu::{DropdownMenu as _, PopupMenuItem},
    },
    div,
    prelude::*,
};
use nocterm_ui::IconName;

pub(super) struct ChatActions;
impl ChatActions {
    pub(super) fn button(thread: Entity<AgentThread>, cx: &mut Context<AgentPanel>) -> AnyElement {
        let panel = cx.weak_entity();
        div()
            .id(("chat-actions-origin", thread.entity_id().as_u64()))
            .flex_shrink_0()
            .child(
                Button::new("agent-chat-actions")
                    .ghost()
                    .small()
                    .icon(IconName::Ellipsis)
                    .tooltip("Chat actions")
                    .accessibility_label("Chat actions")
                    .dropdown_menu_with_anchor(Anchor::TopLeft, move |menu, _, cx| {
                        let id = thread.read(cx).chat_id().to_owned();
                        let weak = panel.clone();
                        let thread = thread.clone();
                        let disabled = thread.read(cx).is_archived()
                            || panel
                                .upgrade()
                                .is_none_or(|panel| panel.read(cx).menus.copying());
                        menu.item(
                            PopupMenuItem::new("Copy history ID")
                                .icon(IconName::Copy)
                                .on_click(move |_, _, cx| {
                                    cx.write_to_clipboard(ClipboardItem::new_string(id.clone()));
                                }),
                        )
                        .item(
                            PopupMenuItem::new("Copy conversation")
                                .icon(IconName::Copy)
                                .disabled(disabled)
                                .on_click(move |_, _, cx| {
                                    Self::copy(weak.clone(), thread.clone(), cx);
                                }),
                        )
                    }),
            )
            .into_any_element()
    }

    fn copy(panel: WeakEntity<AgentPanel>, thread: Entity<AgentThread>, cx: &mut App) {
        let _ = panel.update(cx, |panel, cx| {
            if panel.menus.copying() || thread.read(cx).is_archived() {
                return;
            }
            let snapshot = match thread.update(cx, |thread, cx| thread.shared_snapshot(cx)) {
                Ok(snapshot) => snapshot,
                Err(error) => {
                    tracing::warn!(error = %nocterm_ai::redact::redact(&format!("{error:?}")), "could not copy conversation");
                    use nocterm_agent_runtime::PersistenceFailure;
                    let message = match PersistenceFailure::of_storage(&error) {
                        PersistenceFailure::HistoryMemory | PersistenceFailure::HistoryBusy =>
                            "History is busy. Close another conversation and try copying again.",
                        _ => "Could not copy this conversation. Please try again.",
                    };
                    panel.notices.set_error(Some(message.into()));
                    cx.notify();
                    return;
                }
            };
            let title = thread.read(cx).title();
            let task = cx.spawn(async move |panel, cx| {
                let text = cx
                    .background_executor()
                    .spawn(async move {
                        let entries = snapshot
                            .as_ref()
                            .map_or(&[][..], |snapshot| snapshot.metadata.entries.as_slice());
                        nocterm_ai::thread::export::conversation_text(&title, entries)
                    })
                    .await;
                let _ = panel.update(cx, |panel, cx| {
                    cx.write_to_clipboard(ClipboardItem::new_string(text));
                    panel.menus.finish_copy();
                    cx.notify();
                });
            });
            panel.menus.retain_copy(task);
            cx.notify();
        });
    }
}
