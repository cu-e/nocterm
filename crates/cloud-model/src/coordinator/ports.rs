use super::{
    CommandAcknowledgement, CommandInput, CommandReceiveBudget, CommandReply, CoordinatorError,
    CoordinatorEvent, ProviderOffer, SessionConsent,
};
use crate::{
    authority::SessionReceipt,
    engine::{AgentEngineFactory, EngineError},
    inference::{ModelPricing, ModelSelection},
    journal::*,
    tool::BackgroundCommandJournal,
};
use futures::future::BoxFuture;
use std::sync::Arc;

/// Trusted authenticated typed producer, never passed to an engine or model.
/// It owns sole public wire verification/payload mapping and current full pins;
/// native command constructors alone do not establish those guarantees.
pub trait AuthenticatedCommandSource: Send + Sync {
    fn binding(&self) -> &JournalCatalogBinding;
    fn check_current(&self) -> Result<(), CoordinatorError>;
    /// Reserve from the supplied real budget before ciphertext/decrypt/DTO
    /// allocation. Normal pressure cannot consume urgent capacity. Advancing a
    /// bounded scan must retain/revisit unacknowledged holes without advancing
    /// the durable ACK watermark across them; no unbounded hole list is allowed.
    /// Empty replay/idle waits internally. `None` means authoritative permanent
    /// end-of-source; source failures return an error and fence active work.
    fn receive(
        &self,
        budget: CommandReceiveBudget,
    ) -> BoxFuture<'static, Result<Option<CommandInput>, CoordinatorError>>;
    /// Publishes a bounded native admission response. This is not permission to
    /// final-ACK arbitrary errors: RetryLater always leaves the inbox unacked;
    /// Refused requires a durable rejection or explicit safe native policy.
    /// The actor supplies its actual bounded ACK metadata reservation. Native
    /// implementations retain that same owner through queued SDK request/response
    /// work and cancellation; they must not reserve again from a full ACK lane.
    fn acknowledge(
        &self,
        command: CommandAcknowledgement,
        reply: CommandReply,
        allocation: nocterm_core::allocation::AllocationLease,
    ) -> BoxFuture<'static, Result<(), CoordinatorError>>;
}

pub trait CoordinatorJournal:
    RunJournalWriter
    + JournalReader
    + JournalSnapshotWriter
    + JournalControlReader
    + EngineCheckpointReader
    + JournalWriterFence
    + JournalCatalogReader
    + TargetEffectReader
    + BackgroundCommandJournal
{
}
impl<T> CoordinatorJournal for T where
    T: RunJournalWriter
        + JournalReader
        + JournalSnapshotWriter
        + JournalControlReader
        + EngineCheckpointReader
        + JournalWriterFence
        + JournalCatalogReader
        + TargetEffectReader
        + BackgroundCommandJournal
{
}

pub trait CoordinatorEventEncoder: Send + Sync {
    fn encode(
        &self,
        lease: &WriterLease,
        next: JournalPosition,
        event: &CoordinatorEvent,
        work: &dyn PublicationWorkBudget,
    ) -> Result<ResidentPublication, JournalError>;
}

/// Immutable configured collaborators. The factory still checks its captured
/// actual account/provider/model lease when constructing an engine.
pub struct ConfiguredModel {
    selection: ModelSelection,
    factory: Arc<dyn AgentEngineFactory>,
    pricing: Arc<dyn ModelPricing>,
}
impl ConfiguredModel {
    pub fn new(
        selection: ModelSelection,
        factory: Arc<dyn AgentEngineFactory>,
        pricing: Arc<dyn ModelPricing>,
    ) -> Result<Self, CoordinatorError> {
        if pricing.selection() != &selection {
            return Err(CoordinatorError::InvalidCommand);
        }
        Ok(Self {
            selection,
            factory,
            pricing,
        })
    }
    pub fn selection(&self) -> &ModelSelection {
        &self.selection
    }
    pub fn factory(&self) -> &Arc<dyn AgentEngineFactory> {
        &self.factory
    }
    pub fn pricing(&self) -> &Arc<dyn ModelPricing> {
        &self.pricing
    }
}
pub trait CoordinatorProviders: Send + Sync {
    fn model(&self, selection: &ModelSelection) -> Result<ConfiguredModel, EngineError>;
    /// Consumes the exact original staged boot-bound capsule only after durable
    /// admission. Logged ciphertext is never reopened under another boot.
    fn install(
        &self,
        command: CommandAcknowledgement,
        offer: ProviderOffer,
    ) -> BoxFuture<'static, Result<(), CoordinatorError>>;
    fn revoke(&self, reference: super::GrantReference);
}
pub trait SessionConsentConsumer: Send + Sync {
    /// The staged purpose-specific proof stays with this owner, not an engine.
    fn answer(
        &self,
        command: CommandAcknowledgement,
        answer: SessionConsent,
    ) -> BoxFuture<'static, Result<Option<SessionReceipt>, CoordinatorError>>;
}
pub trait CoordinatorIdentity: Send + Sync {
    fn fresh(&self) -> Result<[u8; 16], CoordinatorError>;
}
