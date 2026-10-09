//! Finishing connection writes when the application quits.

use std::time::{Duration, Instant};

use futures::future::{Either, select};
use gpui_kit::{App, Entity};
use nocterm_core::persist;

use super::Connections;

/// GPUI allows 200 ms for native shutdown callbacks.
const BUDGET: Duration = Duration::from_millis(180);

/// Lets in-flight writes finish, then saves the latest recent connections.
///
/// The app stays borrowed while quit futures run, so the future works only on
/// state captured here; queued profile writes can no longer run.
pub(super) fn save_on_quit(connections: Entity<Connections>, cx: &mut App) {
    cx.on_app_quit(move |cx| {
        let (in_flight, queued, recents) = connections.update(cx, |this, _| {
            let queued = this.queue.close();
            let recents = (this.recents_revision != this.recents_written_revision)
                .then(|| this.recents_file.clone().map(|path| (path, this.recents.clone())))
                .flatten();
            (this.queue.in_flight(), queued, recents)
        });
        let executor = cx.background_executor().clone();
        async move {
            if queued > 0 {
                tracing::warn!(queued, "connection changes queued at shutdown were not saved");
            }
            let deadline = Instant::now() + BUDGET;
            while in_flight.count() > 0 {
                if Instant::now() >= deadline {
                    tracing::warn!(
                        "timed out saving connections during shutdown; queued writes may be incomplete"
                    );
                    return;
                }
                executor.timer(Duration::from_millis(10)).await;
            }
            if let Some((path, recents)) = recents {
                let remaining = deadline.saturating_duration_since(Instant::now());
                let work = executor.spawn(async move { persist::save(&path, &recents) });
                match select(work, executor.timer(remaining)).await {
                    Either::Left((Err(error), _)) => {
                        tracing::warn!(%error, "could not save final recent connections")
                    }
                    Either::Right(_) => tracing::warn!("timed out saving final recent connections"),
                    Either::Left((Ok(()), _)) => {}
                }
            }
        }
    })
    .detach();
}
