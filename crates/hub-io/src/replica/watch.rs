//! Change hints retain one original native source and never reopen dynamic Slot.
use crate::{account::operations::OpenedWork, store::PROFILES_KIND};
use futures::StreamExt as _;
use std::time::Duration;
use tokio::sync::mpsc;

const RETRY: Duration = Duration::from_secs(20);

pub(super) async fn follow(work: OpenedWork, ticks: mpsc::Sender<()>) {
    let source = work.source().clone();
    let Ok(slot) = source.native().slot() else {
        return;
    };
    let mut notice = slot.notice().subscribe();
    loop {
        if source.check().is_err() {
            return;
        }
        notice.borrow_and_update();
        tokio::select! {
            () = ticks.closed() => return,
            changed = notice.changed() => {
                if changed.is_err() || source.check().is_err() { return; }
                // Administrator pins changed in the same original OpenKeys.
                // Restart its original feed to include newly opened groups.
            },
            result = source.during(once(&work, &ticks)) => {
                if result.is_err() { return; }
            }
        }
    }
}

async fn once(work: &OpenedWork, ticks: &mpsc::Sender<()>) {
    let store = work.store();
    let collections: Vec<String> = match store.layout(PROFILES_KIND).await {
        Ok(layout) => layout
            .personal
            .iter()
            .chain(layout.groups.iter().map(|group| &group.at))
            .map(|at| at.id().to_owned())
            .collect(),
        Err(_) => Vec::new(),
    };
    if collections.is_empty() {
        let _ = work.source().publish(|| ticks.try_send(()));
        tokio::time::sleep(RETRY).await;
        return;
    }
    let Ok(mut hints) = work.ops().watch(collections).await else {
        tokio::time::sleep(RETRY).await;
        return;
    };
    let _ = work.source().publish(|| ticks.try_send(()));
    while hints.next().await.is_some() {
        if work.source().publish(|| ticks.try_send(())).is_err() {
            return;
        }
    }
    tokio::time::sleep(RETRY).await;
}
