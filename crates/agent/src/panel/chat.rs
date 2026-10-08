//! An open chat: the transcript, the plan, pending approvals, the status
//! line and the composer.
use gpui_kit::{
    AnyElement, Context, Entity, SharedString, TestSupportExt as _, Window,
    component::{ActiveTheme as _, message_scroller::MessageScroller, v_flex},
    div,
    prelude::*,
};

use super::{AgentPanel, MenuKind, widgets::activity_text};
use crate::thread::AgentThread;

impl AgentPanel {
    pub(super) fn render_chat(
        &mut self,
        thread: Entity<AgentThread>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut body = v_flex().size_full().min_w_0().min_h_0();
        let model = &thread.read(cx).state;
        let count = model.entries.len();
        let state = nocterm_ai::thread::ThreadState {
            config_options: model.config_options.clone(),
            modes: model.modes.clone(),
            current_mode: model.current_mode.clone(),
            plan: model.plan.clone(),
            usage: model.usage.clone(),
            ..Default::default()
        };
        if count != self.list_count {
            if count > self.list_count {
                self.list.update(cx, |list, cx| {
                    list.append(count - self.list_count, cx);
                });
            } else {
                self.list.update(cx, |list, cx| list.reset(count, cx));
            }
            self.list_count = count;
        }
        let dirty = thread.update(cx, |thread, _| std::mem::take(&mut thread.dirty_rows));
        let dirty = dirty.into_iter().collect::<Vec<_>>();
        self.sync_stream(&dirty, cx);
        body = body.child(
            div()
                .id("agent-transcript-area")
                .test_support()
                .relative()
                .w_full()
                .min_w_0()
                .flex_1()
                .min_h_0()
                .child(
                    MessageScroller::new(
                        "agent-transcript",
                        self.list.clone(),
                        cx.processor(|this, index, _, cx| this.render_entry(index, cx)),
                    )
                    .with_row_style(gpui_kit::StyleRefinement::default().px_0().pb_0())
                    .with_bottom_fade(cx.theme().background)
                    .size_full(),
                )
                // The usage card lies over the chat, just above the composer.
                .when(self.menu == Some(MenuKind::Usage), |area| {
                    area.child(
                        div()
                            .absolute()
                            .left_3()
                            .right_3()
                            .bottom_1()
                            .child(self.render_usage(cx)),
                    )
                }),
        );
        if let Some(plan) = &state.plan {
            body = body.child(
                v_flex()
                    .p_2()
                    .border_t_1()
                    .border_color(cx.theme().border)
                    .children(plan.entries.iter().map(|entry| {
                        div()
                            .text_sm()
                            .child(format!("{:?} · {}", entry.status, entry.content))
                    })),
            );
        }
        if let Some(approvals) = self.render_approvals(&thread, window, cx) {
            body = body.child(approvals);
        }
        let labels = super::attachments::AttachmentLabels::new(&self.workspace, cx);
        let composer = self.render_composer(&thread, &state, &labels, cx);
        body = body
            .when(
                thread.read(cx).generating
                    || thread.read(cx).session().is_none()
                    || thread.read(cx).auth_required
                    || thread.read(cx).fallback_history
                    || thread.read(cx).status_error
                    || !thread.read(cx).accept_updates,
                |body| {
                    body.child(
                        div()
                            .id("agent-status")
                            .test_support()
                            .px_3()
                            .pb_2()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(activity_text(
                                SharedString::from(format!(
                                    "agent-live-status-{}",
                                    thread.entity_id()
                                )),
                                thread.read(cx).status.clone(),
                                thread.read(cx).generating && thread.read(cx).accept_updates,
                                cx,
                            )),
                    )
                },
            )
            .children(self.render_queue(&thread, &labels, window, cx))
            .children(self.render_commands(window, cx))
            .child(composer);
        body.into_any_element()
    }
}
