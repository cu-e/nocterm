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
