//! Carries a session's events into the terminal.

use gpui_kit::Context;
use nocterm_session::{CloseReason, Event, Session, SessionError};
use std::{future::poll_fn, task::Poll};

use super::{Status, Terminal};

/// Raw session output per entity update, including the first event and
/// outputs separated by control events. Decoding retains its own boundaries.
const OUTPUT_BATCH_BYTES: usize = 256 * 1024;
/// Control-only bursts and empty outputs must also yield to input/painting.
const BATCH_EVENTS: usize = 64;

impl Terminal {
    /// Hands `session`'s events to this terminal until it ends or is replaced.
    pub(super) fn spawn_pump(&mut self, session: Session, cx: &mut Context<Self>) {
        let epoch = self.connection_epoch;
        self._pump = Some(cx.spawn(async move |this, cx| {
            let mut pending = None;
            loop {
                let first = match pending.take() {
                    Some(event) => event,
                    None => match session.next_event().await {
                        Some(event) => Pending::new(event),
                        None => break,
                    },
                };
                let (batch, remainder) = drain(first, || session.try_next_event());
                pending = remainder;
                let alive = this.update(cx, |this, cx| {
                    if this.connection_epoch != epoch {
                        return false;
                    }
                    for event in batch {
                        this.handle_event(event, cx);
                    }
                    true
                });
                if !matches!(alive, Ok(true)) {
                    return;
                }
                // Receiving queued events and entity updates can both complete
                // synchronously. A bounded batch alone would not yield this
                // foreground task while a producer keeps the queue nonempty.
                yield_once().await;
            }
            // The transport went away without saying why.
            let _ = this.update(cx, |this, cx| {
                if this.connection_epoch == epoch && !matches!(this.status, Status::Closed(_)) {
                    let lost = SessionError::ConnectionLost("the transport stopped".into());
                    this.handle_event(Event::Closed(CloseReason::Failed(lost)), cx);
                }
            });
        }));
    }
}

/// A partial output retains its allocation and advances an offset: repeatedly
/// splitting off the tail of a huge event would copy that tail each batch.
struct Pending {
    event: Event,
    offset: usize,
}
impl Pending {
    fn new(event: Event) -> Self {
        Self { event, offset: 0 }
    }
}

/// Collects already queued events without consuming past either budget.
/// Adjacent output is joined, preserving the placement of all control events.
fn drain(first: Pending, mut next: impl FnMut() -> Option<Event>) -> (Vec<Event>, Option<Pending>) {
    let mut batch = Vec::new();
    let mut current = Some(first);
    let mut bytes = 0;
    for index in 0..BATCH_EVENTS {
        let Some(pending) = current.take() else {
            break;
        };
        match pending.event {
            Event::Output(output) => {
                let available = output.len() - pending.offset;
                let take = available.min(OUTPUT_BATCH_BYTES - bytes);
                let slice = &output[pending.offset..pending.offset + take];
                if let Some(Event::Output(joined)) = batch.last_mut() {
                    joined.extend_from_slice(slice);
                } else {
                    batch.push(Event::Output(slice.to_vec()));
                }
                bytes += take;
                if take < available {
                    return (
                        batch,
                        Some(Pending {
                            event: Event::Output(output),
                            offset: pending.offset + take,
                        }),
                    );
                }
            }
            event => batch.push(event),
        }
        if bytes == OUTPUT_BATCH_BYTES {
            break;
        }
        // Do not remove a 65th event from the session; it belongs to the next
        // batch. This also avoids a pending control event at an event boundary.
        if index + 1 == BATCH_EVENTS {
            break;
        }
        current = next().map(Pending::new);
    }
    (batch, current)
}

async fn yield_once() {
    let mut yielded = false;
    poll_fn(|cx| {
        if std::mem::replace(&mut yielded, true) {
            Poll::Ready(())
        } else {
            cx.waker().wake_by_ref();
            Poll::Pending
        }
    })
    .await;
}

#[cfg(test)]
mod tests;
