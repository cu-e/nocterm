//! Sending to a Cloud chat whose runner refuses or does not answer: the composer never sticks.
use super::*;
use nocterm_ai::conversation::ConversationAction;
use nocterm_cloud_model::projection::ProjectionError;

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

fn press_send_again(f: &Fixture, cx: &mut TestAppContext) {
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("agent-send", cx);
    })
    .unwrap();
    cx.run_until_parked();
}

fn error_shown(f: &Fixture, cx: &mut TestAppContext) -> bool {
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        window.try_find("agent-error").is_some()
    })
    .unwrap()
}

#[gpui_kit::test]
fn a_refused_prompt_sent_again_is_a_new_prompt_not_a_stuck_retry(cx: &mut TestAppContext) {
    let f = fixture(cx);
    let (source, _projection, _selector) = captured_panel(&f, cx);
    source.reject_next_command();
    send(&f, "приве", cx);
    assert!(error_shown(&f, cx), "the refusal is shown");
    assert_eq!(source.effects().staged.len(), 1);
    cx.update(|cx| {
        assert_eq!(f.panel.read(cx).composer_text(cx).as_ref(), "приве");
    });

    press_send_again(&f, cx);
    let effects = source.effects();
    assert_eq!(effects.staged.len(), 2, "the same text is a new prompt");
    assert_eq!(effects.delivered, effects.staged);
    for id in &effects.staged {
        assert_eq!(
            source.staged(*id).unwrap().action,
            ConversationAction::Prompt("приве".into())
        );
    }
    assert!(!error_shown(&f, cx));
    cx.update(|cx| assert!(f.panel.read(cx).composer_text(cx).is_empty()));
}

#[gpui_kit::test]
fn an_unanswered_prompt_is_retried_under_its_own_id_when_sent_again(cx: &mut TestAppContext) {
    let f = fixture(cx);
    let (source, _projection, _selector) = captured_panel(&f, cx);
    source.fail_next_delivery(ProjectionError::Unavailable);
    send(&f, "привет", cx);
    assert!(error_shown(&f, cx));
    let id = source.effects().staged[0];
    assert!(source.effects().delivered.is_empty());

    press_send_again(&f, cx);
    let effects = source.effects();
    assert_eq!(effects.staged, [id], "a retry keeps the original command");
    assert_eq!(effects.delivered, [id]);
    assert!(!error_shown(&f, cx));
    cx.update(|cx| assert!(f.panel.read(cx).composer_text(cx).is_empty()));
}

fn shown(f: &Fixture, id: impl Into<gpui_kit::ElementId>, cx: &mut TestAppContext) -> bool {
    let id = id.into();
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        window.try_find(id).is_some()
    })
    .unwrap()
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
    let (_source, _projection, _selector) = captured_panel(&f, cx);
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
fn a_sent_prompt_shows_at_once_as_pending_until_its_accepted_row_replaces_it(
    cx: &mut TestAppContext,
) {
    use nocterm_cloud_model::projection::testing::{prompt_event, run_binding};
    let f = fixture(cx);
    let (source, mut projection, _selector) = captured_panel(&f, cx);
    let delivery = source.pause_delivery();
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

    delivery.release();
    cx.run_until_parked();
    assert!(shown(&f, ("agent-pending-prompt", 0usize), cx));
    cx.update(|cx| assert!(f.panel.read(cx).composer_text(cx).is_empty()));
    let id = source.effects().staged[0];
    source
        .queue(projection.page(vec![prompt_event(&run_binding(1), "hello", id)]))
        .unwrap();
    cx.run_until_parked();
    assert!(shown(&f, ("agent-user-bubble", 0usize), cx));
    assert!(!shown(&f, ("agent-pending-prompt", 0usize), cx), "accepted");
    assert!(!shown(&f, ("agent-user-bubble", 1usize), cx), "shown once");
}

#[gpui_kit::test]
fn a_refused_prompt_shows_failed_and_its_retry_sends_the_text_as_a_new_prompt(
    cx: &mut TestAppContext,
) {
    let f = fixture(cx);
    let (source, _projection, _selector) = captured_panel(&f, cx);
    source.reject_next_command();
    send(&f, "приве", cx);
    assert!(shown(&f, ("agent-pending-retry", 0usize), cx));
    assert!(shown(&f, ("agent-pending-dismiss", 0usize), cx));

    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        window.click(("agent-pending-retry", 0usize), cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(source.effects().staged.len(), 2, "a new prompt");
    assert!(!shown(&f, ("agent-pending-retry", 0usize), cx));
    assert!(
        shown(&f, ("agent-pending-prompt", 0usize), cx),
        "one row, on its way"
    );
    assert!(!shown(&f, ("agent-pending-prompt", 1usize), cx));
    cx.update(|cx| assert!(f.panel.read(cx).composer_text(cx).is_empty()));
}

#[gpui_kit::test]
fn an_unanswered_prompt_retried_from_its_row_keeps_its_id_and_the_new_draft(
    cx: &mut TestAppContext,
) {
    let f = fixture(cx);
    let (source, _projection, _selector) = captured_panel(&f, cx);
    source.fail_next_delivery(ProjectionError::Unavailable);
    send(&f, "привет", cx);
    assert!(shown(&f, ("agent-pending-retry", 0usize), cx));
    assert!(!shown(&f, ("agent-pending-dismiss", 0usize), cx));
    let id = source.effects().staged[0];

    cx.update_window(f.handle, |_, window, cx| {
        let focus = f.panel.read(cx).composer_focus(cx);
        replace_composer_value(focus, "a new draft", window, cx);
        window.render_frame(cx);
        window.click(("agent-pending-retry", 0usize), cx);
    })
    .unwrap();
    cx.run_until_parked();
    let effects = source.effects();
    assert_eq!(effects.staged, [id]);
    assert_eq!(effects.delivered, [id]);
    assert!(!shown(&f, ("agent-pending-retry", 0usize), cx));
    cx.update(|cx| {
        assert_eq!(f.panel.read(cx).composer_text(cx).as_ref(), "a new draft");
    });
}

#[gpui_kit::test]
fn attaching_an_image_to_a_cloud_prompt_opens_no_file_dialog(cx: &mut TestAppContext) {
    let f = fixture(cx);
    let (_source, _projection, _selector) = captured_panel(&f, cx);
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
