//! A Cloud chat in the one composer (ADR 0134): its rows, approvals and controls, pending
//! prompts, refusals and unanswered deliveries; the composer never sticks.
use super::*;
use nocterm_cloud_conversation_model::testing::{FakeConversation, row, run, tool_row};
use nocterm_cloud_conversation_model::{
    Approval, CommandKey, CommandState, ConversationAction as CloudAction, Freshness, IntentKey,
    RowRole, RunKey, RunStatus, ToolState,
};

pub(in crate::panel::tests) fn captured_panel(
    f: &Fixture,
    cx: &mut TestAppContext,
) -> (FakeConversation, crate::ConversationPanel) {
    let source = FakeConversation::new("runner-chat-1");
    let facade = cx.update(|cx| {
        f.installation
            .installed
            .cloud_chats()
            .open(Arc::new(source.clone()), cx)
            .unwrap()
    });
    let selector = crate::ConversationPanel::new(f.panel.downgrade(), f.handle);
    cx.update_window(f.handle, |_, window, cx| {
        selector
            .open(facade, "Cloud task".into(), window, cx)
            .unwrap();
    })
    .unwrap();
    cx.run_until_parked();
    (source, selector)
}

fn send(f: &Fixture, text: &str, cx: &mut TestAppContext) {
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        let focus = f.panel.read(cx).composer_focus(cx);
        replace_composer_value(focus, text, window, cx);
        window.render_frame(cx);
        window.click("agent-send", cx);
    })
    .unwrap();
    cx.run_until_parked();
}

fn press(f: &Fixture, id: impl Into<gpui_kit::ElementId>, cx: &mut TestAppContext) {
    let id = id.into();
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        window.click(id, cx);
    })
    .unwrap();
    cx.run_until_parked();
}

fn shown(f: &Fixture, id: impl Into<gpui_kit::ElementId>, cx: &mut TestAppContext) -> bool {
    let id = id.into();
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        window.try_find(id).is_some()
    })
    .unwrap()
}

const RUN: RunKey = RunKey::from_bytes([3; 16]);
const FIRST: CommandKey = CommandKey::from_bytes([1; 16]);

fn prompts(text: &str) -> CloudAction {
    CloudAction::Prompt { text: text.into() }
}

