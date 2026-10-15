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
use nocterm_ui::IconName;

use super::{
    AgentPanel,
    image_pipeline::PreviewState,
    message_actions::{MESSAGE_GROUP, escape_markdown, message_text},
    widgets::{chat_markdown, disclosure_header, entry_is_live, safe_markdown},
};

#[derive(Debug, thiserror::Error)]
enum PreviewError {
    #[error("{0}")]
    Base64(#[from] base64::DecodeError),
    #[error("{0}")]
    Codec(#[from] nocterm_ai::images::ImageCodecError),
}
pub(super) mod tool_input;
pub(super) mod tool_output;

impl AgentPanel {
    fn image_view(&self, key: (usize, usize, bool)) -> gpui_kit::AnyElement {
        EntryRenderer::image(self.messages.preview_state(&key))
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
        if let Some(binding) = self.messages.binding()
            && let Some(ticket) = self.messages.request_preview(key)
        {
            let bytes = data();
            let thumbnailer = self.messages.thumbnailer();
            let future = cx.background_executor().spawn(async move {
                let bytes = if encoded {
                    if bytes.len() > nocterm_ai::images::MAX_IMAGE_BYTES.div_ceil(3) * 4 {
                        return Err(PreviewError::Codec(
                            nocterm_ai::images::ImageCodecError::SourceSize,
                        ));
                    }
                    base64::engine::general_purpose::STANDARD.decode(bytes)?
                } else {
                    bytes
                };
                let bounds = nocterm_ai::images::PreviewBounds::new(640, 640)?;
                let preview = thumbnailer.thumbnail(bytes.into(), bounds)?;
                // Supplied previews retain the same pure size/format contract.
                let preview =
                    nocterm_ai::images::EncodedPreview::new(preview.bytes().clone(), bounds)?;
                Ok::<_, PreviewError>(Arc::new(gpui_kit::Image::from_bytes(
                    gpui_kit::ImageFormat::Png,
                    preview.bytes().to_vec(),
                )))
            });
            let task = cx.spawn(async move |this, cx| {
                let result = future.await;
                let _ = this.update(cx, |this, cx| {
                    this.on_preview_completed(binding, ticket, row, result.ok(), cx)
                });
            });
            self.messages.install_preview_task(ticket, task);
        }
        self.image_view(key)
    }
    pub(super) fn render_entry(
        &mut self,
        index: usize,
        cx: &mut Context<Self>,
    ) -> gpui_kit::AnyElement {
        let Some(thread) = self.current() else {
            return div().into_any_element();
        };
        let state = thread.read(cx);
        let Some(entry) = state.history_entries().get(index) else {
            return div().into_any_element();
        };
        let frame = RowPresentation {
            chat: thread.entity_id(),
            binding: self.messages.binding(),
            index,
            expanded: self.messages.expanded(index),
            live: entry_is_live(
                state.history_entries(),
                index,
                state.session_generating() && state.session_accepts_updates(),
            ),
            generating: state.session_generating(),
            time: state.transcript().time(index),
            redact: self
                .settings
                .setting::<nocterm_ai::AiSettings>(cx)
                .approval
                .redact_secrets,
        };
        let (row, pending) = EntryRenderer::render(frame, entry, &self.messages, cx);
        for (key, data) in pending {
            self.cached_image_for_row(key, || data, true, Some(index), cx);
        }
        row
    }
}

struct RowPresentation {
    chat: gpui_kit::EntityId,
    binding: Option<super::message_list::ActiveListBinding>,
    index: usize,
    expanded: bool,
    live: bool,
    generating: bool,
    time: Option<u64>,
    redact: bool,
}
struct EntryRenderer;
impl EntryRenderer {
    fn image(state: Option<&PreviewState>) -> gpui_kit::AnyElement {
        match state {
            Some(PreviewState::Ready(image)) => img(image.clone())
                .max_w_full()
                .h(rems(10.))
                .into_any_element(),
            Some(PreviewState::Failed) => {
                div().child("Image could not be loaded.").into_any_element()
            }
            _ => div().child("Loading image…").into_any_element(),
        }
    }
    fn render(
        frame: RowPresentation,
        entry: &Entry,
        messages: &super::message_list::MessageList,
        cx: &Context<AgentPanel>,
    ) -> (
        gpui_kit::AnyElement,
        Vec<(super::image_pipeline::ImageKey, Vec<u8>)>,
    ) {
        let row = v_flex()
            .id(("agent-entry", frame.index))
            .w_full()
            .min_w_0()
            .gap_1()
            .px_3()
            .py_2()
            .text_sm();
        let mut pending = Vec::new();
        let row = match entry {
            Entry::User(parts) => Self::user(
                row,
                &frame,
                parts,
                messages,
                &mut pending,
                message_text(entry).unwrap_or_default(),
                cx,
            ),
            Entry::Agent(text) => Self::agent(row, &frame, text, messages, cx),
            Entry::Thought(text) => Self::thought(row, &frame, text, cx),
            Entry::Tool(call) => Self::tool(row, &frame, call, cx),
            Entry::Content(acp::ContentBlock::Image(image)) => {
                let key = Self::pending_image(image, messages, &mut pending);
                row.child(Self::image(messages.preview_state(&key)))
            }
            Entry::Content(_) => row.child("Agent supplied additional content."),
        };
        (row.test_support().into_any_element(), pending)
    }
    fn pending_image(
        image: &acp::ImageContent,
        messages: &super::message_list::MessageList,
        pending: &mut Vec<(super::image_pipeline::ImageKey, Vec<u8>)>,
    ) -> super::image_pipeline::ImageKey {
        let key = (image.data.as_ptr() as usize, image.data.len(), true);
        if !messages.has_preview(&key) {
            pending.push((key, image.data.as_bytes().to_vec()));
        }
        key
    }
    fn user(
        mut row: gpui_kit::Stateful<gpui_kit::Div>,
        frame: &RowPresentation,
        parts: &[acp::ContentBlock],
        messages: &super::message_list::MessageList,
        pending: &mut Vec<(super::image_pipeline::ImageKey, Vec<u8>)>,
        text: String,
        cx: &Context<AgentPanel>,
    ) -> gpui_kit::Stateful<gpui_kit::Div> {
        row = row.items_end().group(MESSAGE_GROUP);
        let mut bubble = v_flex()
            .id(("agent-user-bubble", frame.index))
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
                acp::ContentBlock::Text(content) => {
                    bubble = bubble.child(chat_markdown(
                        (
                            SharedString::from(format!("user-text-{}", frame.index)),
                            part_index,
                        ),
                        escape_markdown(&content.text),
                    ));
                }
                acp::ContentBlock::Image(image) => {
                    let key = Self::pending_image(image, messages, pending);
                    has_images = true;
                    images = images.child(Self::image(messages.preview_state(&key)));
                }
                _ => {}
            }
        }
        if has_images {
            bubble = bubble.child(images);
        }
        row.child(bubble).child(
            super::message_actions::MessageRowActions::new(
                frame.chat,
                frame.index,
                text,
                frame.time,
                true,
            )
            .render(cx),
        )
    }
    fn agent(
        mut row: gpui_kit::Stateful<gpui_kit::Div>,
        frame: &RowPresentation,
        text: &str,
        messages: &super::message_list::MessageList,
        cx: &Context<AgentPanel>,
    ) -> gpui_kit::Stateful<gpui_kit::Div> {
        let revealed = messages.revealed_text(frame.index, text);
        row = row.group(MESSAGE_GROUP).child(
            chat_markdown(("agent-message", frame.index), safe_markdown(&revealed))
                .stream_fade(frame.generating || messages.revealing()),
        );
        if !frame.live {
            row = row.child(
                super::message_actions::MessageRowActions::new(
                    frame.chat,
                    frame.index,
                    text.to_owned(),
                    frame.time,
                    false,
                )
                .render(cx),
            );
        }
        row
    }
    fn thought(
        mut row: gpui_kit::Stateful<gpui_kit::Div>,
        frame: &RowPresentation,
        text: &str,
        cx: &Context<AgentPanel>,
    ) -> gpui_kit::Stateful<gpui_kit::Div> {
        let index = frame.index;
        let binding = frame.binding.clone();
        let label = if frame.live {
            text.lines()
                .find(|line| !line.trim().is_empty())
                .unwrap_or("Thinking…")
                .chars()
                .take(100)
                .collect()
        } else {
            "Reasoning".into()
        };
        row = row.text_color(cx.theme().muted_foreground).child(
            disclosure_header(
                ("thought", index),
                (
                    SharedString::from(format!("live-thought-{}", frame.chat)),
                    index,
                ),
                IconName::Brain,
                label,
                frame.live,
                frame.expanded,
                cx,
            )
            .on_click(cx.listener(move |panel, _, _, cx| {
                panel.on_message_expanded(binding.clone(), index, cx)
            })),
        );
        if frame.expanded {
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
        row
    }
    fn tool(
        mut row: gpui_kit::Stateful<gpui_kit::Div>,
        frame: &RowPresentation,
        call: &acp::ToolCall,
        cx: &Context<AgentPanel>,
    ) -> gpui_kit::Stateful<gpui_kit::Div> {
        let index = frame.index;
        let binding = frame.binding.clone();
        let title = nocterm_ai::tool_display::header(call);
        let title = if frame.redact {
            nocterm_ai::redact::redact(&title)
        } else {
            title
        };
        row = row.text_color(cx.theme().muted_foreground).child(
            disclosure_header(
                ("tool-call", index),
                (
                    SharedString::from(format!("live-tool-{}", frame.chat)),
                    index,
                ),
                IconName::Wrench,
                title,
                frame.live,
                frame.expanded,
                cx,
            )
            .on_click(cx.listener(move |panel, _, _, cx| {
                panel.on_message_expanded(binding.clone(), index, cx)
            })),
        );
        if frame.expanded {
            if let Some(input) = tool_input::source(call) {
                let input = if frame.redact { input.redact() } else { input };
                row = row.child(tool_input::render(index, input, cx));
            }
            if let Some(output) = tool_output::source(call, frame.redact) {
                row = row.child(tool_output::render(index, output, cx));
            }
        }
        row
    }
}
