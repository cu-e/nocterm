use std::sync::{Arc, Mutex};
use std::time::Duration;

use nocterm_hub_client::ClientError;
use nocterm_hub_client::terminal::{TerminalEnd, TerminalEvent, TerminalInput};
use nocterm_session::{CloseReason, ConnectStage, Event, PtySize, SessionError};
use tokio::sync::mpsc;

use super::bridge::{bridge, close_reason};

/// What a scripted terminal saw of the session's typing.
type Typed = Arc<Mutex<Vec<TerminalInput>>>;

/// A terminal that shows `script` and then waits for the input to end before it reports `end`;
/// with `wait_for_input` false it reports `end` as soon as the script is shown.
async fn run_script(
    mut input: mpsc::Receiver<TerminalInput>,
    events: mpsc::Sender<TerminalEvent>,
    script: Vec<TerminalEvent>,
    typed: Typed,
    wait_for_input: bool,
    end: TerminalEnd,
) -> Result<TerminalEnd, ClientError> {
    for event in script {
        if events.send(event).await.is_err() {
            return Ok(TerminalEnd::Detached);
        }
    }
    if wait_for_input {
        while let Some(typed_input) = input.recv().await {
            typed.lock().unwrap().push(typed_input);
        }
        return Ok(TerminalEnd::Detached);
    }
    Ok(end)
}

async fn next(session: &nocterm_session::Session) -> Event {
    tokio::time::timeout(Duration::from_secs(5), session.next_event())
        .await
        .expect("an event in time")
        .expect("the session is open")
}

#[tokio::test]
async fn contract_events_are_shown_as_session_events_in_order() {
    let (session, driver) = nocterm_session::channel(None);
    let script = vec![
        TerminalEvent::Snapshot(b"screen".to_vec()),
        TerminalEvent::Attached,
        TerminalEvent::Output(b"out".to_vec()),
        TerminalEvent::Reconnecting("link dropped".into()),
        TerminalEvent::Snapshot(b"again".to_vec()),
        TerminalEvent::Attached,
    ];
    let typed = Typed::default();
    let end = bridge(&driver, |input, events| {
        run_script(input, events, script, typed, false, TerminalEnd::Exited(3))
    })
    .await;
    let reason = close_reason(end.map_err(|_| unreachable!("the script does not fail")));
    assert_eq!(reason, CloseReason::Exited(Some(3)));

    assert!(matches!(next(&session).await, Event::Restore(b) if b == b"screen"));
    assert!(matches!(next(&session).await, Event::Connected));
    assert!(matches!(next(&session).await, Event::Output(b) if b == b"out"));
    assert!(matches!(
        next(&session).await,
        Event::Connecting(ConnectStage::Connecting { hop: None })
    ));
    assert!(matches!(next(&session).await, Event::Restore(b) if b == b"again"));
    assert!(matches!(next(&session).await, Event::Connected));
}

#[tokio::test]
async fn typing_and_resizes_reach_the_terminal_and_closing_detaches() {
    let (session, driver) = nocterm_session::channel(None);
    let typed = Typed::default();
    let seen = Arc::clone(&typed);
    let script = vec![TerminalEvent::Attached];
    let run = bridge(&driver, |input, events| {
        run_script(input, events, script, seen, true, TerminalEnd::Detached)
    });
    let user = async {
        assert!(matches!(next(&session).await, Event::Connected));
        session.input("ls\r");
        session.resize(PtySize {
            cols: 100,
            rows: 30,
            pixel_width: 0,
            pixel_height: 0,
        });
        // Let the bridge carry both before the session is closed.
        tokio::time::sleep(Duration::from_millis(50)).await;
        session.close();
    };
    let (end, ()) = tokio::join!(run, user);
    assert!(matches!(end, Ok(TerminalEnd::Detached)));
    let typed = typed.lock().unwrap().clone();
    assert!(typed.contains(&TerminalInput::Data(b"ls\r".to_vec())));
    assert!(typed.contains(&TerminalInput::Resize {
        cols: 100,
        rows: 30
    }));
}

#[tokio::test]
async fn dropping_the_session_detaches_the_terminal() {
    let (session, driver) = nocterm_session::channel(None);
    let typed = Typed::default();
    let run = bridge(&driver, |input, events| {
        run_script(
            input,
            events,
            Vec::new(),
            typed,
            true,
            TerminalEnd::Detached,
        )
    });
    let user = async {
        tokio::time::sleep(Duration::from_millis(20)).await;
        drop(session);
    };
    let (end, ()) = tokio::join!(run, user);
    assert!(matches!(end, Ok(TerminalEnd::Detached)));
}

#[tokio::test]
async fn an_owner_that_stopped_listening_detaches_the_terminal() {
    let (session, driver) = nocterm_session::channel(None);
    drop(session);
    let typed = Typed::default();
    let script = vec![
        TerminalEvent::Attached,
        TerminalEvent::Output(b"x".to_vec()),
    ];
    let end = bridge(&driver, |input, events| {
        run_script(input, events, script, typed, true, TerminalEnd::Detached)
    })
    .await;
    assert!(matches!(end, Ok(TerminalEnd::Detached)));
}

#[test]
fn how_a_terminal_ended_is_why_the_session_closed() {
    assert_eq!(
        close_reason(Ok(TerminalEnd::Exited(0))),
        CloseReason::Exited(Some(0))
    );
    assert_eq!(
        close_reason(Ok(TerminalEnd::Exited(-1))),
        CloseReason::Exited(None)
    );
    assert_eq!(
        close_reason(Ok(TerminalEnd::Detached)),
        CloseReason::ClosedByUser
    );
    assert_eq!(
        close_reason(Ok(TerminalEnd::Lost("node restarted".into()))),
        CloseReason::Failed(SessionError::ConnectionLost("node restarted".into()))
    );
    assert_eq!(
        close_reason(Ok(TerminalEnd::Refused("forbidden".into()))),
        CloseReason::Failed(SessionError::Other("forbidden".into()))
    );
}

#[test]
fn a_failed_attach_closes_the_session_with_the_reason() {
    use nocterm_hubs_model::HubError;

    assert!(matches!(
        close_reason(Err(HubError::Revoked)),
        CloseReason::Failed(SessionError::Other(_))
    ));
    assert!(matches!(
        close_reason(Err(HubError::NotSignedIn)),
        CloseReason::Failed(SessionError::Other(_))
    ));
}
