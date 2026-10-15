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
use nocterm_ai::{acp, images::PromptImage};
use nocterm_ui::IconName;
use std::time::Duration;

impl AgentPanel {
    pub(super) fn preparing_images(&self) -> bool {
        self.composer.preparing()
    }
    pub(super) fn preparing_queue_edit(&self) -> bool {
        self.composer.preparing_edit()
    }
    pub(super) fn original_composer_text(&self, cx: &gpui_kit::App) -> String {
        if let Some(draft) = self.composer.original_draft() {
            return draft.to_owned();
        }
        let value = self.composer_text(cx).to_string();
        if self.composer.waiting_for_draft(&value) {
            self.current()
                .and_then(|thread| thread.read(cx).draft_text().map(str::to_owned))
                .unwrap_or_default()
        } else {
            value
        }
    }
    fn restore_queue_edit(&mut self, resume: bool, cx: &mut Context<Self>) -> Option<String> {
        let (chat, draft) = self.composer.take_edit()?.into_parts();
        if let Some(thread) = self
            .threads
            .iter()
            .find(|thread| thread.entity_id() == chat)
            .cloned()
        {
            thread.update(cx, |thread, cx| thread.end_composer_edit(resume, cx));
        }
        Some(draft)
    }
    pub(super) fn leave_composer(&mut self, cx: &mut Context<Self>) {
        self.leave_composer_with(false, cx);
    }
    pub(super) fn leave_composer_with(&mut self, discard: bool, cx: &mut Context<Self>) {
        self.composer.leave();
        let draft = self
            .restore_queue_edit(!discard, cx)
            .unwrap_or_else(|| self.original_composer_text(cx));
        if !discard && let Some(thread) = self.current() {
            thread.update(cx, |thread, cx| {
                thread.set_draft(draft, cx);
                thread.save(cx);
            });
        }
    }
    pub(super) fn load_composer(&mut self, cx: &mut Context<Self>) {
        let Some(thread) = self.current() else {
            return;
        };
        let binding = self.composer.bind_loading(thread.entity_id(), cx);
        let Some(window) = thread.read(cx).window() else {
            return;
        };
        let owner = thread.downgrade();
        let panel = cx.weak_entity();
        cx.defer(move |cx| {
            let _ = cx.update_window(window, |_, window, cx| {
                let _ = panel.update(cx, |panel, cx| {
                    panel.on_draft_hydrated(binding, owner, window, cx)
                });
            });
        });
    }
    fn on_draft_hydrated(
        &mut self,
        binding: super::composer_view::ComposerBinding,
        owner: gpui_kit::WeakEntity<AgentThread>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(thread) = owner.upgrade() else {
            return;
        };
        if self
            .current()
            .is_none_or(|current| current.entity_id() != thread.entity_id())
            || !self.composer.current(&binding)
        {
            return;
        }
        self.sync_composer_draft(cx);
        if !self.composer.current(&binding) {
            return;
        }
        let archived = thread.read(cx).is_archived();
        let draft = if archived {
            String::new()
        } else {
            thread
                .read(cx)
                .composer()
                .draft()
                .cloned()
                .unwrap_or_default()
        };
        self.composer
            .finish_loading(&binding, draft, archived, window, cx);
    }
    fn edit_queued(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        if self.composer.editing() || self.preparing_images() {
            return;
        }
        let Some(thread) = self.current() else {
            return;
        };
        let Some(prompt) = thread
            .read(cx)
            .composer()
            .queue()
            .iter()
            .find(|prompt| prompt.saved.id == id)
            .cloned()
        else {
            return;
        };
        let draft = self.original_composer_text(cx);
        let ready = prompt.saved.images.is_empty();
        let binding = self
            .composer
            .begin_edit(thread.entity_id(), id, draft.clone(), ready);
        thread.update(cx, |thread, cx| {
            thread.set_draft(draft, cx);
            thread.begin_composer_edit(Vec::new(), prompt.attachments, cx);
            cx.notify();
        });
        if !ready {
            self.prepare_queue_images(&thread, id, binding, prompt.saved.clone(), cx);
        }
        self.reset_commands();
        self.composer
            .set_text(prompt.saved.text.clone(), window, cx);
        self.composer.open_queue();
        self.focus_composer(window, cx);
        cx.notify();
    }
    fn prepare_queue_images(
        &mut self,
        thread: &Entity<AgentThread>,
        id: u64,
        binding: super::composer_view::ComposerBinding,
        saved: std::sync::Arc<nocterm_ai::history::SavedPrompt>,
        cx: &mut Context<Self>,
    ) {
        let inspector = self.messages.inspector();
        let future = cx.background_executor().spawn(async move {
            saved
                .images
                .iter()
                .filter_map(|block| match block {
                    acp::ContentBlock::Image(image) => {
                        Some(decode_saved_image(&image.data).and_then(|bytes| {
                            PromptImage::validate(bytes, inspector.as_ref())
                                .map_err(ImageAttachError::from)
                        }))
                    }
                    _ => None,
                })
                .collect::<Result<Vec<_>, _>>()
        });
        let owner = thread.downgrade();
        let ticket = thread.read(cx).session_ticket();
        let completion_binding = binding.clone();
        let task = cx.spawn(async move |panel, cx| {
            let result = future.await;
            let _ = panel.update(cx, |panel, cx| {
                panel.on_queue_image_prepared(completion_binding, id, owner, ticket, result, cx)
            });
        });
        self.composer.install_edit_preparation(&binding, id, task);
    }
    fn on_queue_image_prepared(
        &mut self,
        binding: super::composer_view::ComposerBinding,
        id: u64,
        owner: gpui_kit::WeakEntity<AgentThread>,
        ticket: nocterm_ai::session::Ticket,
        result: Result<Vec<PromptImage>, ImageAttachError>,
        cx: &mut Context<Self>,
    ) {
        if !self.composer.edit_preparation_current(&binding, id)
            || !self.settings.setting::<nocterm_ai::AiSettings>(cx).enabled
        {
            return;
        }
        let _ = owner.update(cx, |thread, cx| {
            if !thread.session_request_current(ticket) {
                return;
            }
            match result {
                Ok(images) => match thread.accept_composer_images(images, cx) {
                    Ok(()) => self.composer.edit_images_accepted(&binding, id),
                    Err(error) => self.error = Some(error.to_string()),
                },
                Err(error) => self.error = Some(error.to_string()),
            }
            cx.notify();
        });
        cx.notify();
    }
    fn editing_queued(&self, id: u64) -> bool {
        self.composer.editing_id() == Some(id)
    }
    pub(super) fn remove_queued(&mut self, id: u64, cx: &mut Context<Self>) {
        let editing = self.composer.editing_id();
        if let Some(thread) = self.current() {
            thread.update(cx, |thread, cx| thread.remove_queued(id, editing, cx));
        }
    }
    pub(super) fn finish_queue_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(draft) = self.restore_queue_edit(true, cx) else {
            return;
        };
        self.reset_commands();
        self.composer.set_text(draft, window, cx);
    }
    pub(super) fn render_queue(
        &mut self,
        thread: &Entity<AgentThread>,
        labels: &super::attachments::AttachmentLabels,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if thread.read(cx).composer().queue().is_empty() {
            return None;
        }
        let count = thread.read(cx).queued_prompt_ids().count();
        let first = thread.read(cx).composer().queue()[0].saved.id;
        let renderer = QueueRenderer {
            chat: thread.entity_id(),
            open: self.composer.queue_open(),
            measured: self.composer.queue_height(thread.entity_id()),
            editing: self.composer.editing_chat() == Some(thread.entity_id()),
        };
        let height = renderer.height(window, cx);
        let header = self.queue_header(count, first, cx);
        if !renderer.open && height <= px(0.) {
            return Some(
                QueueRenderer::hood(header, cx)
                    .test_support()
                    .into_any_element(),
            );
        }
        let prompts = thread.read(cx).composer().queue().to_vec();
        let rows = prompts
            .iter()
            .map(|prompt| self.queue_row(prompt, labels, cx))
            .collect();
        Some(renderer.render(header, rows, height, cx))
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
                    .icon(if self.composer.queue_open() {
                        IconName::ChevronDown
                    } else {
                        IconName::ChevronUp
                    })
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.composer.toggle_queue();
                        cx.notify();
                    })),
            )
            .child(
                Button::new("queue-send-now")
                    .ghost()
                    .small()
                    .icon(IconName::ArrowUp)
                    .tooltip("Send now · stop current response")
                    .disabled(self.composer.editing())
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
            .child(
                Button::new("queue-remove-first")
                    .ghost()
                    .small()
                    .icon(IconName::Trash)
                    .tooltip("Remove queued message")
                    .disabled(self.editing_queued(first))
                    .on_click(cx.listener(move |this, _, _, cx| this.remove_queued(first, cx))),
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
                            .disabled(self.composer.editing())
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
                    )
                    .child(
                        Button::new(("queued-remove", id))
                            .ghost()
                            .xsmall()
                            .icon(IconName::Trash)
                            .tooltip("Remove")
                            .disabled(self.editing_queued(id))
                            .on_click(
                                cx.listener(move |this, _, _, cx| this.remove_queued(id, cx)),
                            ),
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
    fn on_queue_height_observed(
        &mut self,
        chat: EntityId,
        height: gpui_kit::Pixels,
        cx: &mut Context<Self>,
    ) {
        if self.composer.observe_queue_height(chat, height) {
            cx.notify();
        }
    }
    pub(super) fn sync_composer_draft(&mut self, cx: &mut Context<Self>) {
        let Some(thread) = self.current() else {
            return;
        };
        let text = self.composer.text(cx).to_string();
        if self.composer.draft_changed(thread.entity_id(), &text) {
            thread.update(cx, |thread, cx| thread.set_draft(text, cx));
        }
    }
}

fn decode_saved_image(data: &str) -> Result<Vec<u8>, ImageAttachError> {
    if data.len() > nocterm_ai::images::MAX_IMAGE_BYTES.div_ceil(3) * 4 {
        return Err(nocterm_ai::images::PromptImageError::TooLarge.into());
    }
    Ok(base64::engine::general_purpose::STANDARD.decode(data)?)
}

/// One immutable frame of the queue, without authoritative queued messages.
struct QueueRenderer {
    chat: EntityId,
    open: bool,
    measured: gpui_kit::Pixels,
    editing: bool,
}
impl QueueRenderer {
    fn height(&self, window: &mut Window, cx: &mut Context<AgentPanel>) -> gpui_kit::Pixels {
        transition(
            (
                SharedString::from(format!("agent-queue-{}", self.chat)),
                "height",
            ),
            if self.open {
                self.measured.min(px(240.))
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
    fn hood(header: AnyElement, cx: &gpui_kit::App) -> gpui_kit::Stateful<gpui_kit::Div> {
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
    fn rows(&self, rows: Vec<AnyElement>, cx: &mut Context<AgentPanel>) -> AnyElement {
        let owner = cx.weak_entity();
        let chat = self.chat;
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
    fn render(
        &self,
        header: AnyElement,
        rows: Vec<AnyElement>,
        height: gpui_kit::Pixels,
        cx: &mut Context<AgentPanel>,
    ) -> AnyElement {
        let target = self.measured.min(px(240.));
        let rows = self.rows(rows, cx);
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
        hood.when(self.editing, |hood| {
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
