//! Scripted host collaborators. None verifies signatures, crypto, or native I/O.
use crate::{coordinator::*, engine::*, identity::*, inference::*, journal::*, tool::*};
use futures::{FutureExt, future::BoxFuture, task::AtomicWaker};
use nocterm_core::allocation::Allocated;
use nocterm_session::testing::Gate;
use std::collections::VecDeque;
use std::sync::{
    Arc, Mutex, PoisonError,
    atomic::{AtomicBool, AtomicU64, Ordering},
};

pub struct ScriptedCommandSource {
    binding: JournalCatalogBinding,
    current: Arc<AtomicBool>,
    inbox: Arc<Mutex<ScriptedInbox>>,
    ready: Arc<AtomicWaker>,
}
#[derive(Default)]
struct ScriptedInbox {
    commands: VecDeque<ReceivedCommand>,
    opened: VecDeque<crate::authority::StreamBinding>,
    replies: Vec<(CommandAcknowledgement, CommandReply)>,
    acknowledgements: Option<Gate>,
}
impl ScriptedCommandSource {
    pub fn new(binding: JournalCatalogBinding) -> Self {
        Self {
            binding,
            current: Arc::new(AtomicBool::new(true)),
            inbox: Arc::new(Mutex::new(ScriptedInbox::default())),
            ready: Arc::new(AtomicWaker::new()),
        }
    }
    /// Fixtures already own their DTO. Native production must instead reserve
    /// before decoding; this shared fake proves actor ownership/backpressure.
    pub fn push(&self, command: ReceivedCommand) -> Result<(), CoordinatorError> {
        let mut state = self.inbox.lock().unwrap_or_else(PoisonError::into_inner);
        if state.commands.len() >= 128 {
            return Err(CoordinatorError::Overloaded);
        }
        state.commands.push_back(command);
        drop(state);
        self.ready.wake();
        Ok(())
    }
    /// Scripts the source's report that a chat's key was admitted. It carries no
    /// command, acknowledgement or control position.
    pub fn push_chat_opened(&self, stream: crate::authority::StreamBinding) {
        self.inbox
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .opened
            .push_back(stream);
        self.ready.wake();
    }
    pub fn pause_acknowledgements(&self) -> Gate {
        let gate = Gate::pending();
        self.inbox
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .acknowledgements = Some(gate.clone());
        gate
    }
    pub fn replies(&self) -> Vec<(CommandAcknowledgement, CommandReply)> {
        self.inbox
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .replies
            .clone()
    }
    pub fn revoke(&self) {
        self.current.store(false, Ordering::Release);
    }
}
impl AuthenticatedCommandSource for ScriptedCommandSource {
    fn binding(&self) -> &JournalCatalogBinding {
        &self.binding
    }
    fn check_current(&self) -> Result<(), CoordinatorError> {
        if self.current.load(Ordering::Acquire) {
            Ok(())
        } else {
            Err(CoordinatorError::StaleSource)
        }
    }
    fn receive(
        &self,
        budget: CommandReceiveBudget,
    ) -> BoxFuture<'static, Result<Option<CommandInput>, CoordinatorError>> {
        let inbox = self.inbox.clone();
        let ready = self.ready.clone();
        let current = self.current.clone();
        async move {
            loop {
                let command = futures::future::poll_fn(|cx| {
                    let mut state = inbox.lock().unwrap_or_else(PoisonError::into_inner);
                    if let Some(stream) = state.opened.pop_front() {
                        std::task::Poll::Ready(Err(stream))
                    } else if let Some(command) = state.commands.pop_front() {
                        std::task::Poll::Ready(Ok(command))
                    } else {
                        ready.register(cx.waker());
                        std::task::Poll::Pending
                    }
                })
                .await;
                if !current.load(Ordering::Acquire) {
                    return Err(CoordinatorError::StaleSource);
                }
                let command = match command {
                    Ok(command) => command,
                    Err(stream) => return Ok(Some(CommandInput::ChatOpened { stream })),
                };
                let high = command.body().high_priority();
                let lane = if high {
                    IntakeLane::Urgent
                } else {
                    IntakeLane::Ordinary
                };
                match budget.try_reserve(lane, command.reservation_bytes(), 1) {
                    Ok(lease) => {
                        return Ok(Some(CommandInput::Ready(Arc::new(Allocated::new(
                            command, lease,
                        )))));
                    }
                    Err(IntakeError::Capacity) => {
                        let ack_lane = if high {
                            IntakeLane::UrgentAcknowledgement
                        } else {
                            IntakeLane::Acknowledgement
                        };
                        let allocation = match budget.try_reserve(ack_lane, 16 * 1024, 0) {
                            Ok(allocation) => allocation,
                            Err(IntakeError::Capacity) => continue,
                            Err(error) => return Err(error.into()),
                        };
                        return Ok(Some(CommandInput::Deferred {
                            command: Box::new(command.acknowledgement()),
                            reason: RetryReason::Capacity,
                            allocation,
                        }));
                    }
                    Err(error) => return Err(error.into()),
                }
            }
        }
        .boxed()
    }
    fn acknowledge(
        &self,
        command: CommandAcknowledgement,
        reply: CommandReply,
        allocation: nocterm_core::allocation::AllocationLease,
    ) -> BoxFuture<'static, Result<(), CoordinatorError>> {
        let inbox = self.inbox.clone();
        async move {
            let _delivery_allocation = allocation;
            let gate = {
                let mut state = inbox.lock().unwrap_or_else(PoisonError::into_inner);
                if state.replies.len() >= 1024 {
                    return Err(CoordinatorError::Overloaded);
                }
                state.replies.push((command, reply));
                state.acknowledgements.clone()
            };
            if let Some(gate) = gate {
                gate.wait().await;
            }
            Ok(())
        }
        .boxed()
    }
}
pub struct ScriptedCoordinatorIdentity {
    next: AtomicU64,
}
impl Default for ScriptedCoordinatorIdentity {
    fn default() -> Self {
        Self {
            next: AtomicU64::new(1),
        }
    }
}
impl CoordinatorIdentity for ScriptedCoordinatorIdentity {
    fn fresh(&self) -> Result<[u8; 16], CoordinatorError> {
        let number = self.next.fetch_add(1, Ordering::Relaxed);
        let mut bytes = [0; 16];
        bytes[..8].copy_from_slice(&number.to_be_bytes());
        Ok(bytes)
    }
}
pub struct ScriptedCoordinatorProviders {
    selection: ModelSelection,
    factory: Arc<dyn AgentEngineFactory>,
    pricing: Arc<dyn ModelPricing>,
    current: AtomicBool,
}
impl ScriptedCoordinatorProviders {
    pub fn new(
        selection: ModelSelection,
        factory: Arc<dyn AgentEngineFactory>,
        pricing: Arc<dyn ModelPricing>,
    ) -> Self {
        Self {
            selection,
            factory,
            pricing,
            current: AtomicBool::new(true),
        }
    }
    pub fn revoke(&self) {
        self.current.store(false, Ordering::Release);
    }
    /// Access arrives again, as when a host hands a managed runner a new token.
    pub fn restore(&self) {
        self.current.store(true, Ordering::Release);
    }
}
impl CoordinatorProviders for ScriptedCoordinatorProviders {
    fn model(&self, selection: &ModelSelection) -> Result<ConfiguredModel, EngineError> {
        if !self.current.load(Ordering::Acquire) {
            return Err(EngineError::NeedsProviderAccess);
        }
        if selection != &self.selection {
            return Err(EngineError::UnsupportedFormat);
        }
        ConfiguredModel::new(
            self.selection.clone(),
            self.factory.clone(),
            self.pricing.clone(),
        )
        .map_err(|_| EngineError::UnsupportedFormat)
    }
    fn install(
        &self,
        _command: CommandAcknowledgement,
        _offer: ProviderOffer,
    ) -> BoxFuture<'static, Result<(), CoordinatorError>> {
        futures::future::ready(Err(CoordinatorError::Unsupported)).boxed()
    }
    fn revoke(&self, _reference: GrantReference) {
        self.revoke();
    }
}
#[derive(Default)]
pub struct UnavailableRunTools;
impl RunToolRuntimeFactory for UnavailableRunTools {
    fn attach(
        &self,
        _run: crate::authority::RunBinding,
        _receipt: crate::authority::SessionReceipt,
        _authority: Arc<dyn ToolAuthority>,
    ) -> BoxFuture<'static, Result<Arc<dyn RunToolRuntime>, ToolExecutionError>> {
        futures::future::ready(Err(ToolExecutionError::NeedsUnlock)).boxed()
    }
}
#[derive(Default)]
pub struct RefusedSessionConsent;
impl SessionConsentConsumer for RefusedSessionConsent {
    fn answer(
        &self,
        _command: CommandAcknowledgement,
        _answer: SessionConsent,
    ) -> BoxFuture<'static, Result<Option<crate::authority::SessionReceipt>, CoordinatorError>>
    {
        futures::future::ready(Err(CoordinatorError::NeedsUnlock)).boxed()
    }
}
/// Constant fixture identity, deliberately not a canonical argument hash.
#[derive(Default)]
pub struct FixtureArgumentFingerprint;
impl ArgumentFingerprint for FixtureArgumentFingerprint {
    fn fingerprint(
        &self,
        _call: &nocterm_ai::TerminalCall,
    ) -> Result<Fingerprint, ToolExecutionError> {
        Ok(Fingerprint::from_bytes([0x71; 32]))
    }
}
#[derive(Default)]
pub struct ObservedCoordinatorEvents {
    events: Mutex<Vec<CoordinatorEvent>>,
    next: AtomicU64,
    refuse_progress: AtomicBool,
}
impl ObservedCoordinatorEvents {
    /// Refuse one progress publication while leaving terminal status publication available.
    pub fn fail_next_progress(&self) {
        self.refuse_progress.store(true, Ordering::Release);
    }
    pub fn events(&self) -> Vec<CoordinatorEvent> {
        self.events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}
impl CoordinatorEventEncoder for ObservedCoordinatorEvents {
    fn encode(
        &self,
        lease: &WriterLease,
        next: JournalPosition,
        event: &CoordinatorEvent,
        work: &dyn PublicationWorkBudget,
    ) -> Result<ResidentPublication, JournalError> {
        if matches!(event, CoordinatorEvent::Progress { .. })
            && self.refuse_progress.swap(false, Ordering::AcqRel)
        {
            return Err(JournalError::Capacity);
        }
        self.events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(event.clone());
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let mut bytes = [0; 16];
        bytes[..8].copy_from_slice(&id.to_be_bytes());
        let run = match event {
            CoordinatorEvent::CommandAccepted { run, .. } => run.clone(),
            CoordinatorEvent::PromptAccepted { run, .. }
            | CoordinatorEvent::Status { run, .. }
            | CoordinatorEvent::Progress { run, .. }
            | CoordinatorEvent::ToolResult { run, .. } => Some(run.clone()),
            CoordinatorEvent::ToolIntent { intent, .. }
            | CoordinatorEvent::Approval { intent, .. } => Some(intent.authority().run().clone()),
            CoordinatorEvent::ApprovalResolved { answer, .. } => {
                Some(answer.authority().run().clone())
            }
            CoordinatorEvent::BackgroundLifecycle(value) => {
                Some(value.job().execution().run().clone())
            }
            CoordinatorEvent::AuthenticationRequested { request, .. } => {
                Some(request.binding().run().clone())
            }
            CoordinatorEvent::SessionState(value) => Some(value.binding().run().clone()),
            CoordinatorEvent::Deleted { .. } | CoordinatorEvent::SshAccess(_) => None,
        };
        fixture_outbox_with_work(
            lease,
            next,
            EventId::from_bytes(bytes),
            run,
            crate::payload::OpaqueBytes::new(vec![0x74])?,
            work,
        )
    }
}

/// Explicit native publication metadata fixture; neither signature nor crypto.
pub fn fixture_outbox(
    lease: &WriterLease,
    next: JournalPosition,
    id: EventId,
    run: Option<crate::authority::RunBinding>,
    sealed: crate::payload::OpaqueBytes,
) -> Result<ResidentPublication, JournalError> {
    let work = fixture_publication_budget();
    fixture_outbox_with_work(lease, next, id, run, sealed, &work)
}
/// Test-only bounded owner: each fixture owns real finite generation permits.
pub fn fixture_publication_budget() -> PublicationBudget {
    PublicationBudget::maintenance(
        crate::coordinator::IntakeLimits::default().pool(crate::coordinator::IntakeLane::Ordinary),
    )
}
pub fn fixture_outbox_with_work(
    lease: &WriterLease,
    next: JournalPosition,
    id: EventId,
    run: Option<crate::authority::RunBinding>,
    sealed: crate::payload::OpaqueBytes,
    work: &dyn PublicationWorkBudget,
) -> Result<ResidentPublication, JournalError> {
    use sha2::{Digest, Sha256};
    let digest = Fingerprint::from_bytes(Sha256::digest(sealed.as_bytes()).into());
    let position = HeadRecordPosition::new(
        lease.epoch(),
        next.events(),
        next.records(),
        next.revision(),
    )?;
    let allocation = work.try_reserve(PublicationAllocation::new(
        ResidentPublication::required_bytes(sealed.len())?,
        1,
    )?)?;
    let mut delivery = *id.as_bytes();
    delivery[0] ^= 0xd1;
    ResidentPublication::new(
        lease.stream().clone(),
        OutboxEntry::new(
            HeadAnchor::new(HeadAnchorKind::Event, position, id, run, digest),
            ChatKeyEpoch::new(7)?,
            OutboxDeliveryId::from_bytes(delivery),
            sealed,
        ),
        allocation,
    )
}
