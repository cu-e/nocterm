//! Telling the profile reconciliation that the hub may have news.
//!
//! A task on the adapter's threads follows the hub's change hints of the account's own
//! collection and of every group's it holds the key of, and ticks when the feed connects, when a
//! hint arrives, when the vault unlocks (profiles can only be read with the keys) and when the
//! open keys change, as they do when an administrator is confirmed. It
//! reconnects with a pause after a failure and ends when the stream is dropped. A group that
//! gets its key later is followed from the next connection on; every synchronization still reads
//! it.

use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt as _;
use tokio::sync::mpsc;

use crate::account::Slot;
use crate::store::{PROFILES_KIND, Store, StoreOps};

/// Pause before reconnecting after the feed failed, or before looking again for a collection
/// of profiles nobody made yet.
const RETRY: Duration = Duration::from_secs(20);

pub(super) async fn follow(slot: Arc<Slot>, ops: Arc<dyn StoreOps>, ticks: mpsc::Sender<()>) {
    let mut notice = slot.notice().subscribe();
    loop {
        tokio::select! {
            () = ticks.closed() => return,
            () = once(&slot, &ops, &ticks, &mut notice) => {}
        }
    }
}

/// One connection of the feed, or one wait.
async fn once(
    slot: &Slot,
    ops: &Arc<dyn StoreOps>,
    ticks: &mpsc::Sender<()>,
    notice: &mut tokio::sync::watch::Receiver<u64>,
) {
    let Some(keys) = slot.open() else {
        // Locked: nothing can be read. Unlocking raises the notice.
        let _ = notice.changed().await;
        return;
    };
    notice.borrow_and_update();
    let store = Store::new(Arc::clone(ops), keys);
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
        let _ = ticks.try_send(());
        tokio::time::sleep(RETRY).await;
        return;
    }
    let Ok(mut hints) = ops.watch(collections).await else {
        tokio::time::sleep(RETRY).await;
        return;
    };
    let _ = ticks.try_send(());
    loop {
        tokio::select! {
            hint = hints.next() => match hint {
                Some(()) => { let _ = ticks.try_send(()); }
                None => break,
            },
            changed = notice.changed() => {
                if changed.is_err() || slot.open().is_none() {
                    return;
                }
                // The keys changed while open: a group may have become readable (an
                // administrator was confirmed), so the profiles are read again.
                let _ = ticks.try_send(());
            }
        }
    }
    tokio::time::sleep(RETRY).await;
}
