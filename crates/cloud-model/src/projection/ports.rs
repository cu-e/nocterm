//! Consumer-specific Cloud view I/O. Native implementations supply real verified
//! replay and encrypted durable outbox/cache operations for ONE captured source.
use super::{
    ConversationProjection, ProjectionBatch, ProjectionBinding, ProjectionCursor, ProjectionError,
};
use crate::{
    authority::RunBinding,
    identity::{ChatControlRevision, CommandId, Fingerprint, Timestamp},
};
use futures::future::BoxFuture;
use nocterm_ai::conversation::ConversationAction;
use nocterm_core::allocation::{Allocated, AllocationLease};
use std::sync::Arc;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConversationCoordinates {
    pub binding: ProjectionBinding,
    pub run: Option<RunBinding>,
    pub control: ChatControlRevision,
}

/// Immutable native preparation facts; retries may never extend this window or
/// reinterpret the captured policy. These facts confer no execution authority.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ConversationPreparation {
    issued: Timestamp,
    expires: Timestamp,
    policy: Fingerprint,
}
impl ConversationPreparation {
    pub fn new(
        issued: Timestamp,
        expires: Timestamp,
        policy: Fingerprint,
    ) -> Result<Self, ProjectionError> {
        if issued.as_millis() == 0 || expires <= issued {
            return Err(ProjectionError::InvalidRecord);
        }
        Ok(Self {
            issued,
            expires,
            policy,
        })
    }
    pub fn issued(self) -> Timestamp {
        self.issued
    }
    pub fn expires(self) -> Timestamp {
        self.expires
    }
    pub fn policy(self) -> Fingerprint {
        self.policy
    }
}

/// IDs are supplied by the actual native producer, never a UI-local counter.
#[derive(Clone, Debug)]
pub struct ConversationCommand {
    pub id: CommandId,
    pub coordinates: ConversationCoordinates,
    pub action: ConversationAction,
    pub preparation: ConversationPreparation,
}
impl ConversationCommand {
    pub fn resident_bytes(&self) -> usize {
        Self::action_bytes(&self.action)
    }
    pub fn action_bytes(action: &ConversationAction) -> usize {
        4096usize.saturating_add(match action {
            ConversationAction::Prompt(text) => text.len().saturating_mul(2),
            ConversationAction::Stop => 0,
        })
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommandDelivery {
    NotFound,
    /// Admission remains unknown; this supplies no new hub storage ACK.
    Pending,
    /// Matching durable hub storage ACK; runner admission is still unknown.
    Delivered,
    /// An independently authenticated original runner admission receipt.
    Accepted,
    Rejected,
}

/// Exact allowance derived by the trusted native adapter's bounded algorithm.
/// This is a requirement, not authority to exceed the installation quota.
#[derive(Clone, Copy, Debug)]
pub struct AllocationRequirements {
    bytes: usize,
    payloads: usize,
}
impl AllocationRequirements {
    pub fn new(bytes: usize, payloads: usize) -> Self {
        Self { bytes, payloads }
    }
    pub fn bytes(self) -> usize {
        self.bytes
    }
    pub fn payloads(self) -> usize {
        self.payloads
    }
}
/// A fresh authenticated head can require a larger real allowance before any
/// record buffer is materialized. This is neither an event nor an empty page.
pub enum ConversationReplay {
    Page(Box<Allocated<ProjectionBatch>>),
    Readmission,
}
pub enum ConversationWork<'a> {
    Replay {
        after: ProjectionCursor,
        maximum_bytes: usize,
    },
    Stage {
        command: &'a ConversationCommand,
    },
    Cache {
        projection: &'a ConversationProjection,
    },
}

/// Injected authority, not a DTO-based source selector. Native capture retains the
/// original actual membership/keys; same-account re-login still ends old work.
pub trait CapturedConversationSource: Send + Sync {
    /// Preflight/count before materializing native plaintext/canonical/cipher
    /// buffers. Include actual concurrent outputs/scratch; input generations
    /// are separately charged. Workers/results retain the granted allowance.
    fn allocation_requirements(
        &self,
        work: ConversationWork<'_>,
    ) -> Result<AllocationRequirements, ProjectionError>;
    fn binding(&self) -> &ProjectionBinding;
    fn check_current(&self) -> Result<(), ProjectionError>;
    /// Resolves when this exact captured membership ends, independently of
    /// network progress or viewer visibility. A later login cannot revive it.
    fn ended(&self) -> BoxFuture<'static, ()>;
    /// Executes only this bounded synchronous publication under the ORIGINAL
    /// membership's admission. No network await or source lookup in the callback.
    fn with_current(&self, publish: &mut dyn FnMut()) -> Result<(), ProjectionError>;
    /// Wait for a bounded verified page after this cursor. Verification includes
    /// account/runner/chat/epochs, signature/decryption and retained known-head.
    /// The implementation must bound decoded buffers before materializing them.
    /// The supplied allocation travels with actual native work and its returned
    /// page until their final drop, independently of the caller future.
    fn next(
        &self,
        after: ProjectionCursor,
        allocation: AllocationLease,
    ) -> BoxFuture<'static, Result<ConversationReplay, ProjectionError>>;
    fn prepare(
        &self,
        coordinates: ConversationCoordinates,
        action: ConversationAction,
    ) -> Result<ConversationCommand, ProjectionError>;
    /// Commit exact ID/body to the encrypted durable client outbox before any send.
    /// Re-staging the same command is idempotent; changed contents are rejected.
    /// Native workers retain this charged generation, including on cancellation.
    fn stage(
        &self,
        command: Arc<Allocated<ConversationCommand>>,
        work: AllocationLease,
    ) -> BoxFuture<'static, Result<(), ProjectionError>>;
    fn deliver(
        &self,
        id: CommandId,
        work: AllocationLease,
    ) -> BoxFuture<'static, Result<CommandDelivery, ProjectionError>>;
    fn query(
        &self,
        id: CommandId,
        work: AllocationLease,
    ) -> BoxFuture<'static, Result<CommandDelivery, ProjectionError>>;
    /// Persist only to the original encrypted cache. This method must recheck its
    /// capture at physical publication, including after its own internal awaits.
    /// Actual workers retain the charged generation through their final drop.
    fn cache(
        &self,
        projection: Arc<Allocated<ConversationProjection>>,
        work: AllocationLease,
    ) -> BoxFuture<'static, Result<(), ProjectionError>>;
}
