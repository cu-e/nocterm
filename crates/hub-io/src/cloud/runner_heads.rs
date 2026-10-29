//! Fresh read-only R witnesses over actual native authenticated current heads.
//! No writer claim, ordinary journal append, control increment or tool dispatch.
mod mapping;
mod work;
use self::{mapping::HeadBinding, work::HeadReservation};
use super::{CloudRunnerIdentity, RunnerHubPeer, runner_peer::RunnerHubConnection};
use futures::future::{BoxFuture, Either, select};
use nocterm_cloud_model::{
    journal::{CurrentCloudHeads, JournalError},
    tool::ToolTimer,
};
use nocterm_core::SharedCause;
use nocterm_hub_model::cloud::{
    capabilities::CapabilityWindow,
    head::{
        HeadProbeKeys, HeadProbeScope, HeadWitness, MAX_HEAD_CLOCK_SKEW_MS,
        MAX_HEAD_PROBE_LIFETIME_MS,
    },
};
use nocterm_keys::cloud::seal_head_witness;
use sha2::{Digest, Sha256};
use std::{
    fmt,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::Semaphore;
pub use work::RunnerHeadWork;

#[derive(Clone, Debug, thiserror::Error)]
pub enum RunnerHeadError {
    #[error("the fresh-head responder configuration is invalid")]
    Configuration,
    #[error("the head probe or native result differs from its captured account/R pins")]
    Binding,
    #[error("the requested chat is absent; no head was fabricated")]
    MissingChat,
    #[error("the original head window expired or its logical clock was fenced")]
    Expired,
    #[error("the original runner source is unavailable")]
    Source(#[source] SharedCause),
    #[error("the native current-head read was refused")]
    Journal(#[from] JournalError),
    #[error("the pinned head codec, proof or crypto was refused")]
    Codec(#[source] SharedCause),
    #[error("the finite native head task did not complete")]
    Task(#[source] SharedCause),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunnerHeadOutcome {
    Idle,
    Replied,
}

/// One captured native R source. Share RunnerHeadWork across responders; each
/// responder permits one actual operation, and starts no per-chat idle worker.
pub struct RunnerHeadResponder {
    connection: Arc<RunnerHubConnection>,
    identity: Arc<CloudRunnerIdentity>,
    reader: Arc<dyn CurrentCloudHeads>,
    timer: Arc<dyn ToolTimer>,
    work: Arc<RunnerHeadWork>,
    binding: HeadBinding,
    clock: Mutex<ClockFence>,
    operation: Arc<Semaphore>,
}
impl RunnerHeadResponder {
    pub fn new(
        peer: RunnerHubPeer,
        identity: Arc<CloudRunnerIdentity>,
        account: [u8; 32],
        reader: Arc<dyn CurrentCloudHeads>,
        timer: Arc<dyn ToolTimer>,
        work: Arc<RunnerHeadWork>,
    ) -> Result<Arc<Self>, RunnerHeadError> {
        let connection = peer.connection().clone();
        if !connection.shares_identity(&identity)
            || identity.binding() != connection.binding()
            || <[u8; 32]>::from(Sha256::digest(account))
                != *connection.binding().scope().account().key().as_bytes()
        {
            return Err(RunnerHeadError::Binding);
        }
        let public = identity.public_identity().map_err(source)?;
        let scope = HeadProbeScope {
            namespace: super::metadata_crypto::namespace(connection.binding())?,
            runner: super::metadata_crypto::runner(connection.binding()),
            keys: HeadProbeKeys {
                account,
                runner_encryption: *public.encryption_key(),
                runner_signing: *public.signing_key(),
            },
        };
        let binding = HeadBinding::new(connection.binding().clone(), scope)?;
        let value = Arc::new(Self {
            connection,
            identity,
            reader,
            timer,
            work,
            binding,
            clock: Mutex::new(ClockFence::default()),
            operation: Arc::new(Semaphore::new(1)),
        });
        value.check_current()?;
        Ok(value)
    }
    /// Captures/reserves before spawning on the injected adapter runtime. The
    /// real task result retains admission until the caller checks its source.
    pub fn respond_once(
        self: &Arc<Self>,
        wait: Duration,
    ) -> BoxFuture<'static, Result<RunnerHeadOutcome, RunnerHeadError>> {
        let owner = self.clone();
        Box::pin(async move {
            if wait > Duration::from_millis(MAX_HEAD_PROBE_LIFETIME_MS as u64) {
                return Err(RunnerHeadError::Configuration);
            }
            owner.check_current()?;
            let turn = owner
                .operation
                .clone()
                .try_acquire_owned()
                .map_err(|_| JournalError::Busy)?;
            let reservation = owner.work.reserve(turn)?;
            let task_owner = owner.clone();
            let task = crate::task::spawn(owner.work.runtime(), async move {
                let result = task_owner.respond(wait, reservation.clone()).await;
                TaskResult {
                    result,
                    _reservation: reservation,
                }
            });
            let result = task
                .await
                .ok_or_else(|| RunnerHeadError::Task(SharedCause::new(HeadTaskStopped)))?;
            owner.check_current()?;
            result.result
        })
    }
    async fn respond(
        self: &Arc<Self>,
        wait: Duration,
        reservation: Arc<HeadReservation>,
    ) -> Result<RunnerHeadOutcome, RunnerHeadError> {
        self.check_current()?;
        let runner = self.connection.runner().map_err(source)?;
        let ports = runner
            .head_probes_with_residency(self.binding.scope().clone(), reservation.clone())
            .map_err(codec)?;
        let response = ports.next(wait).await;
        self.check_current()?;
        let Some(mut incoming) = response.map_err(codec)? else {
            return Ok(RunnerHeadOutcome::Idle);
        };
        let now = self.check_current()?;
        let query = self
            .identity
            .with_keys(|keys| incoming.open_resident(keys.encryption(), self.binding.scope(), now))
            .map_err(source)?
            .map_err(codec)?;
        let original = query.query().query();
        self.check_window(original.window)?;
        let request = self.binding.request(original)?;
        let read_owner = self.clone();
        let window = original.window;
        let task = self.work.spawn_read(reservation.clone(), move || {
            read_owner.check_window(window)?;
            let value = futures::executor::block_on(request.load(read_owner.reader.clone()))?;
            read_owner.check_window(window)?;
            Ok(value)
        })?;
        let remaining = self.check_window(window)?;
        let delay = self.timer.delay(Duration::from_millis(remaining));
        let read = match select(task, delay).await {
            Either::Left((result, _delay)) => {
                result.map_err(|error| RunnerHeadError::Task(SharedCause::new(error)))?
            }
            Either::Right(((), _task)) => return Err(RunnerHeadError::Expired),
        };
        self.check_window(window)?;
        let value = self.binding.value(original, read.value()?)?;
        let witness = HeadWitness {
            query: original.clone(),
            observed_at_ms: self.check_current()?,
            value,
        };
        self.check_window(window)?;
        let signed = self.with_current_keys(window, |keys| {
            keys.signing().attest_head(witness).map_err(codec)
        })?;
        let ciphertext = self.with_current_keys(window, |keys| {
            seal_head_witness(keys.encryption(), &signed).map_err(codec)
        })?;
        self.check_window(window)?;
        let response = incoming.reply(ciphertext).await;
        self.check_window(window)?;
        response.map_err(codec)?;
        Ok(RunnerHeadOutcome::Replied)
    }
    fn check_current(&self) -> Result<i64, RunnerHeadError> {
        self.connection.check_source().map_err(source)?;
        self.clock
            .lock()
            .map_err(|_| RunnerHeadError::Expired)?
            .observe(self.timer.now().as_millis())
    }
    fn check_window(&self, window: CapabilityWindow) -> Result<u64, RunnerHeadError> {
        remaining(window, self.check_current()?)
    }
    fn with_current_keys<T>(
        &self,
        window: CapabilityWindow,
        action: impl FnOnce(&super::identity::IdentityKeys) -> Result<T, RunnerHeadError>,
    ) -> Result<T, RunnerHeadError> {
        self.connection.check_source().map_err(source)?;
        let mut clock = self.clock.lock().map_err(|_| RunnerHeadError::Expired)?;
        remaining(window, clock.observe(self.timer.now().as_millis())?)?;
        // No await under either lock. A previously observed rollback cannot
        // race the finite purpose-specific key action and revive old authority.
        let result = self.identity.with_keys(action).map_err(source)??;
        remaining(window, clock.observe(self.timer.now().as_millis())?)?;
        self.connection.check_source().map_err(source)?;
        Ok(result)
    }
}
#[derive(Default)]
struct ClockFence {
    last: Option<i64>,
    failed: bool,
}
impl ClockFence {
    fn observe(&mut self, now: u64) -> Result<i64, RunnerHeadError> {
        let now = i64::try_from(now).map_err(|_| {
            self.failed = true;
            RunnerHeadError::Expired
        })?;
        if self.failed || now <= 0 || self.last.is_some_and(|last| now < last) {
            self.failed = true;
            return Err(RunnerHeadError::Expired);
        }
        self.last = Some(now);
        Ok(now)
    }
}
fn remaining(window: CapabilityWindow, now: i64) -> Result<u64, RunnerHeadError> {
    if now.saturating_add(MAX_HEAD_CLOCK_SKEW_MS) < window.issued_at_ms
        || now >= window.expires_at_ms
    {
        return Err(RunnerHeadError::Expired);
    }
    u64::try_from(window.expires_at_ms - now).map_err(|_| RunnerHeadError::Expired)
}
struct TaskResult {
    result: Result<RunnerHeadOutcome, RunnerHeadError>,
    _reservation: Arc<HeadReservation>,
}
#[derive(Debug, thiserror::Error)]
#[error("the adapter runtime stopped before the head task result was delivered")]
struct HeadTaskStopped;
fn source(error: impl std::error::Error + Send + Sync + 'static) -> RunnerHeadError {
    RunnerHeadError::Source(SharedCause::new(error))
}
fn codec(error: impl std::error::Error + Send + Sync + 'static) -> RunnerHeadError {
    RunnerHeadError::Codec(SharedCause::new(error))
}
impl fmt::Debug for RunnerHeadResponder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RunnerHeadResponder")
            .field("binding", self.connection.binding())
            .finish_non_exhaustive()
    }
}
#[cfg(test)]
mod tests;
