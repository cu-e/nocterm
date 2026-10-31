//! Native captured Cloud conversation transport, durable outbox and bounded cache.
//! Owns: original account fences, real SDK work and private encrypted I/O.
//! Does not own: renderer, inference policy selection, execution, or sparse
//! snapshot activation. Account/R pins and configuration are explicit inputs.
mod cache;
mod capture;
mod clock;
mod commands;
mod drivers;
mod grants;
mod heads;
mod lifetime;
mod outbox;
mod persistence;
mod policy;
mod replay;
mod requirements;
mod routes;
mod storage;
#[cfg(test)]
mod tests;
mod work;
pub use drivers::ConversationStorage;
pub use grants::ReserveWork;
pub use policy::ConversationPromptPolicy;
pub use work::ConversationWorkers;

use crate::cloud::AccountObservationDecoder;
pub(crate) use capture::NativeSource;
use clock::SourceClock;
use futures::future::BoxFuture;
use nocterm_cloud_model::{
    identity::*,
    projection::{
        ConversationProjection, ProjectionBinding, ProjectionCursor, ProjectionError, ports::*,
    },
    tool::ToolTimer,
};
use nocterm_core::allocation::{Allocated, AllocationLease};
use nocterm_hub_client::cloud::CloudError;
use policy::ConversationPolicy;
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
    time::Duration,
};

const RESIDENT_BYTES: usize = 64 * 1024;
const MAX_WIRE_BYTES: usize = 4 * 1024 * 1024;

