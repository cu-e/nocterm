//! GPUI input ownership must survive presentation optimizations.
use super::*;
use nocterm_workspace::LiveCommandLease;

fn claim(
    cx: &mut TestAppContext,
    view: &Entity<TerminalView>,
    transport: &Scripted,
) -> LiveCommandLease {
    emit(cx, transport, 0, Event::Output(b"\x1b]133;A\x07".to_vec()));
    let lease = cx.update(|cx| {
        view.update(cx, |view, cx| {
            view.terminal
                .update(cx, |terminal, cx| {
                    terminal.begin_live_command("sleep 10", cx)
                })
                .unwrap()
        })
    });
    cx.run_until_parked();
    assert_eq!(
        drain(&scripted_driver(transport, 0)),
        [b"sleep 10\r".to_vec()]
    );
    assert!(lease.is_active());
    lease
}

fn watch(
    cx: &mut TestAppContext,
    view: &Entity<TerminalView>,
) -> (Rc<Cell<usize>>, Rc<Cell<usize>>, Vec<Subscription>) {
    let notifications = Rc::new(Cell::new(0));
    let outputs = Rc::new(Cell::new(0));
    let subscriptions = cx.update(|cx| {
        let notifications = notifications.clone();
        let outputs = outputs.clone();
        let terminal = view.read(cx).terminal.clone();
        vec![
            cx.observe(view, move |_, _| notifications.set(notifications.get() + 1)),
            cx.subscribe(&terminal, move |_, event, _| {
                if *event == TerminalEvent::Output {
                    outputs.set(outputs.get() + 1);
                }
            }),
        ]
    });
    (notifications, outputs, subscriptions)
}

#[gpui_kit::test]
fn accepted_key_revokes_live_ownership_without_echo_or_redraw(cx: &mut TestAppContext) {
    let (handle, view, transport) = fixture(cx);
    let lease = claim(cx, &view, &transport);
    let driver = scripted_driver(&transport, 0);
    let (notifications, outputs, _subscriptions) = watch(cx, &view);
    assert!(view.read_with(cx, |view, _| view.cursor_lit));
    cx.update_window(handle, |_, window, cx| window.press("backspace", cx))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(drain(&driver), [b"\x7f".to_vec()]);
    assert!(!lease.is_active());
    assert_eq!(outputs.get(), 0, "input must wait for transport echo");
    assert_eq!(
        notifications.get(),
        0,
        "an already lit cursor needs no paint"
    );
    lease.cancel();
    assert!(drain(&driver).is_empty(), "stale Stop must not send Ctrl-C");
}

#[gpui_kit::test]
fn ctrl_c_dispatch_revokes_live_ownership_only_when_input_is_accepted(cx: &mut TestAppContext) {
    let (handle, view, transport) = fixture(cx);
    let driver = scripted_driver(&transport, 0);
    let accepted = claim(cx, &view, &transport);
    cx.update_window(handle, |_, window, cx| window.press("ctrl-c", cx))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(drain(&driver), [vec![3]]);
    assert!(
        !accepted.is_active(),
        "accepted interrupt must revoke AI command ownership"
    );
    accepted.cancel();
    assert!(
        drain(&driver).is_empty(),
        "stale Stop emitted a second interrupt"
    );

    let rejected = claim(cx, &view, &transport);
    let queued = cx.update(|cx| {
        let terminal = view.read(cx).terminal.read(cx);
        let mut queued = 0;
        while terminal.send_protocol(b"queued".to_vec()) {
            queued += 1;
            assert!(queued <= 256);
        }
        queued
    });
    cx.update_window(handle, |_, window, cx| window.press("ctrl-c", cx))
        .unwrap();
    cx.run_until_parked();
    assert!(
        rejected.is_active(),
        "rejected interrupt cannot revoke AI command ownership"
    );
    assert_eq!(drain(&driver), vec![b"queued".to_vec(); queued]);
    rejected.cancel();
    assert_eq!(drain(&driver), [vec![3]], "owned Stop must still interrupt");
}

#[gpui_kit::test]
fn ime_composition_keeps_live_ownership_but_committed_text_revokes_it(cx: &mut TestAppContext) {
    let (handle, view, transport) = fixture(cx);
    let lease = claim(cx, &view, &transport);
    let driver = scripted_driver(&transport, 0);
    let (_, outputs, _subscriptions) = watch(cx, &view);
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |view, cx| {
            view.replace_and_mark_text_in_range(None, "界", None, window, cx)
        });
    })
    .unwrap();
    cx.run_until_parked();
    assert!(lease.is_active(), "composition has not sent user input");
    assert!(drain(&driver).is_empty());
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |view, cx| {
            view.replace_text_in_range(None, "界e\u{301}", window, cx)
        });
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(drain(&driver), ["界e\u{301}".as_bytes().to_vec()]);
    assert!(!lease.is_active());
    assert_eq!(outputs.get(), 0, "the IME overlay is not terminal echo");
    lease.cancel();
    assert!(drain(&driver).is_empty(), "stale Stop must not send Ctrl-C");
}

#[gpui_kit::test]
fn rejected_key_keeps_live_ownership_until_an_explicit_stop(cx: &mut TestAppContext) {
    let (handle, view, transport) = fixture(cx);
    let lease = claim(cx, &view, &transport);
    let driver = scripted_driver(&transport, 0);
    let queued = cx.update(|cx| {
        let terminal = view.read(cx).terminal.read(cx);
        let mut queued = 0;
        while terminal.send_protocol(b"queued".to_vec()) {
            queued += 1;
            assert!(queued <= 256, "the input channel must be bounded");
        }
        queued
    });
    assert!(queued > 0);
    assert!(
        lease.is_active(),
        "protocol traffic does not revoke user ownership"
    );
    cx.update_window(handle, |_, window, cx| window.press("backspace", cx))
        .unwrap();
    cx.run_until_parked();
    assert!(
        lease.is_active(),
        "rejected input cannot revoke an owned command"
    );
    assert!(view.read_with(cx, |view, cx| {
        view.terminal.read(cx).text_error().is_some()
    }));
    assert_eq!(drain(&driver), vec![b"queued".to_vec(); queued]);
    lease.cancel();
    assert!(!lease.is_active());
    assert_eq!(drain(&driver), [vec![3]], "owned Stop still sends Ctrl-C");
}
