//! The chat a panel shows keeps its agent session while idle.
use super::*;
use std::time::Duration;

fn tick(cx: &mut TestAppContext, seconds: u64) {
    cx.background_executor
        .advance_clock(Duration::from_secs(seconds));
    cx.run_until_parked();
}

#[gpui_kit::test]
async fn the_shown_chat_outlives_the_idle_timeout_until_the_user_moves_on(cx: &mut TestAppContext) {
    let f = fixture(cx);
    cx.update(|cx| {
        cx.update_setting::<nocterm_ai::AiSettings>(|settings| {
            settings.sessions.idle_timeout_secs = 2
        })
    })
    .await
    .unwrap();
    new_chat(&f, cx);
    exchange(&f, "check the disk", "Use df -h.", cx);
    let first = cx.update(|cx| f.panel.read(cx).current().unwrap());
    tick(cx, 10);
    cx.update(|cx| {
        assert!(first.read(cx).shown);
        assert!(first.read(cx).lease.is_some(), "the shown chat stays warm");
    });

    new_chat(&f, cx);
    cx.update(|cx| assert!(!first.read(cx).shown));
    tick(cx, 1);
    tick(cx, 3);
    cx.update(|cx| {
        assert!(first.read(cx).lease.is_none());
        assert_eq!(first.read(cx).state.entries.len(), 2);
    });
}
