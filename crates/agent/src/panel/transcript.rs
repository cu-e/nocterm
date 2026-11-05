//! An open chat: the transcript, the plan, pending approvals, the status
//! line and the composer.
use gpui_kit::{
    AnyElement, SharedString, TestSupportExt as _,
    component::{ActiveTheme as _, h_flex, v_flex},
    div,
    prelude::*,
};

use super::widgets::activity_text;

pub(super) struct TranscriptView;
impl TranscriptView {
    pub(super) fn chat_status(
        thread: &gpui_kit::Entity<crate::thread::AgentThread>,
        cx: &gpui_kit::App,
    ) -> Option<AnyElement> {
        let state = thread.read(cx);
        let visible = state.session_generating()
            || state.session().is_none()
            || state.session_sign_in_required()
            || state.document().pending_history()
            || state.has_status_error()
            || !state.session_accepts_updates();
        visible.then(|| {
            TranscriptView::status(
                thread.entity_id(),
                state.status(),
                state.session_generating() && state.session_accepts_updates(),
                cx,
            )
        })
    }
    /// The status line of a conversation, as a chat shows its own: hidden while nothing
    /// happens, and live (the thinking indicator and a shimmering label) while the person waits
    /// for the agent: a prompt is on its way, a run starts or runs, or the source reconnects.
    pub(super) fn conversation_status(
        view: &nocterm_ai::conversation::ConversationView<'_>,
    ) -> Option<(String, bool)> {
        use nocterm_ai::conversation::ConversationFreshness;
        use nocterm_ai::conversation::ConversationPhase::*;
        let phase = match view.phase {
            Idle => "Ready",
            Queued => "Queued",
            Running => "Running",
            WaitingApproval => "Waiting for approval",
            NeedsGrant => "Waiting for permission",
            NeedsUnlock => "Unlock the session to continue",
            NeedsProviderAccess => "Provider access required",
            OutcomeUnknown => "Tool outcome unknown",
            Interrupted => "Interrupted",
            Stopped => "Stopped",
            Completed => "Completed",
            Deleted => "Deleted",
        };
        // The prompts on their way are shown as rows; the line says that the person waits.
        let waiting = matches!(view.phase, Idle | Completed)
            && view
                .pending_prompts
                .iter()
                .any(|prompt| prompt.state.on_its_way());
        // A Cloud chat whose runner did not confirm what it shows says so (ADR 0144).
        let notice = match view.freshness {
            ConversationFreshness::Live => None,
            ConversationFreshness::Unconfirmed { can_wake: true } => {
                Some("Seat asleep — stored history, may be incomplete")
            }
            ConversationFreshness::Unconfirmed { can_wake: false } => {
                Some("Runner not answering — stored history, may be incomplete")
            }
            ConversationFreshness::Withheld => Some("The hub withholds newer history of this chat"),
        };
        let quiet = matches!(view.phase, Idle | Completed) && !waiting && !view.reconnecting;
        if quiet && notice.is_none() {
            return None;
        }
        let mut text = if view.reconnecting {
            "Reconnecting to the agent".to_owned()
        } else if waiting {
            "Waiting for the agent".to_owned()
        } else if quiet {
            String::new()
        } else {
            phase.to_owned()
        };
        if let Some(notice) = notice {
            if !text.is_empty() {
                text.push_str(" · ");
            }
            text.push_str(notice);
        }
        if let Some(tokens) = &view.transcript.tokens {
            text.push_str(&format!(" · {} tokens", tokens.total));
        }
        let live = view.accepts_updates
            && (view.generating || matches!(view.phase, Queued) || view.reconnecting || waiting);
        Some((text, live))
    }
    /// Wakes the sleeping seat of the shown Cloud chat; shown while its history is unconfirmed.
    pub(super) fn wake_button(
        origin: gpui_kit::EntityId,
        cx: &mut gpui_kit::Context<super::AgentPanel>,
    ) -> AnyElement {
        use gpui_kit::component::{Sizable as _, button::Button};
        h_flex()
            .px_3()
            .pb_2()
            .child(
                Button::new("agent-wake")
                    .small()
                    .label("Wake")
                    .accessibility_label("Wake the agent's seat and read the chat again")
                    .tooltip("Wake the agent's seat and read the chat again")
                    .on_click(cx.listener(move |panel, _, _, cx| {
                        panel.wake_conversation(origin, cx);
                    })),
            )
            .into_any_element()
    }
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
    pub(super) fn plan(plan: &nocterm_ai::model::plan::Plan, cx: &gpui_kit::App) -> AnyElement {
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
        h_flex()
            .id("agent-status")
            .test_support()
            .gap_1p5()
            .px_3()
            .pb_2()
            .text_xs()
            .text_color(cx.theme().muted_foreground)
            .when(live, |row| {
                row.child(div().id("agent-thinking").test_support().child(
                    nocterm_ui::thinking_indicator(
                        SharedString::from(format!("agent-thinking-{id}")),
                        cx.theme().foreground,
                    ),
                ))
            })
            .child(activity_text(
                SharedString::from(format!("agent-live-status-{id}")),
                text,
                live,
                cx,
            ))
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::TranscriptView;
    use nocterm_ai::conversation::{
        ComposerCapabilities, ConversationFreshness, ConversationPhase, ConversationView,
    };

    fn status(phase: ConversationPhase, freshness: ConversationFreshness) -> Option<String> {
        let thread = nocterm_ai::thread::ThreadState::default();
        let view = ConversationView {
            transcript: &thread,
            phase,
            generating: false,
            accepts_updates: true,
            can_fork: false,
            can_prompt: true,
            can_stop: false,
            can_allow: false,
            updated_at: 0,
            pending_commands: 0,
            reconnecting: false,
            freshness,
            composer: ComposerCapabilities::default(),
            pending_prompts: &[],
        };
        TranscriptView::conversation_status(&view).map(|(text, _)| text)
    }

    #[test]
    fn an_unconfirmed_history_says_so_even_when_nothing_runs() {
        use ConversationFreshness::*;
        assert_eq!(status(ConversationPhase::Idle, Live), None);
        let asleep = status(ConversationPhase::Idle, Unconfirmed { can_wake: true }).unwrap();
        assert!(asleep.starts_with("Seat asleep"), "{asleep}");
        let silent = status(ConversationPhase::Idle, Unconfirmed { can_wake: false }).unwrap();
        assert!(silent.starts_with("Runner not answering"), "{silent}");
        let running = status(ConversationPhase::Running, Withheld).unwrap();
        assert_eq!(
            running,
            "Running · The hub withholds newer history of this chat"
        );
    }
}