pub struct ConversationSetup {
    policy: ConversationPolicy,
    workers: Arc<ConversationWorkers>,
    timer: Arc<dyn ToolTimer>,
    storage: Arc<ConversationStorage>,
}
impl ConversationSetup {
    pub fn new(
        policy: ConversationPromptPolicy,
        workers: Arc<ConversationWorkers>,
        timer: Arc<dyn ToolTimer>,
        storage: Arc<ConversationStorage>,
    ) -> Self {
        Self {
            policy: ConversationPolicy::for_prompt(policy),
            workers,
            timer,
            storage,
        }
    }
    /// History/control capture works with no invented provider/model/budget.
    pub fn controls(
        lifetime_ms: u64,
        workers: Arc<ConversationWorkers>,
        timer: Arc<dyn ToolTimer>,
        storage: Arc<ConversationStorage>,
    ) -> Result<Self, ProjectionError> {
        Ok(Self {
            policy: ConversationPolicy::controls(lifetime_ms)?,
            workers,
            timer,
            storage,
        })
    }
}
pub struct CloudConversationSource {
    core: Arc<SourceCore>,
}
struct SourceCore {
    native: Arc<NativeSource>,
    binding: ProjectionBinding,
    decoder: AccountObservationDecoder,
    workers: Arc<ConversationWorkers>,
    clock: SourceClock,
    policy: ConversationPolicy,
    files: persistence::Files,
    progress: Mutex<Progress>,
    lifetime: lifetime::SourceLifetime,
    _resident: AllocationLease,
}
struct StageAttempt {
    intent: Fingerprint,
    unsent: bool,
    urgent: bool,
}
struct Progress {
    head: heads::FreshHead,
    routes: routes::ReplayRoutes,
    attempts: BTreeMap<CommandId, StageAttempt>,
    maintenance: Option<ProjectionError>,
}
impl CloudConversationSource {
    pub fn open(
        access: &crate::cloud::CloudAccountMaterial,
        material: Arc<crate::cloud::AccountChatMaterial>,
        setup: ConversationSetup,
        resident: AllocationLease,
        work: AllocationLease,
    ) -> BoxFuture<'static, Result<Arc<Self>, ProjectionError>> {
        let prepared = (|| {
            if resident.bytes() < RESIDENT_BYTES
                || resident.payloads() == 0
                || work.payloads() == 0
                || work.bytes() < Self::capture_work_bytes()?
            {
                return Err(ProjectionError::Capacity);
            }
            let (bound, cloud, authority) = access.conversation_parts();
            NativeSource::new(bound, cloud, material, authority)
        })();
        let native = match prepared {
            Ok(native) => native,
            Err(error) => return Box::pin(async move { Err(error) }),
        };
        let workers = setup.workers.clone();
        let timer = setup.timer.clone();
        let original = native.clone();
        let future = Self::capture(
            native.clone(),
            setup.policy,
            setup.workers,
            setup.timer,
            setup.storage,
            resident,
            work.clone(),
        );
        let future = async move {
            tokio::select! {result=future=>result,_=original.ended()=>Err(ProjectionError::Stale),_=timer.delay(Duration::from_secs(10))=>Err(ProjectionError::Unavailable)}
        };
        let worker = workers.task(work, future);
        Box::pin(async move {
            let result = worker
                .await
                .ok_or(ProjectionError::Unavailable)?
                .into_allocated()?;
            native.check()?;
            Ok(result.value().clone())
        })
    }
    /// Native composition only. The account facade supplies these actual owners;
    /// a UI selector cannot instantiate their authority from descriptive DTOs.
    async fn capture(
        native: Arc<NativeSource>,
        mut policy: ConversationPolicy,
        workers: Arc<ConversationWorkers>,
        timer: Arc<dyn ToolTimer>,
        storage: Arc<ConversationStorage>,
        resident: AllocationLease,
        work: AllocationLease,
    ) -> Result<Arc<Self>, ProjectionError> {
        if resident.bytes() < RESIDENT_BYTES
            || resident.payloads() == 0
            || work.payloads() == 0
            || work.bytes() < Self::capture_work_bytes()?
        {
            return Err(ProjectionError::Capacity);
        }
        policy.bind_capture()?;
        let clock = SourceClock::new(timer);
        let head = heads::current(&native, &clock, &heads::ProbeWork::new(work.clone())?).await?;
        let writer = head
            .value()
            .head
            .facts()
            .writer
            .ok_or(ProjectionError::Unavailable)?;
        let binding = ProjectionBinding::new(native.material().request().stream.clone(), writer);
        let files = persistence::open(
            native.clone(),
            workers.clone(),
            storage,
            &binding,
            head.value(),
            work.clone(),
        )
        .await?;
        let decoder = AccountObservationDecoder::new(native.material().clone(), binding.clone())
            .map_err(observation_error)?;
        let source = Self {
            core: Arc::new(SourceCore {
                native,
                binding,
                decoder,
                workers,
                clock,
                policy,
                files,
                progress: Mutex::new(Progress {
                    head: head.value().clone(),
                    routes: routes::ReplayRoutes::new(),
                    attempts: BTreeMap::new(),
                    maintenance: None,
                }),
                lifetime: lifetime::SourceLifetime::new(),
                _resident: resident,
            }),
        };
        source.check_current()?;
        Ok(Arc::new(source))
    }
    pub fn capture_work_bytes() -> Result<usize, ProjectionError> {
        MAX_WIRE_BYTES
            .checked_mul(2)
            .and_then(|bytes| bytes.checked_add(heads::work_bytes()))
            .and_then(|bytes| bytes.checked_add(1024 * 1024))
            .ok_or(ProjectionError::Capacity)
    }
    /// Exact shared Stop resident+work preflight, including the consumer's fixed
    /// pending metadata margin and two actual installation payload slots.
    pub fn urgent_requirements() -> Result<AllocationRequirements, ProjectionError> {
        let action = nocterm_ai::conversation::ConversationAction::Stop;
        let bytes = ConversationCommand::action_bytes(&action)
            .checked_add(512)
            .and_then(|bytes| bytes.checked_add(requirements::action(&action).ok()?))
            .ok_or(ProjectionError::Capacity)?;
        Ok(AllocationRequirements::new(bytes, 2))
    }
    pub fn resident_bytes() -> usize {
        RESIDENT_BYTES
    }
    /// Bounded recovery IDs only; original bodies remain encrypted. The native
    /// composition can query/deliver these exact IDs without manufacturing a new
    /// command or pretending that a previous pending UI payload was restored.
    pub fn pending_ids(
        &self,
        work: AllocationLease,
    ) -> BoxFuture<'static, Result<Allocated<Vec<CommandId>>, ProjectionError>> {
        let core = self.core.clone();
        Box::pin(async move {
            core.require_work(&work, Self::pending_work_bytes())?;
            let current = core.clone();
            let workers = core.workers.clone();
            let result = workers
                .blocking(work, false, move || {
                    core.check()?;
                    let ids = core
                        .files
                        .outbox()
                        .lock()
                        .map_err(|_| ProjectionError::Unavailable)?
                        .pending()?;
                    core.check()?;
                    Ok(ids)
                })?
                .await
                .map_err(|_| ProjectionError::Unavailable)?;
            current.check()?;
            result.into_allocated()
        })
    }
    pub fn pending_work_bytes() -> usize {
        64 * 1024
    }
}
impl CloudConversationSource {
    async fn poll_page(
        core: Arc<SourceCore>,
        after: ProjectionCursor,
        allocation: AllocationLease,
    ) -> Result<replay::NativePoll, ProjectionError> {
        // Idle waits use only head work. Every actual record/network read has a
        // finite deadline and an independent original-membership end race.
        let operation = replay::page(core.clone(), after, allocation);
        tokio::pin!(operation);
        tokio::select! {value=&mut operation=>value,_=core.ended()=>Err(ProjectionError::Stale)}
    }
    fn run_unit(
        &self,
        allocation: AllocationLease,
        operation: BoxFuture<'static, Result<(), ProjectionError>>,
    ) -> BoxFuture<'static, Result<(), ProjectionError>> {
        self.run_copy(allocation, operation)
    }
    fn run_copy<T: Copy + Send + 'static>(
        &self,
        allocation: AllocationLease,
        operation: BoxFuture<'static, Result<T, ProjectionError>>,
    ) -> BoxFuture<'static, Result<T, ProjectionError>> {
        let core = self.core.clone();
        let source = core.clone();
        let workers = core.workers.clone();
        let worker=workers.task(allocation,async move{
            tokio::select!{value=operation=>value,_=core.ended()=>Err(ProjectionError::Stale),_=core.clock.timer().delay(Duration::from_secs(10))=>Err(ProjectionError::Unavailable)}
        });
        Box::pin(async move {
            let value = worker
                .await
                .ok_or(ProjectionError::Unavailable)?
                .into_allocated()
                .inspect_err(|error| source.lifetime.refuse(error))?;
            source.check()?;
            Ok(*value.value())
        })
    }
}
impl CapturedConversationSource for CloudConversationSource {
    fn supports_prompt(&self) -> bool {
        self.core.policy.allows_prompt()
    }
    fn binding(&self) -> &ProjectionBinding {
        &self.core.binding
    }
    fn check_current(&self) -> Result<(), ProjectionError> {
        self.core.check()
    }
    fn ended(&self) -> BoxFuture<'static, ()> {
        let core = self.core.clone();
        Box::pin(async move { core.ended().await })
    }
    fn with_current(&self, publish: &mut dyn FnMut()) -> Result<(), ProjectionError> {
        self.check_current()?;
        self.core.publish(publish)
    }
    fn allocation_requirements(
        &self,
        work: ConversationWork<'_>,
    ) -> Result<AllocationRequirements, ProjectionError> {
        self.check_current()?;
        let bytes = match work {
            ConversationWork::Stage { command } => requirements::stage(command)?,
            ConversationWork::Replay {
                after,
                maximum_bytes,
            } => {
                if maximum_bytes > nocterm_cloud_model::projection::MAX_REPLAY_BYTES {
                    return Err(ProjectionError::Capacity);
                }
                let progress = self
                    .core
                    .progress
                    .lock()
                    .map_err(|_| ProjectionError::Unavailable)?;
                return progress
                    .routes
                    .requirements(after, head_cursor(&progress.head.head));
            }
            ConversationWork::Cache { projection } => {
                if projection.binding() != &self.core.binding {
                    return Err(ProjectionError::WrongSource);
                }
                64 * 1024
            }
        };
        Ok(AllocationRequirements::new(bytes, 1))
    }
    fn prepare(
        &self,
        coordinates: ConversationCoordinates,
        action: nocterm_cloud_model::projection::ConversationIntent,
    ) -> Result<ConversationCommand, ProjectionError> {
        commands::prepare(&self.core, coordinates, action)
    }
    fn stage(
        &self,
        command: Arc<Allocated<ConversationCommand>>,
        work: AllocationLease,
    ) -> BoxFuture<'static, Result<(), ProjectionError>> {
        self.run_unit(
            work.clone(),
            commands::stage(self.core.clone(), command, work),
        )
    }
    fn deliver(
        &self,
        id: CommandId,
        work: AllocationLease,
    ) -> BoxFuture<'static, Result<CommandDelivery, ProjectionError>> {
        self.run_copy(work.clone(), commands::deliver(self.core.clone(), id, work))
    }
    fn query(
        &self,
        id: CommandId,
        work: AllocationLease,
    ) -> BoxFuture<'static, Result<CommandDelivery, ProjectionError>> {
        self.run_copy(work.clone(), commands::query(self.core.clone(), id, work))
    }
    fn next(
        &self,
        after: ProjectionCursor,
        allocation: AllocationLease,
    ) -> BoxFuture<'static, Result<ConversationReplay, ProjectionError>> {
        let core = self.core.clone();
        let source = core.clone();
        let worker = core.workers.task(
            allocation.clone(),
            Self::poll_page(core.clone(), after, allocation),
        );
        Box::pin(async move {
            let value = worker.await.ok_or(ProjectionError::Unavailable)?;
            source.check()?;
            value
                .into_replay()
                .inspect_err(|error| source.lifetime.refuse(error))
        })
    }
    fn cache(
        &self,
        projection: Arc<Allocated<ConversationProjection>>,
        work: AllocationLease,
    ) -> BoxFuture<'static, Result<(), ProjectionError>> {
        self.run_unit(
            work.clone(),
            persistence::confirm(self.core.clone(), projection, work),
        )
    }
}
fn sdk_error(error: CloudError) -> ProjectionError {
    match error {
        CloudError::SourceChanged | CloudError::NotSignedIn => ProjectionError::Stale,
        CloudError::Unsupported => ProjectionError::Unsupported,
        CloudError::Protocol
        | CloudError::Capability(_)
        | CloudError::Relay(_)
        | CloudError::Command(_)
        | CloudError::Material(_)
        | CloudError::Encryption(_)
        | CloudError::RunnerProof(_)
        | CloudError::Payload(_)
        | CloudError::Head(_)
        | CloudError::HeadCrypto(_) => ProjectionError::InvalidRecord,
        _ => ProjectionError::Unavailable,
    }
}
fn observation_error(error: crate::cloud::ObservationError) -> ProjectionError {
    match error {
        crate::cloud::ObservationError::Projection(error) => error,
        _ => ProjectionError::InvalidRecord,
    }
}
impl SourceCore {
    fn marker(&self) -> Result<cache::CacheMarker, ProjectionError> {
        let progress = self
            .progress
            .lock()
            .map_err(|_| ProjectionError::Unavailable)?;
        let head = &progress.head.head;
        Ok(cache::CacheMarker {
            writer: self.binding.writer(),
            known: head_cursor(head),
            control: head.facts().control,
            deleted: head.facts().deleted,
            catalog_revision: progress.head.revision,
            catalog_root: progress.head.root,
            delivered: progress.routes.delivered(),
        })
    }
    fn chat(&self) -> Result<nocterm_hub_model::cloud::ChatCoordinate, ProjectionError> {
        let request = self.native.material().request();
        nocterm_hub_model::cloud::ChatCoordinate::new(
            self.native.cloud().namespace().clone(),
            nocterm_hub_model::cloud::RunnerCoordinate {
                id: *request.stream.runner().as_bytes(),
                key_epoch: request.stream.runner_epoch().get(),
            },
            *request.stream.chat().as_bytes(),
            request.epoch.get(),
        )
        .map_err(|_| ProjectionError::WrongSource)
    }
    fn require_work(&self, work: &AllocationLease, minimum: usize) -> Result<(), ProjectionError> {
        self.check()?;
        if work.bytes() < minimum || work.payloads() == 0 {
            Err(ProjectionError::Capacity)
        } else {
            Ok(())
        }
    }
    async fn refresh(&self, work: &heads::ProbeWork) -> Result<(), ProjectionError> {
        let next = heads::current(&self.native, &self.clock, work).await?;
        let mut previous = self
            .progress
            .lock()
            .map_err(|_| ProjectionError::Unavailable)?;
        heads::require_monotonic(&previous.head.head, &next.value().head)?;
        if next.value().head.facts().writer != Some(self.binding.writer()) {
            return Err(ProjectionError::WrongSource);
        }
        if next.value().revision < previous.head.revision
            || (next.value().revision == previous.head.revision
                && next.value().root != previous.head.root)
        {
            return Err(ProjectionError::Rollback);
        }
        previous.head = next.value().clone();
        Ok(())
    }
    async fn network<T>(
        &self,
        operation: impl std::future::Future<Output = Result<T, ProjectionError>>,
    ) -> Result<T, ProjectionError> {
        tokio::pin!(operation);
        tokio::select! {value=&mut operation=>value,_=self.ended()=>Err(ProjectionError::Stale),_=self.clock.timer().delay(Duration::from_secs(10))=>Err(ProjectionError::Unavailable)}
    }
    async fn idle(&self) -> Result<(), ProjectionError> {
        self.clock.timer().delay(Duration::from_millis(1000)).await;
        self.check()
    }
}
fn head_cursor(head: &nocterm_cloud_model::journal::CurrentChatHead) -> ProjectionCursor {
    head.facts()
        .head
        .as_ref()
        .map_or(ProjectionCursor::origin(), |anchor| {
            ProjectionCursor::at(anchor.position().event(), anchor.ciphertext())
        })
}

impl SourceCore {
    fn check_raw(&self) -> Result<(), ProjectionError> {
        self.native.check()?;
        self.clock.now().map(|_| ())
    }
    pub(super) fn check(&self) -> Result<(), ProjectionError> {
        self.lifetime.check(|| self.check_raw())
    }
    fn now(&self) -> Result<Timestamp, ProjectionError> {
        self.lifetime.check(|| {
            self.native.check()?;
            self.clock.now()
        })
    }
    pub(super) fn publish<T>(&self, action: impl FnOnce() -> T) -> Result<T, ProjectionError> {
        self.lifetime
            .publish(|| self.check_raw(), || self.native.publish(action))
    }
    pub(super) async fn ended(&self) {
        self.lifetime
            .ended(|| self.check_raw(), self.native.clone().ended())
            .await;
    }
}
