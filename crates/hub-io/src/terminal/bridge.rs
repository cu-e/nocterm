//! Shows the events of a hub terminal as session events and carries typing back.

use std::future::Future;

use nocterm_hub_client::ClientError;
use nocterm_hub_client::terminal::{TerminalEnd, TerminalEvent, TerminalInput};
use nocterm_hubs_model::HubError;
use nocterm_session::{CloseReason, Command, ConnectStage, Event, SessionDriver, SessionError};
use tokio::sync::mpsc;

use super::session_error;

const INPUT_BACKLOG: usize = 64;
const EVENT_BACKLOG: usize = 64;

/// Runs `run` against the session until it ends.
///
/// `run` gets the typing to send and a place for what it shows. Typing arrives as the session's
/// commands; when the session is closed or its owner goes away the input ends, which makes
/// `run` detach. Every event `run` produced is shown before this returns.
pub(crate) async fn bridge<F, Fut>(
    driver: &SessionDriver,
    run: F,
) -> Result<TerminalEnd, ClientError>
where
    F: FnOnce(mpsc::Receiver<TerminalInput>, mpsc::Sender<TerminalEvent>) -> Fut,
    Fut: Future<Output = Result<TerminalEnd, ClientError>>,
{
    let (input_tx, input_rx) = mpsc::channel(INPUT_BACKLOG);
    let (event_tx, event_rx) = mpsc::channel(EVENT_BACKLOG);
    let shown = async {
        let (end, ()) = tokio::join!(run(input_rx, event_tx), show(driver, event_rx));
        end
    };
    tokio::pin!(shown);
    tokio::select! {
        () = forward_input(driver, input_tx) => shown.await,
        end = &mut shown => end,
    }
}

/// Sends commands of the session to the terminal until the session is closed or dropped.
async fn forward_input(driver: &SessionDriver, input: mpsc::Sender<TerminalInput>) {
    loop {
        let typed = tokio::select! {
            command = driver.next_command() => match command {
                Some(Command::Input(bytes)) => TerminalInput::Data(bytes),
                Some(Command::Resize(size)) => TerminalInput::Resize {
                    cols: size.cols,
                    rows: size.rows,
                },
                Some(Command::Close) | None => return,
            },
            () = driver.closed() => return,
        };
        tokio::select! {
            sent = input.send(typed) => if sent.is_err() { return },
            () = driver.closed() => return,
        }
    }
}

/// Emits what the terminal shows; stops, and so detaches, when nobody listens any more.
async fn show(driver: &SessionDriver, mut events: mpsc::Receiver<TerminalEvent>) {
    while let Some(event) = events.recv().await {
        if !driver.emit(session_event(event)).await {
            return;
        }
    }
}

fn session_event(event: TerminalEvent) -> Event {
    match event {
        TerminalEvent::Snapshot(screen) => Event::Restore(screen),
        TerminalEvent::Output(bytes) => Event::Output(bytes),
        TerminalEvent::Attached => Event::Connected,
        TerminalEvent::Reconnecting(_) => Event::Connecting(ConnectStage::Connecting { hop: None }),
    }
}

/// Why the session ended, when the terminal ended or failed.
pub(crate) fn close_reason(end: Result<TerminalEnd, HubError>) -> CloseReason {
    match end {
        Ok(TerminalEnd::Exited(code)) => CloseReason::Exited(u32::try_from(code).ok()),
        Ok(TerminalEnd::Lost(reason)) => CloseReason::Failed(SessionError::ConnectionLost(reason)),
        Ok(TerminalEnd::Refused(reason)) => CloseReason::Failed(SessionError::Other(reason)),
        Ok(TerminalEnd::Detached) => CloseReason::ClosedByUser,
        Err(error) => failed(&error),
    }
}

pub(crate) fn failed(error: &HubError) -> CloseReason {
    CloseReason::Failed(session_error(error))
}
