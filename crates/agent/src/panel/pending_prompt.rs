//! A prompt the person sent that its source has not accepted yet: drawn after the transcript
//! as the same user row, marked as on its way or failed, until the accepted row replaces it.
use super::{AgentPanel, widgets::activity_text};
use gpui_kit::{
    AnyElement, Context, EntityId, SharedString, TestSupportExt as _,
    component::{
        ActiveTheme as _, Sizable as _,
        button::{Button, ButtonVariants as _},
        h_flex,
    },
    div,
    prelude::*,
};
use nocterm_ai::{
    conversation::{ConversationView, PendingPromptKey, PendingPromptState},
    model::content::Content,
    thread::Entry,
};
use nocterm_ui::IconName;

#[derive(Clone, Copy)]
pub(super) struct PendingMark {
    key: PendingPromptKey,
    state: PendingPromptState,
}
impl PendingMark {
    /// The pending prompt drawn at `row`, after the transcript's entries, as a user entry.
    pub(super) fn at(view: &ConversationView<'_>, row: usize) -> Option<(Self, Entry)> {
        let prompt = view
            .pending_prompts
            .get(row.checked_sub(view.transcript.entries.len())?)?;
        Some((
            Self {
                key: prompt.key,
                state: prompt.state,
            },
            Entry::User(vec![Content::text(prompt.text)]),
        ))
    }
    /// Whether the prompt is still on its way, so its bubble is drawn subdued.
    pub(super) fn on_its_way(&self) -> bool {
        self.state.on_its_way()
    }
    /// The line under the bubble: its progress, or why it failed with Retry and Dismiss.
    pub(super) fn render(
        &self,
        chat: EntityId,
        row: usize,
        cx: &Context<AgentPanel>,
    ) -> AnyElement {
        let text = match self.state {
            PendingPromptState::Sending => "Sending…",
            PendingPromptState::Delivered => "On its way to the agent",
            PendingPromptState::Unreachable => "Not delivered: the agent cannot be reached",
            PendingPromptState::Refused => "The agent refused this message",
        };
        let key = self.key;
        let line = h_flex()
            .id(("agent-pending-prompt", row))
            .test_support()
            .max_w_full()
            .gap_1()
            .text_xs();
        if self.on_its_way() {
            return line
                .child(activity_text(
                    SharedString::from(format!("agent-pending-{chat}-{row}")),
                    text.to_owned(),
                    true,
                    cx,
                ))
                .into_any_element();
        }
        line.w_full()
            .justify_end()
            .text_color(cx.theme().danger)
            .child(div().min_w_0().truncate().child(text))
            .child(
                Button::new(("agent-pending-retry", row))
                    .ghost()
                    .xsmall()
                    .flex_shrink_0()
                    .icon(IconName::RotateCcw)
                    .label("Retry")
                    .on_click(cx.listener(move |panel, _, window, cx| {
                        panel.retry_pending_prompt(chat, key, window, cx);
                    })),
            )
            .child(
                Button::new(("agent-pending-dismiss", row))
                    .ghost()
                    .xsmall()
                    .flex_shrink_0()
                    .icon(IconName::X)
                    .tooltip("Dismiss")
                    .on_click(cx.listener(move |panel, _, _, cx| {
                        panel.dismiss_pending_prompt(chat, key, cx);
                    })),
            )
            .into_any_element()
    }
}
