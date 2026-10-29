//! Retirement interrupts an idle event reader independently of transport output.
use super::physical::PhysicalTerminal;
use futures::{FutureExt as _, future::BoxFuture};
use nocterm_cloud_model::tool::ToolTimer;
use nocterm_session::{Event, Session};
use std::sync::{Arc, Mutex, PoisonError};

pub(super) struct SessionWatch {
    entry: Arc<Mutex<PhysicalTerminal>>,
    session: Session,
    timer: Arc<dyn ToolTimer>,
    retired: async_channel::Receiver<()>,
}
impl SessionWatch {
    pub(super) fn new(
        entry: Arc<Mutex<PhysicalTerminal>>,
        session: Session,
        timer: Arc<dyn ToolTimer>,
        retired: async_channel::Receiver<()>,
    ) -> Self {
        Self {
            entry,
            session,
            timer,
            retired,
        }
    }
    pub(super) fn run(self) -> BoxFuture<'static, ()> {
        let cleanup = ReaderLifetime(Some(self.entry.clone()));
        Box::pin(async move {
            loop {
                let retired = self.retired.recv().fuse();
                let event = self.session.next_event().fuse();
                futures::pin_mut!(retired, event);
                let event = futures::select_biased! {
                    _ = retired => break,
                    event = event => event,
                };
                let Some(event) = event else {
                    break;
                };
                let closed = matches!(&event, Event::Closed(_));
                self.entry
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .event(event, self.timer.now());
                if closed {
                    cleanup.finished();
                    return;
                }
            }
            self.entry
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .retire();
            cleanup.finished();
        })
    }
}

/// Own cleanup before the first poll, so dropping a queued reader cannot leak it.
struct ReaderLifetime(Option<Arc<Mutex<PhysicalTerminal>>>);
impl ReaderLifetime {
    fn finished(mut self) {
        self.0.take();
    }
}
impl Drop for ReaderLifetime {
    fn drop(&mut self) {
        if let Some(entry) = &self.0 {
            entry
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .retire();
        }
    }
}
