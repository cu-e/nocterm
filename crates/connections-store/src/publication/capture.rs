//! Immutable coupled capture uses the existing two-notification feed and shared gate.
use super::*;
pub(super) fn register(
    profiles: Entity<Persisted<ProfileSnapshot>>,
    recents: Entity<Persisted<Recents>>,
    consumed: Rc<Cell<u64>>,
    shutdown: &ShutdownCoordinator,
) -> Result<(), StoreError> {
    shutdown.register_capture(move |cx| {
        let consumed = consumed.get();
        let profiles = profiles.update(cx, |state, _| state.capture());
        let recents = recents.update(cx, |state, _| state.capture());
        let selector = ProjectionSelector::new(&profiles, recents.final_snapshot(), consumed);
        let (profiles, feed) = profiles.run_observed();
        let recents = recents.run_projected(feed, move |phase, committed| {
            selector.select(phase, committed)
        });
        join_captured(profiles, recents, cx)
    })
}
struct ProjectionSelector {
    durable: (u64, u64),
    prefix: Option<((u64, u64), Recents)>,
    final_branch: ((u64, u64), Recents),
}
impl ProjectionSelector {
    fn new(
        profiles: &nocterm_store::Captured<ProfileSnapshot>,
        baseline: &Recents,
        consumed: u64,
    ) -> Self {
        let prefix = profiles.active().map(|active| {
            let mut recents = baseline.clone();
            active.snapshot().apply_effects(&mut recents, consumed);
            ((active.revision(), active.snapshot().generation()), recents)
        });
        let mut final_recents = baseline.clone();
        profiles
            .final_snapshot()
            .apply_effects(&mut final_recents, consumed);
        Self {
            durable: (
                profiles.durable().revision(),
                profiles.durable().snapshot().generation(),
            ),
            prefix,
            final_branch: (
                (
                    profiles.final_revision(),
                    profiles.final_snapshot().generation(),
                ),
                final_recents,
            ),
        }
    }
    fn select(
        &self,
        phase: CommitPhase,
        committed: &Committed<ProfileSnapshot>,
    ) -> Result<Option<Recents>, StoreError> {
        let key = (committed.revision(), committed.snapshot().generation());
        if phase == CommitPhase::Final && key == self.final_branch.0 {
            return Ok(Some(self.final_branch.1.clone()));
        }
        if let Some((prefix_key, recents)) = &self.prefix
            && key == *prefix_key
        {
            return Ok(Some(recents.clone()));
        }
        if key == self.durable {
            return Ok(None);
        }
        Err(StoreError::backend(UnknownProjection))
    }
}
#[derive(Debug, thiserror::Error)]
#[error("captured profile projection was not one of the prepared committed snapshots")]
struct UnknownProjection;

fn join_captured(
    profiles: BoxFuture<'static, nocterm_store::CapturedOutcome<ProfileSnapshot>>,
    recents: BoxFuture<'static, nocterm_store::CapturedOutcome<Recents>>,
    cx: &App,
) -> BoxFuture<'static, Result<(), StoreError>> {
    let executor = cx.background_executor().clone();
    Box::pin(async move {
        let profiles = executor.spawn(profiles);
        let recents = executor.spawn(recents);
        let (profiles, recents) = futures::join!(profiles, recents);
        match (profiles.into_result(), recents.into_result()) {
            (Ok(()), Ok(())) => Ok(()),
            (Err(error), Ok(())) | (Ok(()), Err(error)) => Err(error),
            (Err(profiles), Err(recents)) => {
                Err(StoreError::joined(Arc::new(profiles), Arc::new(recents)))
            }
        }
    })
}

#[cfg(test)]
mod tests;
