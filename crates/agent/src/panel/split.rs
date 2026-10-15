//! The coordinator applies the history owner's concrete widening effect.
use super::AgentPanel;
use gpui_kit::{AnyElement, Context, Window};
use nocterm_workspace::RightPanelEvent;
impl AgentPanel {
    pub(super) fn set_history(&mut self, open: bool, cx: &mut Context<Self>) {
        if let Some(width) = self.layout.set_history(open, self.current().is_some(), cx) {
            cx.emit(RightPanelEvent::Widen(width));
        }
        cx.notify();
    }
    pub(super) fn render_split(
        &mut self,
        chat: AnyElement,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let history = self.render_history(cx);
        self.layout.render_split(chat, history, window, cx)
    }
}
