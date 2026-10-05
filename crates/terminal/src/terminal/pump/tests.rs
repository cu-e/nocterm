use super::*;
use std::collections::VecDeque;

fn queued(events: Vec<Event>) -> (Vec<Event>, Option<Pending>, VecDeque<Event>) {
    let mut events: VecDeque<_> = events.into();
    let first = Pending::new(events.pop_front().unwrap());
    let (batch, remainder) = drain(first, || events.pop_front());
    (batch, remainder, events)
}

#[test]
fn joins_adjacent_output_and_keeps_other_events_in_order() {
    let (batch, remainder, queue) = queued(vec![
        Event::Output(b"a".to_vec()),
        Event::Output(b"b".to_vec()),
        Event::Connected,
        Event::Output(b"c".to_vec()),
        Event::Output(b"d".to_vec()),
        Event::Closed(CloseReason::Exited(Some(0))),
    ]);
    assert!(matches!(batch.as_slice(),
        [Event::Output(first), Event::Connected, Event::Output(second), Event::Closed(_)]
        if first == b"ab" && second == b"cd"
    ));
    assert!(remainder.is_none());
    assert!(queue.is_empty());
}

#[test]
fn byte_budget_counts_first_and_nonadjacent_outputs() {
    let (batch, remainder, queue) = queued(vec![
        Event::Output(vec![b'a'; OUTPUT_BATCH_BYTES / 2]),
        Event::Connected,
        Event::Output(vec![b'b'; OUTPUT_BATCH_BYTES]),
        Event::Closed(CloseReason::Exited(Some(0))),
    ]);
    let bytes: usize = batch
        .iter()
        .map(|event| match event {
            Event::Output(bytes) => bytes.len(),
            _ => 0,
        })
        .sum();
    assert_eq!(bytes, OUTPUT_BATCH_BYTES);
    let pending = remainder.unwrap();
    assert_eq!(pending.offset, OUTPUT_BATCH_BYTES / 2);
    let mut queue = queue;
    let (batch, remainder) = drain(pending, || queue.pop_front());
    assert!(
        matches!(batch.as_slice(), [Event::Output(bytes), Event::Closed(_)]
        if bytes == &vec![b'b'; OUTPUT_BATCH_BYTES / 2])
    );
    assert!(remainder.is_none());
}

#[test]
fn huge_first_output_is_carried_without_reordering_or_loss() {
    let bytes: Vec<u8> = (0..OUTPUT_BATCH_BYTES * 3 + 5)
        .map(|i| (i % 251) as u8)
        .collect();
    let original = bytes.clone();
    let allocation = bytes.as_ptr();
    let mut queue = VecDeque::from([Event::Connected]);
    let mut pending = Some(Pending::new(Event::Output(bytes)));
    let mut joined = Vec::new();
    while let Some(first) = pending.take() {
        let (batch, remainder) = drain(first, || queue.pop_front());
        if let Some(Pending {
            event: Event::Output(bytes),
            ..
        }) = &remainder
        {
            assert_eq!(bytes.as_ptr(), allocation);
        }
        let mut size = 0;
        for event in batch {
            match event {
                Event::Output(bytes) => {
                    size += bytes.len();
                    joined.extend(bytes);
                }
                Event::Connected => assert_eq!(joined, original),
                _ => panic!("unexpected event"),
            }
        }
        assert!(size <= OUTPUT_BATCH_BYTES);
        pending = remainder;
    }
    assert_eq!(joined, original);
    assert!(queue.is_empty());
}

#[test]
fn control_and_empty_output_bursts_obey_event_budget() {
    for empty_output in [false, true] {
        let events = (0..BATCH_EVENTS * 2 + 1)
            .map(|_| {
                if empty_output {
                    Event::Output(Vec::new())
                } else {
                    Event::Connected
                }
            })
            .collect();
        let (batch, remainder, queue) = queued(events);
        assert_eq!(queue.len(), BATCH_EVENTS + 1);
        assert!(remainder.is_none());
        assert_eq!(batch.len(), if empty_output { 1 } else { BATCH_EVENTS });
    }
}

