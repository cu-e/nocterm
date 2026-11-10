//! One installation-scoped, coalesced projection of the shared transfer service.
//! Native snapshot locking/copying is background work; views observe immutable entries.
use gpui_kit::{Context, Task};
use nocterm_transfers::{Progress, TransferService};
use std::{sync::Arc, time::Duration};

pub(crate) struct TransferProjection {
    entries: Arc<[Progress]>,
    _poll: Task<()>,
}
impl TransferProjection {
    pub(crate) fn new(service: Arc<dyn TransferService>, cx: &mut Context<Self>) -> Self {
        let poll = cx.spawn(async move |this, cx| {
            let mut revision = None;
            let mut previous: Arc<[Progress]> = Arc::from([]);
            loop {
                let poll_service = service.clone();
                let before = previous.clone();
                let result = cx
                    .background_executor()
                    .spawn(async move {
                        let next = poll_service.revision();
                        if next.is_some() && next != Some(u64::MAX) && next == revision {
                            return (revision, None);
                        }
                        let snapshot: Arc<[Progress]> = poll_service.snapshot().into();
                        let changed = (snapshot != before).then_some(snapshot);
                        (next, changed)
                    })
                    .await;
                revision = result.0;
                if let Some(snapshot) = result.1 {
                    previous = snapshot.clone();
                    if this
                        .update(cx, |this, cx| {
                            this.entries = snapshot;
                            cx.notify();
                        })
                        .is_err()
                    {
                        return;
                    }
                } else if this.upgrade().is_none() {
                    return;
                }
                if let Some(wait) = revision.and_then(|revision| service.changed_since(revision)) {
                    wait.await;
                }
                cx.background_executor()
                    .timer(Duration::from_millis(150))
                    .await;
            }
        });
        Self {
            entries: Arc::from([]),
            _poll: poll,
        }
    }
    pub(crate) fn entries(&self) -> &Arc<[Progress]> {
        &self.entries
    }
}
