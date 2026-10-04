//! An open chat: the transcript, the plan, pending approvals, the status
//! line and the composer.
use gpui_kit::{
    AnyElement, Context, Entity, SharedString, TestSupportExt as _, Window,
    component::{ActiveTheme as _, v_flex},
    div, list,
    prelude::*,
    px,
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
                self.list
                    .splice(self.list_count..self.list_count, count - self.list_count);
            } else {
                self.list.reset(count);
            }
            self.list_count = count;
        } else if count > 0 {
            self.list.remeasure_items(0..count);
        }
        body = body.child(
            div()
                .relative()
                .w_full()
                .min_w_0()
                .flex_1()
                .min_h_0()
                .child(
                    list(
                        self.list.clone(),
                        cx.processor(|this, index, _, cx| this.render_entry(index, cx)),
                    )
                    .size_full(),
                )
                // The transcript fades out above the composer instead of
                // ending at a hard edge.
                .child({
                    let background = cx.theme().background;
                    div()
                        .absolute()
                        .left_0()
                        .right_0()
                        .bottom_0()
                        .h(px(28.))
                        .bg(gpui_kit::linear_gradient(
                            180.,
                            gpui_kit::linear_color_stop(background.opacity(0.), 0.),
                            gpui_kit::linear_color_stop(background, 1.),
                        ))
                })
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
        let composer = self.render_composer(&thread, &state, cx);
        body = body
            .when(
                thread.read(cx).generating
                    || thread.read(cx).session.is_none()
                    || thread.read(cx).auth_required
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
            .child(composer);
        body.into_any_element()
    }
}
