//! User-message anchors stay independent of virtual row heights and tail following.
use super::AgentPanel;
use gpui_kit::{
    AnyElement, Context, EntityId, ListOffset, TestSupportExt as _,
    component::{
        ActiveTheme as _, Disableable as _, Sizable as _,
        button::{Button, ButtonVariants as _},
        v_flex,
    },
    prelude::*,
    px, rems,
};
use nocterm_ai::thread::Entry;
use nocterm_ui::IconName;

#[derive(Default)]
pub(super) struct MessageNavigation {
    owner: Option<EntityId>,
    known_rows: usize,
    anchors: Vec<usize>,
    #[cfg(test)]
    inspections: usize,
}

impl MessageNavigation {
    pub(super) fn clear(&mut self) {
        *self = Self::default();
    }

    /// Inspect only appended rows and existing rows whose classification may have changed.
    pub(super) fn observe(&mut self, owner: EntityId, entries: &[Entry], dirty: &[usize]) {
        if self.owner != Some(owner) || entries.len() < self.known_rows {
            self.clear();
            self.owner = Some(owner);
        }
        let known = self.known_rows;
        for (index, entry) in entries.iter().enumerate().skip(known) {
            if self.is_user_message(entry) {
                self.anchors.push(index);
            }
        }
        for &index in dirty {
            if index >= known {
                continue;
            }
            let Some(entry) = entries.get(index) else {
                continue;
            };
            let message = self.is_user_message(entry);
            match (self.anchors.binary_search(&index), message) {
                (Ok(position), false) => {
                    self.anchors.remove(position);
                }
                (Err(position), true) => self.anchors.insert(position, index),
                _ => {}
            }
        }
        self.known_rows = entries.len();
    }

    fn is_user_message(&mut self, entry: &Entry) -> bool {
        #[cfg(test)]
        {
            self.inspections += 1;
        }
        matches!(entry, Entry::User(_))
    }

    fn previous(&self, top: ListOffset) -> Option<usize> {
        let end = self.anchors.partition_point(|&index| {
            index < top.item_ix || (index == top.item_ix && top.offset_in_item > px(0.))
        });
        end.checked_sub(1).map(|index| self.anchors[index])
    }

    fn first(&self, top: ListOffset) -> Option<usize> {
        (self.known_rows > 0 && (top.item_ix > 0 || top.offset_in_item > px(0.))).then_some(0)
    }
}

impl AgentPanel {
    fn jump_to_message(&mut self, first: bool, cx: &mut Context<Self>) {
        if self.current().map(|thread| thread.entity_id()) != self.navigation.owner {
            return;
        }
        let top = self.list.read(cx).logical_scroll_top();
        let target = if first {
            self.navigation.first(top)
        } else {
            self.navigation.previous(top)
        };
        if let Some(index) = target {
            self.list.update(cx, |list, cx| {
                list.scroll_to_item(index, cx);
            });
            cx.notify();
        }
    }

    pub(super) fn render_message_navigation(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let list = self.list.read(cx);
        if !list.is_scrolled_up() || self.navigation.known_rows == 0 {
            return None;
        }
        let top = list.logical_scroll_top();
        let previous = self.navigation.previous(top);
        let first = self.navigation.first(top);
        Some(
            v_flex()
                .id("agent-message-navigation")
                .test_support()
                .absolute()
                .right_3()
                .bottom(rems(1.))
                .px_1()
                .py_0()
                .rounded(cx.theme().radius_full())
                .border_1()
                .border_color(cx.theme().border)
                .bg(cx.theme().background)
                .child(
                    Button::new("agent-first-message")
                        .ghost()
                        .small()
                        .icon(IconName::ChevronsUp)
                        .rounded(cx.theme().radius_full())
                        .tooltip("Top of chat")
                        .accessibility_label("Top of chat")
                        .disabled(first.is_none())
                        .on_click(cx.listener(|this, _, _, cx| this.jump_to_message(true, cx))),
                )
                .child(
                    Button::new("agent-previous-message")
                        .ghost()
                        .small()
                        .icon(IconName::ArrowUp)
                        .rounded(cx.theme().radius_full())
                        .tooltip("Previous user message")
                        .accessibility_label("Previous user message")
                        .disabled(previous.is_none())
                        .on_click(cx.listener(|this, _, _, cx| this.jump_to_message(false, cx))),
                )
                .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests;
