//! An open chat: the transcript, the plan, pending approvals, the status
//! line and the composer.
use gpui_kit::{
    AnyElement, SharedString, TestSupportExt as _,
    component::{ActiveTheme as _, v_flex},
    div,
    prelude::*,
};

use super::widgets::activity_text;

pub(super) struct TranscriptView;
impl TranscriptView {
    pub(super) fn area(
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
    pub(super) fn plan(plan: &nocterm_ai::acp::Plan, cx: &gpui_kit::App) -> AnyElement {
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
    pub(super) fn status(
        id: gpui_kit::EntityId,
        text: String,
        live: bool,
        cx: &gpui_kit::App,
    ) -> AnyElement {
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
