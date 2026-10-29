//! Serial remote work avoids overlapping source checks with token rotation.
use super::credentials::CredentialOperations;
use crate::{RunnerError, bootstrap::State};
use nocterm_cloud_model::{
    coordinator::IntakePoolLimits, journal::PublicationBudget, tool::ToolTimer,
};
use nocterm_cloud_runner::NativeToolTimer;
use nocterm_hub_io::cloud::{
    RunnerHeadOutcome, RunnerHeadResponder, RunnerHeadWork, RunnerHubPeer,
};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

pub(super) fn work() -> Result<Arc<RunnerHeadWork>, RunnerError> {
    let bytes = RunnerHeadWork::operation_bytes()
        .checked_mul(2)
        .ok_or(RunnerError::Value("head work budget"))?;
    let limits =
        IntakePoolLimits::new(bytes, 2, 2).map_err(|_| RunnerError::Value("head work budget"))?;
    Ok(RunnerHeadWork::new(
        Arc::new(PublicationBudget::maintenance(limits)),
        tokio::runtime::Handle::current(),
    ))
}
pub(super) async fn serve(
    state: State,
    peer: RunnerHubPeer,
    work: Arc<RunnerHeadWork>,
    ready: Arc<AtomicBool>,
    credentials: Arc<CredentialOperations>,
    mut stopped: tokio::sync::oneshot::Receiver<()>,
) -> Result<(), RunnerError> {
    let timer = Arc::new(NativeToolTimer::new(tokio::runtime::Handle::current()));
    let responder = RunnerHeadResponder::new(
        peer,
        state.identity(),
        state.account(),
        Arc::new(state.heads()),
        timer.clone(),
        work.clone(),
    )?;
    let mut highwater = timer.now().as_millis();
    let mut scheduled_access = None;
    let mut refresh_at = highwater;
    loop {
        match stopped.try_recv() {
            Ok(()) | Err(tokio::sync::oneshot::error::TryRecvError::Closed) => return Ok(()),
            Err(tokio::sync::oneshot::error::TryRecvError::Empty) => {}
        }
        state.connection().check_source()?;
        let now = timer.now().as_millis();
        if now < highwater {
            return Err(RunnerError::Clock);
        }
        highwater = now;
        let deadlines = state.connection().credential_deadlines()?;
        let access =
            u64::try_from(deadlines.access_expires_at_ms()).map_err(|_| RunnerError::Clock)?;
        let refresh_expires =
            u64::try_from(deadlines.refresh_expires_at_ms()).map_err(|_| RunnerError::Clock)?;
        if now >= refresh_expires {
            return Err(RunnerError::Expired);
        }
        if scheduled_access != Some(access) {
            let remaining = access.saturating_sub(now);
            let margin = (remaining / 10).clamp(1, 5_000);
            refresh_at = access.saturating_sub(margin);
            scheduled_access = Some(access);
        }
        if now >= refresh_at {
            ready.store(false, Ordering::Release);
            let previous_access = deadlines.access_expires_at_ms();
            refresh(&state, &work, &credentials, previous_access).await?;
            continue;
        }
        let wait = Duration::from_millis(
            state
                .limits()
                .head_wait_ms
                .min(refresh_at.saturating_sub(now)),
        );
        let result = tokio::select! {
            _ = &mut stopped => return Ok(()),
            result = tokio::time::timeout(state.limits().operation(), responder.respond_once(wait)) => result,
        };
        state.connection().check_source()?;
        match result {
            Ok(Ok(outcome)) => {
                ready.store(true, Ordering::Release);
                if outcome == RunnerHeadOutcome::Idle {
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            }
            _ => {
                ready.store(false, Ordering::Release);
                // A refused or malformed head query never acquires execution
                // authority and does not permanently kill this independent R.
                tokio::time::sleep(Duration::from_millis(250)).await;
            }
        }
    }
}

// One serialized native credential operation: old head owners finish before
// rotation, and only observed RPC plus durable publication establishes success.
async fn refresh(
    state: &State,
    work: &RunnerHeadWork,
    credentials: &Arc<CredentialOperations>,
    previous_access: i64,
) -> Result<(), RunnerError> {
    // Await actual old body/read destruction before setting rotating.
    tokio::time::timeout(state.limits().operation(), async {
        while work.in_flight_operations() != 0 || work.in_flight_reads() != 0 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .map_err(|_| RunnerError::Timeout)?;
    // Unknown mutation outcome fences/exits. This never retries HTTP or
    // clears a pending native write by restoring older on-disk tokens.
    credentials
        .observe(async {
            tokio::time::timeout(state.limits().operation(), state.connection().refresh())
                .await
                .map_err(|_| RunnerError::Timeout)??;
            Ok(())
        })
        .await?;
    let renewed = state.connection().credential_deadlines()?;
    if renewed.access_expires_at_ms() <= previous_access {
        return Err(RunnerError::Expired);
    }
    Ok(())
}
