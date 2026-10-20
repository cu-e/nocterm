//! The queue peeks from behind the composer and unfolds above it.
use super::{AgentPanel, images::ImageAttachError};
use crate::thread::AgentThread;
use base64::Engine as _;
use gpui_kit::{
    AnyElement, Context, Entity, EntityId, SharedString, TestSupportExt as _, Window,
    base::motion::{Transition, transition},
    canvas,
    component::{
        ActiveTheme as _, Disableable as _, Sizable as _,
        button::{Button, ButtonVariants as _},
        h_flex, v_flex,
    },
    div,
    prelude::*,
    px, rems,
};
use nocterm_ui::IconName;
use std::time::Duration;

pub(super) fn decode_saved_image(data: &str) -> Result<Vec<u8>, ImageAttachError> {
    if data.len() > nocterm_ai::images::MAX_IMAGE_BYTES.div_ceil(3) * 4 {
        return Err(nocterm_ai::images::PromptImageError::TooLarge.into());
    }
    Ok(base64::engine::general_purpose::STANDARD.decode(data)?)
}

/// One immutable frame of the queue, without authoritative queued messages.
pub(super) struct QueueFrame {
    chat: EntityId,
    open: bool,
    measured: gpui_kit::Pixels,
    editing: bool,
}
pub(super) struct QueueRenderer;
impl QueueRenderer {
    pub(super) fn height(
        frame: &QueueFrame,
        window: &mut Window,
        cx: &mut Context<AgentPanel>,
    ) -> gpui_kit::Pixels {
        transition(
            (
                SharedString::from(format!("agent-queue-{}", frame.chat)),
                "height",
            ),
            if frame.open {
                frame.measured.min(px(240.))
            } else {
                px(0.)
            },
            Transition::new(Duration::from_millis(220)).ease(|t| {
                if t >= 1. {
                    1.
                } else {
                    1. - 2_f32.powf(-10. * t)
                }
            }),
            window,
            cx,
        )
    }
    pub(super) fn hood(
        header: AnyElement,
        cx: &gpui_kit::App,
    ) -> gpui_kit::Stateful<gpui_kit::Div> {
        v_flex()
            .id("agent-queue")
            .mx_3()
            .mb(px(-28.))
            .pb(px(16.))
            .rounded_t(px(16.))
            .bg(cx.theme().muted.blend(cx.theme().background.opacity(0.35)))
            .border_1()
            .border_color(cx.theme().border)
            .min_w_0()
            .child(header)
    }
    fn rows(frame: &QueueFrame, rows: Vec<AnyElement>, cx: &mut Context<AgentPanel>) -> AnyElement {
        let owner = cx.weak_entity();
        let chat = frame.chat;
        v_flex()
            .id("agent-queue-rows")
            .test_support()
            .relative()
            .w_full()
            .flex_shrink_0()
            .min_w_0()
            .gap_2()
            .p_2()
            .children(rows)
            .child(
                canvas(
                    move |bounds, _, cx| {
                        let owner = owner.clone();
                        cx.defer(move |cx| {
                            let _ = owner.update(cx, |panel, cx| {
                                panel.on_queue_height_observed(chat, bounds.size.height, cx)
                            });
                        });
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .inset_0(),
            )
            .into_any_element()
    }
    pub(super) fn render(
        frame: &QueueFrame,
        header: AnyElement,
        rows: Vec<AnyElement>,
        height: gpui_kit::Pixels,
        cx: &mut Context<AgentPanel>,
    ) -> AnyElement {
        let target = frame.measured.min(px(240.));
        let rows = Self::rows(frame, rows, cx);
        let hood = Self::hood(header, cx).child(
            div()
                .id("agent-queue-reveal")
                .test_support()
                .h(height)
                .overflow_hidden()
                .opacity(if target > px(0.) {
                    (height / target).clamp(0., 1.)
                } else {
                    0.
                })
                .child(
                    div()
                        .id("agent-queue-scroll")
                        .test_support()
                        .max_h(px(240.))
                        .overflow_y_scroll()
                        .child(rows),
                ),
        );
        hood.when(frame.editing, |hood| {
            hood.child(
                h_flex()
                    .px_2()
                    .gap_2()
                    .child(div().text_xs().child("Editing queued message"))
                    .child(
                        Button::new(SharedString::from("queue-cancel-edit"))
                            .ghost()
                            .xsmall()
                            .label("Cancel")
                            .on_click(cx.listener(|panel, _, window, cx| {
                                panel.finish_queue_edit(window, cx)
                            })),
                    ),
            )
        })
        .test_support()
        .into_any_element()
    }
}

impl QueueRenderer {
    fn queue_header(
        composer: &super::composer_view::ComposerView,
        origin: EntityId,
        count: usize,
        first: u64,
        cx: &mut Context<AgentPanel>,
    ) -> AnyElement {
        h_flex()
            .w_full()
            .min_w_0()
            .px_2()
            .py_1()
            .gap_1()
            .child(
                Button::new("queue-toggle")
                    .ghost()
                    .small()
                    .flex_1()
                    .min_w_0()
                    .label(format!(
                        "{count} {} queued",
                        if count == 1 { "message" } else { "messages" }
                    ))
                    .icon(if composer.queue_open() {
                        IconName::ChevronDown
                    } else {
                        IconName::ChevronUp
                    })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.on_queue_toggled(origin, cx);
                    })),
            )
            .child(
                Button::new("queue-send-now")
                    .ghost()
                    .small()
                    .icon(IconName::ArrowUp)
                    .tooltip("Send now · stop current response")
                    .disabled(composer.editing())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.on_queue_send_now(origin, first, cx);
                    })),
            )
            .child(
                Button::new("queue-edit-first")
                    .ghost()
                    .small()
                    .icon(IconName::Pencil)
                    .tooltip("Edit queued message")
                    .disabled(composer.preparing())
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.on_queue_edit_requested(origin, first, window, cx)
                    })),
            )
            .child(
                Button::new("queue-remove-first")
                    .ghost()
                    .small()
                    .icon(IconName::Trash)
                    .tooltip("Remove queued message")
                    .disabled(composer.editing_id() == Some(first))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.on_queue_remove_requested(origin, first, cx)
                    })),
            )
            .into_any_element()
    }
}

