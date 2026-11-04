//! Following one host's engine: a listing, then another only of what its events change.

use std::{sync::Arc, time::Duration};

use futures::{
    FutureExt as _, StreamExt as _,
    channel::mpsc,
    future::{Either, select},
    pin_mut,
};
use gpui::{AsyncApp, WeakEntity};
use nocterm_containers::{Changes, ContainersError, Engine, EventReader, detect};
use nocterm_session::{ExecOutput, HostExec};

use crate::host::{HostContainers, Listed};

/// How long a failed listing, or an ended event stream, waits before the host is listed
/// again, unless someone asks sooner.
pub const RETRY: Duration = Duration::from_secs(10);
/// How long a burst of events (a Compose project starting) may take before the host is
/// listed once for all of it.
pub const SETTLE: Duration = Duration::from_millis(300);

/// Lists the host and follows its changes, again after failures, until the host has no
/// engine or nobody holds the entity any more.
pub(crate) async fn watch(
    this: WeakEntity<HostContainers>,
    exec: Arc<dyn HostExec>,
    mut engine: Option<Engine>,
    mut asked: mpsc::UnboundedReceiver<Changes>,
    cx: &mut AsyncApp,
) {
    loop {
        let outcome = follow(&this, exec.as_ref(), &mut engine, &mut asked, cx).await;
        let retry = this.update(cx, |this, cx| this.ended(outcome, cx));
        if !matches!(retry, Ok(true)) {
            return;
        }
        let timer = cx.background_executor().timer(RETRY);
        let request = asked.next();
        pin_mut!(timer, request);
        if let Either::Right((None, _)) = select(timer, request).await {
            return;
        }
    }
}

/// What woke the watch.
enum Heard {
    Output(Vec<u8>),
    Asked(Changes),
    Ended,
}

/// One attempt: a listing, then another of what changed after each burst of events that
/// change something, until the event stream ends.
async fn follow(
    this: &WeakEntity<HostContainers>,
    exec: &dyn HostExec,
    engine: &mut Option<Engine>,
    asked: &mut mpsc::UnboundedReceiver<Changes>,
    cx: &mut AsyncApp,
) -> Result<(), ContainersError> {
    let (found, snapshot) = match *engine {
        Some(known) => (known, known.snapshot(exec).await?),
        None => detect(exec).await?,
    };
    *engine = Some(found);
    // Requests made before this listing are answered by it.
    while asked.try_recv().is_ok() {}
    publish(this, found, Listed::Both(snapshot), cx)?;
    let mut events = found.events(exec).await?;
    let mut reader = EventReader::default();
    loop {
        let mut changes = match next(&mut events, asked).await {
            Heard::Output(chunk) => reader.feed(&chunk),
            Heard::Asked(changes) => changes,
            Heard::Ended => return Ok(()),
        };
        if changes.is_empty() {
            continue;
        }
        cx.background_executor().timer(SETTLE).await;
        while let Some(Some(chunk)) = events.next().now_or_never() {
            changes |= reader.feed(&chunk);
        }
        while let Ok(more) = asked.try_recv() {
            changes |= more;
        }
        publish(this, found, relist(found, exec, changes).await?, cx)?;
    }
}

async fn next(events: &mut ExecOutput, asked: &mut mpsc::UnboundedReceiver<Changes>) -> Heard {
    let output = events.next();
    let request = asked.next();
    pin_mut!(output, request);
    match select(output, request).await {
        Either::Left((Some(chunk), _)) => Heard::Output(chunk),
        Either::Right((Some(changes), _)) => Heard::Asked(changes),
        // The program ended, or nobody holds the entity any more.
        Either::Left((None, _)) | Either::Right((None, _)) => Heard::Ended,
    }
}

/// Only the listings `changes` names.
async fn relist(
    engine: Engine,
    exec: &dyn HostExec,
    changes: Changes,
) -> Result<Listed, ContainersError> {
    Ok(match (changes.containers(), changes.images()) {
        (true, false) => Listed::Containers(engine.containers(exec).await?),
        (false, true) => Listed::Images(engine.images(exec).await?),
        _ => Listed::Both(engine.snapshot(exec).await?),
    })
}

fn publish(
    this: &WeakEntity<HostContainers>,
    engine: Engine,
    listed: Listed,
    cx: &mut AsyncApp,
) -> Result<(), ContainersError> {
    this.update(cx, |this, cx| this.listed(engine, listed, cx))
        .map_err(|_| ContainersError::Disconnected)
}
