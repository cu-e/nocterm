//! A queued Cloud run of the person's own prompt waits for permission to use the model
//! (ADR 0129).
use super::*;
use nocterm_cloud_conversation_model::{
    CommandKey, ConversationAction as CloudAction, RunKey, RunStatus,
};

#[gpui_kit::test]
fn allow_button_grants_the_waiting_run_once_and_disappears(cx: &mut TestAppContext) {
    let f = fixture(cx);
    new_chat(&f, cx);
    let (source, _selector) = super::cloud_delivery::captured_panel(&f, cx);
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("agent-allow-run").is_none());
        let focus = f.panel.read(cx).composer_focus(cx);
        replace_composer_value(focus, "Cloud question", window, cx);
        window.render_frame(cx);
        window.click("agent-send", cx);
    })
    .unwrap();
    cx.run_until_parked();
    let run = RunKey::from_bytes([1; 16]);
    source.accept_prompt(CommandKey::from_bytes([1; 16]), run, RunStatus::NeedsGrant);
    cx.run_until_parked();
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("agent-allow-run", cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(source.delivered().last(), Some(&CloudAction::Allow { run }));
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("agent-allow-run").is_none());
    })
    .unwrap();
}
