//! An open chat: the transcript, the plan, pending approvals, the status
//! line and the composer.
use gpui_kit::{
    AnyElement, Context, Entity, SharedString, TestSupportExt as _, Window,
    component::{ActiveTheme as _, v_flex},
    div,
    prelude::*,
};

use super::{AgentPanel, MenuKind, widgets::activity_text};
use crate::thread::AgentThread;

impl AgentPanel {
    pub(super) fn on_reveal_tick(
        &mut self,
        binding: super::message_list::ActiveListBinding,
        cx: &mut Context<Self>,
    ) {
        let Some(thread) = self.current() else {
            return;
        };
        let reduced = cx.reduce_motion();
        let changed =
            self.messages
                .advance_reveal(&binding, thread.read(cx).history_entries(), reduced);
        if let Some(changed) = changed {
            self.messages.apply_reveal(changed, cx);
            cx.notify();
        }
    }
    pub(super) fn on_preview_completed(
        &mut self,
        binding: super::message_list::ActiveListBinding,
        ticket: super::image_pipeline::PreviewTicket,
        row: Option<usize>,
        image: Option<std::sync::Arc<gpui_kit::Image>>,
        cx: &mut Context<Self>,
    ) {
        if self
            .messages
            .accept_preview(&binding, ticket, row, image, cx)
        {
            cx.notify();
        }
    }
    pub(super) fn on_message_expanded(
        &mut self,
        binding: Option<super::message_list::ActiveListBinding>,
        row: usize,
        cx: &mut Context<Self>,
    ) {
        if let Some(binding) = binding
            && self.messages.toggle_expanded(&binding, row, cx)
        {
            cx.notify();
        }
    }
    fn chat_status(&self, thread: &Entity<AgentThread>, cx: &gpui_kit::App) -> Option<AnyElement> {
        let state = thread.read(cx);
        let visible = state.session_generating()
            || state.session().is_none()
            || state.session_sign_in_required()
            || state.document().pending_history()
            || state.has_status_error()
            || !state.session_accepts_updates();
        visible.then(|| {
            TranscriptView::status(
                thread.entity_id(),
                state.status(),
                state.session_generating() && state.session_accepts_updates(),
                cx,
            )
        })
    }
    pub(super) fn render_chat(
        &mut self,
        thread: Entity<AgentThread>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut body = v_flex().size_full().min_w_0().min_h_0();
        let plan = thread.read(cx).transcript().plan.clone();
        let usage = thread.read(cx).transcript().usage.clone();
        let dirty = thread
            .update(cx, |thread, _| thread.take_changed_rows())
            .into_iter()
            .collect::<Vec<_>>();
        self.messages.bind_chat(thread.entity_id(), cx);
        let rows = self.messages.observe_rows(
            thread.entity_id(),
            thread.read(cx).history_entries(),
            &dirty,
            thread.read(cx).session_generating(),
            cx.reduce_motion(),
        );
        self.messages.apply_rows(rows, cx);
        let scroller = self.messages.scroller(cx);
        let navigation = self.render_message_navigation(cx);
        let overlay = self
            .menus
            .is_open(&MenuKind::Usage)
            .then(|| self.render_usage(cx));
        body = body.child(TranscriptView::area(scroller, navigation, overlay, cx));
        if let Some(plan) = &plan {
            body = body.child(TranscriptView::plan(plan, cx));
        }
        if let Some(approvals) = self.render_approvals(&thread, window, cx) {
            body = body.child(approvals);
        }
        let labels = super::attachments::AttachmentLabels::new(self.host.workspace(), cx);
        let composer = self.render_composer(&thread, usage.as_ref(), &labels, cx);
        let status = self.chat_status(&thread, cx);
        body = body
            .children(status)
            .children(self.render_queue(&thread, &labels, window, cx))
            .children(self.render_commands(window, cx))
            .child(composer);
        body.into_any_element()
    }
}

struct TranscriptView;
impl TranscriptView {
    fn area(
        scroller: gpui_kit::component::message_scroller::MessageScroller,
        navigation: Option<AnyElement>,
        usage: Option<AnyElement>,
        cx: &gpui_kit::App,
    ) -> AnyElement {
        div()
            .id("agent-transcript-area")
            .test_support()
            .relative()
            .w_full()
            .min_w_0()
            .flex_1()
            .min_h_0()
            .child(
                scroller
                    .with_row_style(
                        gpui_kit::StyleRefinement::default()
                            .px_0()
                            .pr(gpui_kit::px(48.))
                            .pb_0(),
                    )
                    .with_bottom_fade(cx.theme().background)
                    .size_full(),
            )
            .children(navigation)
            .when_some(usage, |area, usage| {
                area.child(div().absolute().left_3().right_3().bottom_1().child(usage))
            })
            .into_any_element()
    }
    fn plan(plan: &nocterm_ai::acp::Plan, cx: &gpui_kit::App) -> AnyElement {
        v_flex()
            .p_2()
            .border_t_1()
            .border_color(cx.theme().border)
            .children(plan.entries.iter().map(|entry| {
                div()
                    .text_sm()
                    .child(format!("{:?} · {}", entry.status, entry.content))
            }))
            .into_any_element()
    }
    fn status(id: gpui_kit::EntityId, text: String, live: bool, cx: &gpui_kit::App) -> AnyElement {
        div()
            .id("agent-status")
            .test_support()
            .px_3()
            .pb_2()
            .text_xs()
            .text_color(cx.theme().muted_foreground)
            .child(activity_text(
                SharedString::from(format!("agent-live-status-{id}")),
                text,
                live,
                cx,
            ))
            .into_any_element()
    }
}