#[gpui_kit::test]
fn a_cloud_chat_shows_its_rows_answers_an_approval_and_stops_its_run(cx: &mut TestAppContext) {
    let f = fixture(cx);
    let (source, _selector) = captured_panel(&f, cx);
    let intent = IntentKey::from_bytes([7; 16]);
    let display = nocterm_ai::tool_display::ToolDisplay::new(
        &nocterm_ai::TerminalCall::ListTerminals,
        Some("original target".into()),
    );
    let mut tool = tool_row(
        RUN,
        intent,
        &display.tool,
        ToolState::AwaitingApproval {
            expires_at_ms: 20_000,
        },
    );
    if let Some(call) = tool.tool.as_mut() {
        call.arguments_json = display.arguments.to_string();
    }
    source.set_runs(vec![run(
        RUN,
        RunStatus::WaitingApproval,
        CommandKey::from_bytes([22; 16]),
    )]);
    source.push_rows(vec![row(RowRole::User, Some(RUN), "Cloud question"), tool]);
    source.set_approvals(vec![Approval {
        intent,
        run: RUN,
        expires_at_ms: 20_000,
    }]);
    cx.run_until_parked();
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        window.hover(("agent-entry", 0usize), cx);
        window.click(("copy-message", 0usize), cx);
        assert_eq!(
            cx.read_from_clipboard().unwrap().text().as_deref(),
            Some("Cloud question")
        );
        assert!(window.try_find(("fork-message", 0usize)).is_none());
        assert!(window.try_find("agent-attach-context").is_none());
        let approvals = window.find("agent-approvals").bounds();
        assert!(approvals.bottom() <= window.find("agent-composer").bounds().origin.y);
        window.click(("tool-Allow once", 0usize), cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(
        source.delivered(),
        [CloudAction::Answer {
            intent,
            approve: true
        }]
    );
    press(&f, "agent-send", cx);
    assert_eq!(
        source.delivered().last(),
        Some(&CloudAction::Stop { run: Some(RUN) }),
        "an empty composer stops the shown run"
    );
}

#[gpui_kit::test]
fn an_obsolete_rendered_stop_never_stops_the_run_that_replaced_it(cx: &mut TestAppContext) {
    let f = fixture(cx);
    let (source, _selector) = captured_panel(&f, cx);
    let a = RunKey::from_bytes([1; 16]);
    source.set_runs(vec![run(
        a,
        RunStatus::Running,
        CommandKey::from_bytes([44; 16]),
    )]);
    source.push_rows(vec![row(RowRole::User, Some(a), "Cloud A")]);
    cx.run_until_parked();
    let rendered = cx
        .update_window(f.handle, |_, window, cx| {
            window.render_frame(cx);
            f.panel.read(cx).capture_rendered_stop(cx).unwrap()
        })
        .unwrap();
    let b = RunKey::from_bytes([2; 16]);
    source.set_runs(vec![
        run(a, RunStatus::Completed, CommandKey::from_bytes([44; 16])),
        run(b, RunStatus::Running, CommandKey::from_bytes([45; 16])),
    ]);
    source.push_rows(vec![row(RowRole::User, Some(b), "Cloud B")]);
    cx.run_until_parked();
    f.panel.update(cx, |panel, cx| rendered.activate(panel, cx));
    cx.run_until_parked();
    assert!(
        source.delivered().is_empty(),
        "the old A callback must never stop B"
    );
    press(&f, "agent-send", cx);
    assert_eq!(source.delivered(), [CloudAction::Stop { run: Some(b) }]);
}

#[gpui_kit::test]
fn a_sent_prompt_shows_at_once_as_pending_until_its_accepted_row_replaces_it(
    cx: &mut TestAppContext,
) {
    let f = fixture(cx);
    let (source, _selector) = captured_panel(&f, cx);
    send(&f, "hello", cx);
    assert!(shown(&f, ("agent-user-bubble", 0usize), cx));
    assert!(
        shown(&f, ("agent-pending-prompt", 0usize), cx),
        "marked as on its way"
    );
    assert!(
        shown(&f, "agent-thinking", cx),
        "the person waits for the agent"
    );
    assert!(!shown(&f, ("agent-pending-retry", 0usize), cx));
    // Sending again while the prompt is on its way sends nothing new.
    press(&f, "agent-send", cx);
    assert_eq!(source.delivered(), [prompts("hello")]);
    source.settle(FIRST, CommandState::Delivered);
    cx.run_until_parked();
    assert!(shown(&f, ("agent-pending-prompt", 0usize), cx));
    cx.update(|cx| assert!(f.panel.read(cx).composer_text(cx).is_empty()));
    source.accept_prompt(FIRST, RUN, RunStatus::Queued);
    cx.run_until_parked();
    assert!(shown(&f, ("agent-user-bubble", 0usize), cx));
    assert!(!shown(&f, ("agent-pending-prompt", 0usize), cx), "accepted");
    assert!(!shown(&f, ("agent-user-bubble", 1usize), cx), "shown once");
}

#[gpui_kit::test]
fn a_refused_prompt_sent_again_is_a_new_prompt_not_a_stuck_retry(cx: &mut TestAppContext) {
    let f = fixture(cx);
    let (source, _selector) = captured_panel(&f, cx);
    source.settle_deliveries(Some(CommandState::Refused));
    send(&f, "приве", cx);
    assert!(shown(&f, "agent-error", cx), "the refusal is shown");
    cx.update(|cx| assert_eq!(f.panel.read(cx).composer_text(cx).as_ref(), "приве"));
    source.settle_deliveries(Some(CommandState::Delivered));
    press(&f, "agent-send", cx);
    assert_eq!(
        source.delivered(),
        [prompts("приве"), prompts("приве")],
        "the same text is a new prompt"
    );
    assert!(!shown(&f, "agent-error", cx));
    assert!(
        !shown(&f, ("agent-pending-retry", 0usize), cx),
        "it replaced the refused row"
    );
    cx.update(|cx| assert!(f.panel.read(cx).composer_text(cx).is_empty()));
}

#[gpui_kit::test]
fn a_failed_prompt_shows_retry_and_dismiss_and_its_retry_keeps_the_new_draft(
    cx: &mut TestAppContext,
) {
    let f = fixture(cx);
    let (source, _selector) = captured_panel(&f, cx);
    source.settle_deliveries(Some(CommandState::Unreachable));
    send(&f, "привет", cx);
    assert!(shown(&f, ("agent-pending-retry", 0usize), cx));
    assert!(shown(&f, ("agent-pending-dismiss", 0usize), cx));
    source.settle_deliveries(None);
    cx.update_window(f.handle, |_, window, cx| {
        let focus = f.panel.read(cx).composer_focus(cx);
        replace_composer_value(focus, "a new draft", window, cx);
        window.render_frame(cx);
        window.click(("agent-pending-retry", 0usize), cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(source.delivered(), [prompts("привет"), prompts("привет")]);
    assert!(!shown(&f, ("agent-pending-retry", 0usize), cx));
    assert!(
        shown(&f, ("agent-pending-prompt", 0usize), cx),
        "one row, on its way"
    );
    assert!(!shown(&f, ("agent-pending-prompt", 1usize), cx));
    cx.update(|cx| assert_eq!(f.panel.read(cx).composer_text(cx).as_ref(), "a new draft"));
}

#[gpui_kit::test]
fn a_sleeping_seat_says_so_and_wake_wakes_it(cx: &mut TestAppContext) {
    let f = fixture(cx);
    let (source, _selector) = captured_panel(&f, cx);
    assert!(
        !shown(&f, "agent-wake", cx),
        "nothing is claimed before a round"
    );
    source.set_freshness(Freshness::Unconfirmed {
        last_witnessed_at_ms: None,
    });
    cx.run_until_parked();
    assert!(
        shown(&f, "agent-status", cx),
        "the stored history is called unconfirmed"
    );
    assert!(shown(&f, "agent-wake", cx));
    press(&f, "agent-wake", cx);
    assert_eq!(source.wakes(), 1);
    source.set_freshness(Freshness::Current { witnessed_at_ms: 1 });
    cx.run_until_parked();
    assert!(!shown(&f, "agent-wake", cx));
    assert!(!shown(&f, "agent-status", cx), "nothing happens");
}

#[gpui_kit::test]
fn a_cloud_chat_draws_the_local_composer_with_only_the_controls_it_supports(
    cx: &mut TestAppContext,
) {
    let f = fixture(cx);
    new_chat(&f, cx);
    for local in [
        "agent-composer",
        "agent-attach-context",
        "agent-attach-image",
        "agent-send",
    ] {
        assert!(shown(&f, local, cx), "a local chat shows {local}");
    }
    let (_source, _selector) = captured_panel(&f, cx);
    assert!(shown(&f, "agent-composer", cx));
    assert!(shown(&f, "agent-send", cx));
    assert!(
        !shown(&f, "agent-attach-context", cx),
        "a Cloud prompt is text"
    );
    assert!(!shown(&f, "agent-attach-image", cx));
    assert!(!shown(&f, "agent-status", cx), "nothing happens yet");
}

#[gpui_kit::test]
fn attaching_an_image_to_a_cloud_prompt_opens_no_file_dialog(cx: &mut TestAppContext) {
    let f = fixture(cx);
    let (_source, _selector) = captured_panel(&f, cx);
    cx.update_window(f.handle, |_, window, cx| {
        let focus = f.panel.read(cx).composer_focus(cx);
        window.focus(&focus, cx);
        window.render_frame(cx);
        window.dispatch_action(Box::new(crate::AttachImage), cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert!(!cx.did_prompt_for_paths(), "a Cloud prompt is text");
}

#[gpui_kit::test]
fn a_draft_edited_while_a_prompt_is_on_its_way_stays_after_it_was_delivered(
    cx: &mut TestAppContext,
) {
    let f = fixture(cx);
    new_chat(&f, cx);
    let local = cx.update(|cx| f.panel.read(cx).current().unwrap());
    let (source, selector) = captured_panel(&f, cx);
    send(&f, "original prompt", cx);
    cx.update_window(f.handle, |_, window, cx| {
        let focus = f.panel.read(cx).composer_focus(cx);
        replace_composer_value(focus, "new unsent draft", window, cx);
    })
    .unwrap();
    source.settle(FIRST, CommandState::Delivered);
    cx.run_until_parked();
    cx.update(|cx| {
        assert_eq!(
            f.panel.read(cx).composer_text(cx).as_ref(),
            "new unsent draft"
        )
    });
    press(&f, "agent-send", cx);
    cx.update(|cx| selector.close(cx).unwrap());
    cx.run_until_parked();
    assert_eq!(
        source.delivered(),
        [prompts("original prompt"), prompts("new unsent draft")]
    );
    cx.update(|cx| {
        assert_eq!(
            f.panel.read(cx).current().unwrap().entity_id(),
            local.entity_id()
        )
    });
}