#[test]
fn exact_byte_boundary_leaves_control_event_queued() {
    let (_, remainder, queue) = queued(vec![
        Event::Output(vec![0; OUTPUT_BATCH_BYTES]),
        Event::Closed(CloseReason::Exited(Some(0))),
    ]);
    assert!(remainder.is_none());
    assert!(matches!(queue.front(), Some(Event::Closed(_))));
}

#[test]
fn cooperative_yield_requires_a_second_poll() {
    use futures::{FutureExt as _, task::noop_waker_ref};
    let mut future = yield_once().boxed();
    let mut cx = std::task::Context::from_waker(noop_waker_ref());
    assert!(future.as_mut().poll(&mut cx).is_pending());
    assert!(future.as_mut().poll(&mut cx).is_ready());
}

fn terminal(cx: &mut gpui_kit::TestAppContext) -> gpui_kit::Entity<Terminal> {
    use gpui_kit::AppContext as _;
    cx.update(|cx| {
        gpui_kit::init(cx);
        nocterm_ui::init(
            nocterm_ui::DesignTokens::builtin(),
            nocterm_ui::SettingsStore::in_memory(Default::default()),
            cx,
        );
        cx.new(|cx| {
            Terminal::new(
                nocterm_workspace::SessionSpec {
                    options: Default::default(),
                    title: "test".into(),
                    profile: None,
                    target: nocterm_session::Target::new("user", "host", 22),
                    auth: nocterm_session::Auth::Password,
                    launch: None,
                    credential: None,
                },
                cx,
            )
        })
    })
}

#[gpui_kit::test]
fn pump_drains_partial_output_before_channel_loss_and_preserves_explicit_close(
    cx: &mut gpui_kit::TestAppContext,
) {
    for explicit in [false, true] {
        let terminal = terminal(cx);
        let (session, driver) = nocterm_session::channel(None);
        let mut output = vec![b'x'; OUTPUT_BATCH_BYTES * 2 - 1];
        output.extend_from_slice("🙂".as_bytes());
        output.extend_from_slice(b"\x1b]2;drained\x07");
        futures::executor::block_on(async {
            assert!(driver.emit(Event::Output(output)).await);
            if explicit {
                assert!(
                    driver
                        .emit(Event::Closed(CloseReason::Exited(Some(7))))
                        .await
                );
            }
        });
        drop(driver);
        cx.update(|cx| {
            terminal.update(cx, |terminal, cx| {
                terminal.status = Status::Connected;
                terminal.spawn_pump(session, cx);
            })
        });
        cx.run_until_parked();
        cx.update(|cx| {
            let terminal = terminal.read(cx);
            assert_eq!(terminal.program_title.as_deref(), Some("drained"));
            let mut frame = nocterm_vt::Frame::default();
            terminal.emulator.snapshot(&mut frame);
            assert!(
                frame.cells.iter().any(|cell| cell.ch == '🙂'),
                "UTF-8 must survive a raw byte budget boundary"
            );
            if explicit {
                assert_eq!(
                    terminal.status,
                    Status::Closed(CloseReason::Exited(Some(7)))
                );
            } else {
                assert!(matches!(
                    terminal.status,
                    Status::Closed(CloseReason::Failed(SessionError::ConnectionLost(_)))
                ));
            }
        });
    }
}

#[gpui_kit::test]
fn obsolete_epoch_ignores_queued_output_and_channel_close(cx: &mut gpui_kit::TestAppContext) {
    let terminal = terminal(cx);
    let (session, driver) = nocterm_session::channel(None);
    futures::executor::block_on(driver.emit(Event::Output(b"\x1b]2;obsolete\x07".to_vec())));
    drop(driver);
    cx.update(|cx| {
        terminal.update(cx, |terminal, cx| {
            terminal.spawn_pump(session, cx);
            terminal.connection_epoch += 1;
            terminal.status = Status::Connected;
        })
    });
    cx.run_until_parked();
    cx.update(|cx| {
        let terminal = terminal.read(cx);
        assert!(terminal.program_title.is_none());
        assert_eq!(terminal.status, Status::Connected);
    });
}
