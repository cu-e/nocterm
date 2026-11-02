//! A queued Cloud run waits for the person's permission to use the model (ADR 0129).
use super::*;

#[gpui_kit::test]
fn allow_button_grants_the_waiting_run_once_and_disappears(cx: &mut TestAppContext) {
    use nocterm_cloud_model::{
        identity::CommandId,
        projection::{
            ConversationIntent,
            testing::{prompt_event, run_binding},
        },
    };
    let f = fixture(cx);
    new_chat(&f, cx);
    let (source, mut projection, _selector) = captured_panel(&f, cx);
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("agent-allow-run").is_none());
    })
    .unwrap();
    // A run queued by another device waits for permission.
    let run = run_binding(1);
    source
        .queue(projection.page(vec![prompt_event(
            &run,
            "Cloud question",
            CommandId::from_bytes([22; 16]),
        )]))
        .unwrap();
    cx.run_until_parked();
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("agent-allow-run", cx);
    })
    .unwrap();
    cx.run_until_parked();
    let effects = source.effects();
    assert_eq!(effects.delivered.len(), 1);
    let grant = source.staged(effects.delivered[0]).unwrap();
    assert!(matches!(grant.action, ConversationIntent::GrantInference));
    assert_eq!(grant.coordinates.run, Some(run));
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("agent-allow-run").is_none());
    })
    .unwrap();
}
