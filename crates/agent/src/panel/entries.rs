//! The chat transcript: messages, reasoning, tool calls and images.
use std::sync::Arc;

use base64::Engine as _;
use gpui_kit::{
    Context, SharedString, TestSupportExt as _,
    component::{ActiveTheme as _, h_flex, v_flex},
    div, img,
    prelude::*,
    px, relative, rems,
};
use nocterm_ai::{acp, thread::Entry};
use nocterm_ui::ActiveSettings as _;
use nocterm_ui::IconName;

use super::{
    AgentPanel, CachedImage,
    message_actions::{MESSAGE_GROUP, escape_markdown, message_text},
    widgets::{chat_markdown, disclosure_header, entry_is_live, safe_markdown},
};

pub(super) mod tool_input;
pub(super) mod tool_output;

impl AgentPanel {
    fn image_view(&self, key: (usize, usize, bool)) -> gpui_kit::AnyElement {
        match self.image_cache.get(&key) {
            Some(CachedImage::Ready(image)) => img(image.clone())
                .max_w_full()
                .h(rems(10.))
                .into_any_element(),
            Some(CachedImage::Failed) => {
                div().child("Image could not be loaded.").into_any_element()
            }
            _ => div().child("Loading image…").into_any_element(),
        }
    }
    pub(super) fn cached_image(
        &mut self,
        key: (usize, usize, bool),
        data: impl FnOnce() -> Vec<u8>,
        encoded: bool,
        cx: &mut Context<Self>,
    ) -> gpui_kit::AnyElement {
        self.cached_image_for_row(key, data, encoded, None, cx)
    }
    fn cached_image_for_row(
        &mut self,
        key: (usize, usize, bool),
        data: impl FnOnce() -> Vec<u8>,
        encoded: bool,
        row: Option<usize>,
        cx: &mut Context<Self>,
    ) -> gpui_kit::AnyElement {
        if let std::collections::hash_map::Entry::Vacant(entry) = self.image_cache.entry(key) {
            entry.insert(CachedImage::Loading(None));
            let bytes = data();
            let future = cx.background_executor().spawn(async move {
                let bytes = if encoded {
                    if bytes.len() > nocterm_ai::images::MAX_IMAGE_BYTES.div_ceil(3) * 4 {
                        return Err("Image exceeds size limit".to_owned());
                    }
                    base64::engine::general_purpose::STANDARD
                        .decode(bytes)
                        .map_err(|error| error.to_string())?
                } else {
                    bytes
                };
                let image = nocterm_ai::images::PromptImage::validate(bytes)?;
                let thumbnail = image::load_from_memory(image.bytes())
                    .map_err(|error| error.to_string())?
                    .thumbnail(640, 640);
                let mut png = std::io::Cursor::new(Vec::new());
                thumbnail
                    .write_to(&mut png, image::ImageFormat::Png)
                    .map_err(|error| error.to_string())?;
                Ok::<_, String>(Arc::new(gpui_kit::Image::from_bytes(
                    gpui_kit::ImageFormat::Png,
                    png.into_inner(),
                )))
            });
            let task = cx.spawn(async move |this, cx| {
                let result = future.await;
                let _ = this.update(cx, |this, cx| {
                    if let std::collections::hash_map::Entry::Occupied(mut entry) =
                        this.image_cache.entry(key)
                    {
                        entry.insert(match result {
                            Ok(image) => CachedImage::Ready(image),
                            Err(_) => CachedImage::Failed,
                        });
                        if let Some(row) = row {
                            this.list
                                .update(cx, |list, cx| list.remeasure_items(row..row + 1, cx));
                        }
                        cx.notify();
                    }
                });
            });
            if let Some(CachedImage::Loading(pending)) = self.image_cache.get_mut(&key) {
                *pending = Some(task);
            }
        }
        self.image_view(key)
    }
    #[expect(clippy::too_many_lines, reason = "predates the limit")]
    pub(super) fn render_entry(
        &mut self,
        index: usize,
        cx: &mut Context<Self>,
    ) -> gpui_kit::AnyElement {
        let Some(thread) = self.current() else {
            return div().into_any_element();
        };
        let Some(entry) = thread.read(cx).state.entries.get(index) else {
            return div().into_any_element();
        };
        let mut pending_images = Vec::new();
        let expanded = self.expanded.contains(&index);
        let live = entry_is_live(
            &thread.read(cx).state.entries,
            index,
            thread.read(cx).generating && thread.read(cx).accept_updates,
        );
        let mut row = v_flex()
            .id(("agent-entry", index))
            .test_support()
            .w_full()
            .min_w_0()
            .gap_1()
            .px_3()
            .py_2()
            .text_sm();
        match entry {
            Entry::User(parts) => {
                row = row.items_end().group(MESSAGE_GROUP);
                let mut bubble = v_flex()
                    .id(("agent-user-bubble", index))
                    .test_support()
                    .min_w_0()
                    .max_w(relative(0.88))
                    .gap_1()
                    .px_3()
                    .py_2()
                    .rounded(px(12.))
                    .bg(cx.theme().muted);
                let mut images = h_flex().flex_wrap().gap_2();
                let mut has_images = false;
                for (part_index, part) in parts.iter().enumerate() {
                    match part {
                        acp::ContentBlock::Text(text) => {
                            bubble = bubble.child(chat_markdown(
                                (SharedString::from(format!("user-text-{index}")), part_index),
                                escape_markdown(&text.text),
                            ))
                        }
                        acp::ContentBlock::Image(image) => {
                            let key = (image.data.as_ptr() as usize, image.data.len(), true);
                            if !self.image_cache.contains_key(&key) {
                                pending_images.push((key, image.data.as_bytes().to_vec()));
                            }
                            has_images = true;
                            images = images.child(self.image_view(key))
                        }
                        _ => {}
                    }
                }
                if has_images {
                    bubble = bubble.child(images);
                }
                row = row.child(bubble);
                if let Some(text) = message_text(entry) {
                    let time = thread.read(cx).state.time(index);
                    row = row.child(self.render_message_actions(
                        thread.entity_id(),
                        index,
                        text,
                        time,
                        true,
                        cx,
                    ));
                }
            }
            Entry::Agent(text) => {
                let time = thread.read(cx).state.time(index);
                let full_text = text.clone();
                let text = self.stream.text(index, text);
                row = row.group(MESSAGE_GROUP).child(
                    chat_markdown(("agent-message", index), safe_markdown(&text))
                        .stream_fade(thread.read(cx).generating || self.stream.pending()),
                );
                if !live {
                    row = row.child(self.render_message_actions(
                        thread.entity_id(),
                        index,
                        full_text,
                        time,
                        false,
                        cx,
                    ));
                }
            }
            Entry::Thought(text) => {
                row = row.text_color(cx.theme().muted_foreground);
                row = row.child(
                    disclosure_header(
                        ("thought", index),
                        (
                            SharedString::from(format!("live-thought-{}", thread.entity_id())),
                            index,
                        ),
                        IconName::Brain,
                        if live {
                            text.lines()
                                .find(|line| !line.trim().is_empty())
                                .unwrap_or("Thinking…")
                                .chars()
                                .take(100)
                                .collect()
                        } else {
                            "Reasoning".into()
                        },
                        live,
                        expanded,
                        cx,
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if !this.expanded.insert(index) {
                            this.expanded.remove(&index);
                        }
                        this.list
                            .update(cx, |list, cx| list.remeasure_items(index..index + 1, cx));
                        cx.notify();
                    })),
                );
                if expanded {
                    row = row.child(
                        div()
                            .id(("thought-output", index))
                            .test_support()
                            .w_full()
                            .min_w_0()
                            .max_w_full()
                            .overflow_x_scroll()
                            .child(
                                chat_markdown(("thought-text", index), safe_markdown(text))
                                    .text_color(cx.theme().muted_foreground)
                                    .w_full()
                                    .max_w_full(),
                            ),
                    );
                }
            }
            Entry::Tool(call) => {
                row = row.text_color(cx.theme().muted_foreground);
                row = row.child(
                    disclosure_header(
                        ("tool-call", index),
                        (
                            SharedString::from(format!("live-tool-{}", thread.entity_id())),
                            index,
                        ),
                        IconName::Wrench,
                        {
                            let title = nocterm_ai::tool_display::header(call);
                            if cx.settings().ai.approval.redact_secrets {
                                nocterm_ai::redact::redact(&title)
                            } else {
                                title
                            }
                        },
                        live,
                        expanded,
                        cx,
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if !this.expanded.insert(index) {
                            this.expanded.remove(&index);
                        }
                        this.list
                            .update(cx, |list, cx| list.remeasure_items(index..index + 1, cx));
                        cx.notify();
                    })),
                );
                if expanded {
                    if let Some(input) = tool_input::source(call) {
                        let input = if cx.settings().ai.approval.redact_secrets {
                            input.redact()
                        } else {
                            input
                        };
                        row = row.child(tool_input::render(index, input, cx));
                    }
                    if let Some(output) =
                        tool_output::source(call, cx.settings().ai.approval.redact_secrets)
                    {
                        row = row.child(tool_output::render(index, output, cx));
                    }
                }
            }
            Entry::Content(acp::ContentBlock::Image(image)) => {
                let key = (image.data.as_ptr() as usize, image.data.len(), true);
                if !self.image_cache.contains_key(&key) {
                    pending_images.push((key, image.data.as_bytes().to_vec()));
                }
                row = row.child(self.image_view(key))
            }
            Entry::Content(_) => row = row.child("Agent supplied additional content."),
        }
        for (key, data) in pending_images {
            self.cached_image_for_row(key, || data, true, Some(index), cx);
        }
        row.into_any_element()
    }
}
