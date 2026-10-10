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

#[gpui_kit::test]
async fn a_session_still_closing_frees_the_slot_so_no_second_chat_is_evicted(
    cx: &mut TestAppContext,
) {
    let f = fixture(cx);
    cx.update(|cx| {
        cx.update_setting::<nocterm_ai::AiSettings>(|settings| settings.sessions.max_live = 2)
    })
    .await
    .unwrap();
    new_chat(&f, cx);
    let first = cx.update(|cx| f.panel.read(cx).current().unwrap());
    new_chat(&f, cx);
    let second = cx.update(|cx| f.panel.read(cx).current().unwrap());
    cx.run_until_parked();
    assert_eq!(f.connector.connects.load(Ordering::SeqCst), 2);

    // The evicted session takes a while to close.
    let (open, gate) = futures::channel::oneshot::channel();
    *f.commands.close_gate.lock().unwrap() = Some(gate);
    new_chat(&f, cx);
    let third = cx.update(|cx| f.panel.read(cx).current().unwrap());
    for _ in 0..5 {
        tick(cx, 1);
    }
    cx.update(|cx| {
        let leased = [&first, &second]
            .iter()
            .filter(|thread| thread.read(cx).lease.is_some())
            .count();
        assert_eq!(leased, 1, "one chat yields; the other keeps its session");
        assert!(third.read(cx).session().is_none());
    });

    open.send(()).unwrap();
    tick(cx, 1);
    cx.update(|cx| {
        assert!(third.read(cx).session().is_some());
        let leased = [&first, &second]
            .iter()
            .filter(|thread| thread.read(cx).lease.is_some())
            .count();
        assert_eq!(leased, 1);
    });
    assert_eq!(f.connector.connects.load(Ordering::SeqCst), 3);
}

#[gpui_kit::test]
fn news_of_an_earlier_connection_leaves_the_current_session_alone(cx: &mut TestAppContext) {
    let f = fixture(cx);
    new_chat(&f, cx);
    let thread = cx.update(|cx| f.panel.read(cx).current().unwrap());
    cx.run_until_parked();
    let (key, session) = cx.update(|cx| {
        let thread = thread.read(cx);
        (thread.connection_key().unwrap(), thread.session().clone())
    });
    // A lease is released asynchronously, so what its runtime connection
    // said last may reach a chat that has moved on to another connection.
    cx.update(|cx| {
        let client = crate::thread::client(&thread);
        client.emit(
            crate::runtime::SessionEvent::Stopped {
                connection: key + 1,
                message: "an earlier agent process exited".into(),
            },
            cx,
        );
        client.emit(
            crate::runtime::SessionEvent::Idle {
                connection: key + 1,
            },
            cx,
        );
    });
    cx.run_until_parked();
    cx.update(|cx| {
        let thread = thread.read(cx);
        assert_eq!(
            thread.lifecycle.phase(),
            nocterm_ai::session::SessionPhase::Ready
        );
        assert_eq!(thread.session(), &session);
        assert!(!thread.status_error);
    });
    exchange(&f, "still there?", "Yes.", cx);
}
