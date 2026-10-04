//! Chat history beside the chat: opening it widens the panel by a column the
//! user can resize, and both widths are remembered.
use super::AgentPanel;
use gpui_kit::{
    AnyElement, AppContext as _, Context, Entity, Pixels, Subscription, Window,
    component::{
        ActiveTheme as _, ResizablePanelEvent, ResizableState, h_resizable, resizable_panel,
    },
    div,
    prelude::*,
    px,
};
use nocterm_ui::LayoutMemory;
use nocterm_workspace::RightPanelEvent;

const HISTORY_WIDTH: &str = "agent.history";
const DEFAULT_WIDTH: Pixels = px(280.);

/// The resize state of the split with the history at column `history`:
/// after the chat when the panel is on the right, before it on the left.
pub(super) fn split_state(
    history: usize,
    cx: &mut Context<AgentPanel>,
) -> (Entity<ResizableState>, Subscription) {
    let split = cx.new(|_| ResizableState::default());
    let subscription = cx.subscribe(&split, move |_, split, _: &ResizablePanelEvent, cx| {
        if let Some(size) = split.read(cx).sizes().get(history).copied() {
            LayoutMemory::set(HISTORY_WIDTH, size, cx);
        }
    });
    (split, subscription)
}

impl AgentPanel {
    fn history_width(cx: &gpui_kit::App) -> Pixels {
        LayoutMemory::get(HISTORY_WIDTH, cx).unwrap_or(DEFAULT_WIDTH)
    }

    /// Shows or hides the history column, growing the panel to make room
    /// for it and giving the room back when it closes.
    pub(super) fn set_history(&mut self, open: bool, cx: &mut Context<Self>) {
        if self.history == open {
            return;
        }
        self.history = open;
        if open && self.current().is_some() && !self.maximized {
            let width = Self::history_width(cx);
            self.widened = Some(width);
            cx.emit(RightPanelEvent::Widen(width));
        } else if let Some(width) = self.widened.take() {
            cx.emit(RightPanelEvent::Widen(-width));
        }
        cx.notify();
    }

    /// The chat with its history beside it.
    pub(super) fn render_split(
        &mut self,
        chat: AnyElement,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let history = self.render_history(cx);
        let history = resizable_panel()
            .size(Self::history_width(cx))
            .size_range(px(180.)..px(640.))
            .child(
                div()
                    .id("agent-history-column")
                    .size_full()
                    .flex()
                    .flex_col()
                    .map(|column| {
                        if self.docked_left {
                            column.border_r_1()
                        } else {
                            column.border_l_1()
                        }
                    })
                    .border_color(cx.theme().border)
                    .child(history),
            );
        let chat = resizable_panel()
            .size_range(px(240.)..Pixels::MAX)
            .child(div().size_full().flex().flex_col().child(chat));
        let split = h_resizable("agent-split").with_state(&self.splits[self.docked_left as usize]);
        // On the left of the window the history goes on the outer side too.
        let split = if self.docked_left {
            split.child(history).child(chat)
        } else {
            split.child(chat).child(history)
        };
        div().flex_1().min_h_0().child(split).into_any_element()
    }
}
