//! The queue peeks from behind the composer and unfolds above it.
use super::AgentPanel;
use crate::thread::AgentThread;
use base64::Engine as _;
use gpui_kit::{
    AnyElement, Context, Entity, EntityId, Focusable as _, SharedString, Task, TestSupportExt as _,
    Window,
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
use nocterm_ai::{acp, images::PromptImage};
use nocterm_ui::{ActiveAi as _, IconName};
use std::time::Duration;

#[derive(Default)]
pub(super) struct ComposerState {
    pub edit: Option<QueueEdit>,
    /// Text still belongs to the previously displayed chat until hydration runs.
    loading: Option<(EntityId, String)>,
    pub revision: u64,
    pub pending_images: usize,
}
pub(super) struct QueueEdit {
    pub thread: EntityId,
    pub id: u64,
    draft: String,
    ready: bool,
    preparation: Option<Task<()>>,
}
impl AgentPanel {
    pub(super) fn preparing_images(&self) -> bool {
        self.composer.pending_images > 0 || self.preparing_queue_edit()
    }
    pub(super) fn preparing_queue_edit(&self) -> bool {
        self.composer.edit.as_ref().is_some_and(|edit| !edit.ready)
    }
    pub(super) fn original_composer_text(&self, cx: &gpui_kit::App) -> String {
        self.composer.edit.as_ref().map_or_else(
            || {
                let value = self.input.read(cx).value().to_string();
                if self
                    .composer
                    .loading
                    .as_ref()
                    .is_some_and(|(_, previous)| previous == &value)
                {
                    self.current()
                        .and_then(|thread| thread.read(cx).draft.clone())
                        .unwrap_or_default()
                } else {
                    value
                }
            },
            |edit| edit.draft.clone(),
        )
    }
    /// Restore the original composer without overriding any stop or error pause.
    fn restore_queue_edit(&mut self, resume: bool, cx: &mut Context<Self>) -> Option<String> {
        let edit = self.composer.edit.take()?;
        self.composer.revision = self.composer.revision.wrapping_add(1);
        self.composer.pending_images = 0;
        if let Some(thread) = self
            .threads
            .iter()
            .find(|thread| thread.entity_id() == edit.thread)
            .cloned()
        {
            thread.update(cx, |thread, cx| thread.end_composer_edit(resume, cx));
        }
        Some(edit.draft)
    }
    pub(super) fn leave_composer(&mut self, cx: &mut Context<Self>) {
        self.leave_composer_with(false, cx);
    }
    pub(super) fn leave_composer_with(&mut self, discard: bool, cx: &mut Context<Self>) {
        self.reset_commands();
        self.composer.revision = self.composer.revision.wrapping_add(1);
        self.composer.pending_images = 0;
        let draft = self
            .restore_queue_edit(!discard, cx)
            .unwrap_or_else(|| self.original_composer_text(cx));
        if !discard && let Some(thread) = self.current() {
            thread.update(cx, |thread, cx| {
                thread.set_draft(draft, cx);
                thread.save(cx);
            });
        }
        self.queue_open = false;
    }
    pub(super) fn load_composer(&mut self, cx: &mut Context<Self>) {
        let Some(thread) = self.current() else {
            return;
        };
        let id = thread.entity_id();
        let revision = self.composer.revision;
        self.composer.loading = Some((id, self.input.read(cx).value().to_string()));
        let Some(window) = thread.read(cx).window else {
            return;
        };
        let panel = cx.weak_entity();
        cx.defer(move |cx| {
            let _ = cx.update_window(window, |_, window, cx| {
                let _ = panel.update(cx, |panel, cx| {
                    if panel
                        .current()
                        .is_some_and(|thread| thread.entity_id() == id)
                        && panel.composer.revision == revision
                    {
                        panel.sync_composer_draft(cx);
                        if panel.composer.revision != revision {
                            return;
                        }
                        let draft = panel
                            .current()
                            .unwrap()
                            .read(cx)
                            .draft
                            .clone()
                            .unwrap_or_default();
                        panel.composer.loading = None;
                        panel
                            .input
                            .update(cx, |input, cx| input.set_value(draft, window, cx));
                    }
                });
            });
        });
    }

    fn edit_queued(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        if self.composer.edit.is_some() || self.preparing_images() {
            return;
        }
        let Some(thread) = self.current() else {
            return;
        };
        let Some(prompt) = thread
            .read(cx)
            .queue
            .iter()
            .find(|prompt| prompt.saved.id == id)
            .cloned()
        else {
            return;
        };
        self.composer.revision = self.composer.revision.wrapping_add(1);
        self.composer.pending_images = 0;
        let revision = self.composer.revision;
        let draft = self.original_composer_text(cx);
        self.composer.loading = None;
        thread.update(cx, |thread, cx| thread.set_draft(draft.clone(), cx));
        let epoch = thread.read(cx).epoch;
        let ready = prompt.saved.images.is_empty();
        self.composer.edit = Some(QueueEdit {
            thread: thread.entity_id(),
            id,
            draft,
            ready,
            preparation: None,
        });
        thread.update(cx, |thread, cx| {
            thread.begin_composer_edit(Vec::new(), prompt.attachments);
            cx.notify();
        });
        if !ready {
            let saved = prompt.saved.clone();
            let future = cx.background_executor().spawn(async move {
                saved
                    .images
                    .iter()
                    .filter_map(|block| match block {
                        acp::ContentBlock::Image(image) => Some(
                            base64::engine::general_purpose::STANDARD
                                .decode(&image.data)
                                .map_err(|error| error.to_string())
                                .and_then(PromptImage::validate),
                        ),
                        _ => None,
                    })
                    .collect::<Result<Vec<_>, _>>()
            });
            let owner = thread.downgrade();
            let task = cx.spawn(async move |this, cx| {
                let result = future.await;
                let _ = this.update(cx, |panel, cx| {
                    if panel.composer.revision != revision
                        || !panel
                            .composer
                            .edit
                            .as_ref()
                            .is_some_and(|edit| edit.id == id && !edit.ready)
                    {
                        return;
                    }
                    let _ = owner.update(cx, |thread, cx| {
                        if thread.epoch != epoch || !cx.ai_enabled() {
                            return;
                        }
                        match result {
                            Ok(images) => {
                                thread.images = images;
                                if let Some(edit) = &mut panel.composer.edit {
                                    edit.ready = true;
                                }
                            }
                            Err(error) => panel.error = Some(error),
                        }
                        cx.notify();
                    });
                    cx.notify();
                });
            });
            self.composer.edit.as_mut().unwrap().preparation = Some(task);
        }
        self.reset_commands();
        self.input.update(cx, |input, cx| {
            input.set_value(prompt.saved.text.clone(), window, cx)
        });
        self.queue_open = true;
        window.focus(&self.input.read(cx).focus_handle(cx), cx);
        cx.notify();
    }
    pub(super) fn finish_queue_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(draft) = self.restore_queue_edit(true, cx) else {
            return;
        };
        self.reset_commands();
        self.input
            .update(cx, |input, cx| input.set_value(draft, window, cx));
    }
    pub(super) fn render_queue(
        &mut self,
        thread: &Entity<AgentThread>,
        labels: &super::attachments::AttachmentLabels,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if thread.read(cx).queue.is_empty() {
            return None;
        }
        let count = thread.read(cx).queue.len();
        let first = thread.read(cx).queue[0].saved.id;
        let measured = self
            .queue_heights
            .get(&thread.entity_id())
            .copied()
            .unwrap_or(px(0.));
        let target = measured.min(px(240.));
        let height = transition(
            (
                SharedString::from(format!("agent-queue-{}", thread.entity_id())),
                "height",
            ),
            if self.queue_open { target } else { px(0.) },
            Transition::new(Duration::from_millis(220)).ease(|t| {
                if t >= 1. {
                    1.
                } else {
                    1. - 2_f32.powf(-10. * t)
                }
            }),
            window,
            cx,
        );
        let mut hood = v_flex()
            .id("agent-queue")
            .test_support()
            .mx_3()
            .mb(px(-28.))
            .pb(px(16.))
            .rounded_t(px(16.))
            .bg(cx.theme().muted.blend(cx.theme().background.opacity(0.35)))
            .border_1()
            .border_color(cx.theme().border)
            .min_w_0();
        hood = hood.child(self.queue_header(count, first, cx));
        if !self.queue_open && height <= px(0.) {
            return Some(hood.into_any_element());
        }
        let mut rows = v_flex()
            .id("agent-queue-rows")
            .test_support()
            .relative()
            .w_full()
            .flex_shrink_0()
            .min_w_0()
            .gap_2()
            .p_2();
        for prompt in thread.read(cx).queue.clone() {
            rows = rows.child(self.queue_row(&prompt, labels, cx));
        }
        // Measure the actual laid-out rows, including wrapping and image previews.
        // The measurement element is absolute and never contributes height itself.
        let owner = cx.weak_entity();
        let thread_id = thread.entity_id();
        rows = rows.child(
            canvas(
                move |bounds, _, cx| {
                    let owner = owner.clone();
                    cx.defer(move |cx| {
                        let _ = owner.update(cx, |panel, cx| {
                            if panel
                                .queue_heights
                                .get(&thread_id)
                                .is_none_or(|old| (*old - bounds.size.height).abs() > px(0.5))
                            {
                                panel.queue_heights.insert(thread_id, bounds.size.height);
                                cx.notify();
                            }
                        });
                    });
                },
                |_, _, _, _| {},
            )
            .absolute()
            .inset_0(),
        );
        hood = hood.child(
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
        if let Some(edit) = &self.composer.edit
            && edit.thread == thread.entity_id()
        {
            hood = hood.child(
                h_flex()
                    .px_2()
                    .gap_2()
                    .child(div().text_xs().child("Editing queued message"))
                    .child(
                        Button::new(SharedString::from("queue-cancel-edit"))
                            .ghost()
                            .xsmall()
                            .label("Cancel")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.finish_queue_edit(window, cx)
                            })),
                    ),
            );
        }
        Some(hood.into_any_element())
    }
    fn queue_header(&mut self, count: usize, first: u64, cx: &mut Context<Self>) -> AnyElement {
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
                    .icon(if self.queue_open {
                        IconName::ChevronDown
                    } else {
                        IconName::ChevronUp
                    })
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.queue_open = !this.queue_open;
                        cx.notify();
                    })),
            )
            .child(
                Button::new("queue-send-now")
                    .ghost()
                    .small()
                    .icon(IconName::ArrowUp)
                    .tooltip("Send now · stop current response")
                    .disabled(self.composer.edit.is_some())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(thread) = this.current() {
                            thread.update(cx, |thread, cx| thread.send_now(first, cx));
                        }
                    })),
            )
            .child(
                Button::new("queue-edit-first")
                    .ghost()
                    .small()
                    .icon(IconName::Pencil)
                    .tooltip("Edit queued message")
                    .disabled(self.preparing_images())
                    .on_click(
                        cx.listener(move |this, _, window, cx| this.edit_queued(first, window, cx)),
                    ),
            )
            .into_any_element()
    }
    fn queue_row(
        &mut self,
        prompt: &crate::thread::QueuedPrompt,
        labels: &super::attachments::AttachmentLabels,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id = prompt.saved.id;
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
                            .child(prompt.saved.text.clone()),
                    )
                    .child(
                        Button::new(("queued-send-now", id))
                            .ghost()
                            .xsmall()
                            .icon(IconName::ArrowUp)
                            .tooltip("Send now")
                            .disabled(self.composer.edit.is_some())
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if let Some(thread) = this.current() {
                                    thread.update(cx, |thread, cx| thread.send_now(id, cx));
                                }
                            })),
                    )
                    .child(
                        Button::new(("queued-edit", id))
                            .ghost()
                            .xsmall()
                            .icon(IconName::Pencil)
                            .tooltip("Edit")
                            .disabled(self.preparing_images())
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.edit_queued(id, window, cx)
                            })),
                    ),
            );
        for attachment in &prompt.attachments {
            row = row.child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(labels.label(attachment)),
            );
        }
        let mut previews = h_flex().flex_wrap().gap_1();
        for (block, key) in prompt.saved.images.iter().zip(prompt.image_keys.iter()) {
            if let acp::ContentBlock::Image(image) = block {
                let Some(key) = *key else {
                    continue;
                };
                let image = self.cached_image(key, || image.data.as_bytes().to_vec(), true, cx);
                previews = previews.child(div().size(rems(3.5)).overflow_hidden().child(image));
            }
        }
        row = row.child(previews);
        row.into_any_element()
    }
}

impl AgentPanel {
    pub(super) fn sync_composer_draft(&mut self, cx: &mut Context<Self>) {
        if self.composer.edit.is_none()
            && let Some(thread) = self.current()
        {
            let text = self.input.read(cx).value().to_string();
            if let Some((owner, previous)) = &self.composer.loading {
                if *owner != thread.entity_id() || previous == &text {
                    return;
                }
                self.composer.loading = None;
                self.composer.revision = self.composer.revision.wrapping_add(1);
            }
            thread.update(cx, |thread, cx| thread.set_draft(text, cx));
        }
    }
}
