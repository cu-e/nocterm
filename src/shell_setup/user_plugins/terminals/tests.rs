//! A plugin terminal's session: lines handed on, output shown, and the prompt the terminal is told.

use std::sync::{Arc, atomic::AtomicBool};

use futures::channel::mpsc;
use gpui_kit::TestAppContext;
use nocterm_session::{Event as SessionEvent, Session};

use super::{drive, line::SETTLE};

/// The prompt marker of a clean prompt (OSC 133 A).
const READY: &[u8] = b"\x1b]133;A\x1b\\";
/// The marker of a line whose answer is awaited (OSC 133 C).
const ANSWERING: &[u8] = b"\x1b]133;C\x1b\\";

/// What the session showed since the last call.
fn shown(session: &Session) -> Vec<u8> {
    let mut bytes = Vec::new();
    while let Some(event) = session.try_next_event() {
        if let SessionEvent::Output(output) = event {
            bytes.extend(output);
        }
    }
    bytes
}

#[gpui_kit::test]
fn the_console_tells_the_terminal_when_it_shows_a_clean_prompt(cx: &mut TestAppContext) {
    let (session, driver) = nocterm_session::channel(None);
    let (mut output, written) = mpsc::channel(8);
    let (typed, mut lines) = mpsc::channel(8);
    let executor = cx.executor();
    executor
        .spawn(drive(
            driver,
            written,
            typed,
            Arc::new(AtomicBool::new(true)),
            executor.clone(),
        ))
        .detach();
    cx.run_until_parked();
    assert_eq!(shown(&session), READY, "a console just opened takes a line");

    assert!(session.input(b"list\r".to_vec()));
    cx.run_until_parked();
    assert_eq!(
        shown(&session),
        [b"list\r\n".as_slice(), ANSWERING].concat()
    );
    assert_eq!(lines.try_recv().ok(), Some("list".to_owned()));

    executor.advance_clock(SETTLE / 2);
    output
        .try_send(b"There are 0 players online\r\n".to_vec())
        .unwrap();
    cx.run_until_parked();
    executor.advance_clock(SETTLE * 3 / 4);
    cx.run_until_parked();
    assert_eq!(
        shown(&session),
        b"There are 0 players online\r\n",
        "the answer is still coming"
    );

    executor.advance_clock(SETTLE / 2);
    cx.run_until_parked();
    assert_eq!(
        shown(&session),
        READY,
        "the prompt is back once the output went quiet"
    );
}

#[gpui_kit::test]
fn a_half_typed_line_is_not_a_clean_prompt_until_it_is_cleared(cx: &mut TestAppContext) {
    let (session, driver) = nocterm_session::channel(None);
    let (_output, written) = mpsc::channel::<Vec<u8>>(8);
    let (typed, _lines) = mpsc::channel(8);
    let executor = cx.executor();
    executor
        .spawn(drive(
            driver,
            written,
            typed,
            Arc::new(AtomicBool::new(true)),
            executor.clone(),
        ))
        .detach();
    cx.run_until_parked();
    shown(&session);

    assert!(session.input(b"stop\rsa".to_vec()));
    cx.run_until_parked();
    executor.advance_clock(SETTLE * 2);
    cx.run_until_parked();
    assert_eq!(
        shown(&session),
        [b"stop\r\nsa".as_slice(), ANSWERING].concat(),
        "no clean prompt while something is typed"
    );

    assert!(session.input(b"\x15".to_vec()));
    cx.run_until_parked();
    assert_eq!(
        shown(&session),
        [b"\x08 \x08\x08 \x08".as_slice(), READY].concat()
    );
}
