//! The chat transcript: messages, reasoning, tool calls and images.

use gpui_kit::{
    Context, SharedString, TestSupportExt as _,
    component::{ActiveTheme as _, h_flex, v_flex},
    div, img,
    prelude::*,
    px, relative, rems,
};
use nocterm_ai::{
    history::physical::{ContentSlot, MediaContent, RowBody, RowSlot, ToolSlot, UserSlot},
    model::content::MediaKind,
};
use nocterm_ui::IconName;

use super::{
    AgentPanel,
    image_pipeline::PreviewState,
    message_actions::{MESSAGE_GROUP, escape_markdown, message_text},
    widgets::{chat_markdown, disclosure_header, safe_markdown},
};

#[derive(Debug, thiserror::Error)]
pub(super) enum PreviewError {
    #[error("{0}")]
    Base64(#[from] base64::DecodeError),
    #[error("{0}")]
    Codec(#[from] nocterm_ai::images::ImageCodecError),
}
pub(super) mod tool_input;
pub(super) mod tool_output;

pub(super) struct RowPresentation {
    chat: gpui_kit::EntityId,
    binding: Option<super::message_list::ActiveListBinding>,
    index: usize,
    expanded: bool,
    live: bool,
    generating: bool,
    time: Option<u64>,
    redact: bool,
}
pub(super) struct EntryRenderer;
impl EntryRenderer {
    pub(super) fn image(state: Option<&PreviewState>) -> gpui_kit::AnyElement {
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
    pub(super) fn render(
        frame: RowPresentation,
        entry: &RowSlot,
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
        let row = match entry.view() {
            RowBody::User(parts) => Self::user(
                row,
                &frame,
                parts,
                messages,
                &mut pending,
                message_text(entry).unwrap_or_default(),
                cx,
            ),
            RowBody::Agent(text) => Self::agent(row, &frame, &text.read(), messages, cx),
            RowBody::Thought(text) => Self::thought(row, &frame, &text.read(), cx),
            RowBody::Tool(call) => Self::tool(row, &frame, call, cx),
            RowBody::Content(ContentSlot::Media(image)) if image.kind() == MediaKind::Image => {
                let key = Self::pending_image(image, messages, &mut pending);
                row.child(Self::image(messages.preview_state(&key)))
            }
            RowBody::Content(_) => row.child("Agent supplied additional content."),
        };
        (row.test_support().into_any_element(), pending)
    }
    fn pending_image(
        image: &MediaContent,
        messages: &super::message_list::MessageList,
        pending: &mut Vec<(super::image_pipeline::ImageKey, Vec<u8>)>,
    ) -> super::image_pipeline::ImageKey {
        let encoded = image.encoded();
        let key = (encoded.as_ptr() as usize, encoded.len(), true);
        if !messages.has_preview(&key) {
            pending.push((key, encoded.as_bytes().to_vec()));
        }
        key
    }
    fn user(
        mut row: gpui_kit::Stateful<gpui_kit::Div>,
        frame: &RowPresentation,
        parts: &UserSlot,
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
        for (part_index, part) in parts.contents().enumerate() {
            match part {
                ContentSlot::Text(content) => {
                    bubble = bubble.child(chat_markdown(
                        (
                            SharedString::from(format!("user-text-{}", frame.index)),
                            part_index,
                        ),
                        escape_markdown(&content.text()),
                    ));
                }
                ContentSlot::Media(image) if image.kind() == MediaKind::Image => {
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
        call: &ToolSlot,
        cx: &Context<AgentPanel>,
    ) -> gpui_kit::Stateful<gpui_kit::Div> {
        let index = frame.index;
        let binding = frame.binding.clone();
        let title = nocterm_ai::tool_display::managed_header(call);
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
            if let Some(input) = tool_input::source_managed(call) {
                let input = if frame.redact { input.redact() } else { input };
                row = row.child(tool_input::render(index, input, cx));
            }
            if let Some(output) = tool_output::source_managed(call, frame.redact) {
                row = row.child(tool_output::render(index, output, cx));
            }
        }
        row
    }
}

impl RowPresentation {
    pub(super) fn new(
        chat: gpui_kit::EntityId,
        binding: Option<super::message_list::ActiveListBinding>,
        index: usize,
        expanded: bool,
        live: bool,
        generating: bool,
        time: Option<u64>,
        redact: bool,
    ) -> Self {
        Self {
            chat,
            binding,
            index,
            expanded,
            live,
            generating,
            time,
            redact,
        }
    }
}