impl QueueRenderer {
    fn queue_row(
        composer: &super::composer_view::ComposerView,
        origin: EntityId,
        prompt: &crate::thread::QueuedPrompt,
        previews: Vec<AnyElement>,
        labels: &super::attachments::AttachmentLabels,
        cx: &mut Context<AgentPanel>,
    ) -> AnyElement {
        let id = prompt.saved().id;
        let mut row = v_flex()
            .id(("queued-message", id))
            .test_support()
            .min_w_0()
            .gap_1()
            .child(
                h_flex()
                    .min_w_0()
                    .gap_1()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_sm()
                            .whitespace_normal()
                            .child(prompt.saved().text.clone()),
                    )
                    .child(
                        Button::new(("queued-send-now", id))
                            .ghost()
                            .xsmall()
                            .icon(IconName::ArrowUp)
                            .tooltip("Send now")
                            .disabled(composer.editing())
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.on_queue_send_now(origin, id, cx);
                            })),
                    )
                    .child(
                        Button::new(("queued-edit", id))
                            .ghost()
                            .xsmall()
                            .icon(IconName::Pencil)
                            .tooltip("Edit")
                            .disabled(composer.preparing())
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.on_queue_edit_requested(origin, id, window, cx)
                            })),
                    )
                    .child(
                        Button::new(("queued-remove", id))
                            .ghost()
                            .xsmall()
                            .icon(IconName::Trash)
                            .tooltip("Remove")
                            .disabled(composer.editing_id() == Some(id))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.on_queue_remove_requested(origin, id, cx)
                            })),
                    ),
            );
        for attachment in prompt.attachments() {
            row = row.child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(labels.label(attachment)),
            );
        }
        row = row.child(
            h_flex().flex_wrap().gap_1().children(
                previews
                    .into_iter()
                    .map(|image| div().size(rems(3.5)).overflow_hidden().child(image)),
            ),
        );
        row.into_any_element()
    }
}

impl QueueRenderer {
    pub(super) fn render_queue(
        composer: &super::composer_view::ComposerView,
        prompts: Vec<crate::thread::QueuedPrompt>,
        previews: Vec<Vec<AnyElement>>,
        thread: &Entity<AgentThread>,
        labels: &super::attachments::AttachmentLabels,
        height: gpui_kit::Pixels,
        cx: &mut Context<AgentPanel>,
    ) -> Option<AnyElement> {
        let origin = thread.entity_id();
        if thread.read(cx).composer().queue().is_empty() {
            return None;
        }
        let count = thread.read(cx).queued_prompt_ids().count();
        let first = thread.read(cx).composer().queue()[0].saved().id;
        let frame = QueueFrame {
            chat: thread.entity_id(),
            open: composer.queue_open(),
            measured: composer.queue_height(thread.entity_id()),
            editing: composer.editing_chat() == Some(thread.entity_id()),
        };
        let header = Self::queue_header(composer, origin, count, first, cx);
        if !frame.open && height <= px(0.) {
            return Some(
                QueueRenderer::hood(header, cx)
                    .test_support()
                    .into_any_element(),
            );
        }
        let rows = prompts
            .iter()
            .zip(previews)
            .map(|(prompt, images)| Self::queue_row(composer, origin, prompt, images, labels, cx))
            .collect();
        Some(Self::render(&frame, header, rows, height, cx))
    }
}

impl QueueRenderer {
    pub(super) fn visible_height(
        composer: &super::composer_view::ComposerView,
        thread: &Entity<AgentThread>,
        window: &mut Window,
        cx: &mut Context<AgentPanel>,
    ) -> gpui_kit::Pixels {
        let frame = QueueFrame {
            chat: thread.entity_id(),
            open: composer.queue_open(),
            measured: composer.queue_height(thread.entity_id()),
            editing: composer.editing_chat() == Some(thread.entity_id()),
        };
        Self::height(&frame, window, cx)
    }
}
