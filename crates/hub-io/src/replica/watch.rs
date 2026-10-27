//! Telling the profile reconciliation that the hub may have news.
//!
//! A task on the adapter's threads follows the hub's change hints and ticks when the feed
//! connects, when a hint arrives and when the vault unlocks (profiles can only be read with the
//! keys). It reconnects with a pause after a failure and ends when the stream is dropped.

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
    let Ok(Some(at)) = store.find(PROFILES_KIND).await else {
        let _ = ticks.try_send(());
        tokio::time::sleep(RETRY).await;
        return;
    };
    let Ok(mut hints) = ops.watch(vec![at.id().to_owned()]).await else {
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
            }
        }
    }
    tokio::time::sleep(RETRY).await;
}
