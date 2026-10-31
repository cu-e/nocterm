//! Pure desktop replay rules and ports for an originally captured verified source.
//! A source implementation owns authentication, signatures, decryption and durable
//! encrypted client outbox/cache I/O. Coordinate constructors confer no trust.
mod approvals;
mod intent;
mod provider_access;
pub use approvals::{ApprovalKey, ProjectedApproval};
pub use intent::{ApprovalRequest, ConversationIntent};
pub use provider_access::ProviderAccess;
mod baseline;
pub use baseline::{SnapshotBaseline, SnapshotFact};
mod cursor;
mod event;
pub mod ports;
mod reducer;
mod runs;
mod sizing;
mod tools;
mod usage;
pub use cursor::{ProjectionBinding, ProjectionCursor};
pub use event::{ProjectionBatch, ProjectionEvent};
pub use reducer::ConversationProjection;

pub const MAX_REPLAY_EVENTS: usize = 64;
pub const MAX_PROJECTION_ENTRIES: usize = 2_000;
pub const MAX_PENDING_COMMANDS: usize = 32;
/// Maximum retained semantic page, including decoded metadata and JSON nodes.
/// Native decoders enforce this bound before allocating their output page.
pub const MAX_REPLAY_BYTES: usize = 8 * 1024 * 1024;

#[cfg(any(test, feature = "test-support"))]
pub mod testing;
#[cfg(test)]
mod tests;

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ProjectionError {
    #[error("The original authenticated Cloud source has ended.")]
    Stale,
    #[error("The verified event belongs to another account, runner, chat or key epoch.")]
    WrongSource,
    #[error("The event chain has a gap or a conflicting duplicate.")]
    Discontinuous,
    #[error("The event stream or control revision moved backwards.")]
    Rollback,
    #[error("The event does not belong to the displayed run.")]
    WrongRun,
    #[error("The bounded Cloud projection or pending queue is full.")]
    Capacity,
    #[error("The original command outcome is unknown; retry its original ID.")]
    OutcomeUnknown,
    #[error("The captured Cloud transport is unavailable.")]
    Unavailable,
    #[error("The signed observation uses semantics unsupported by this client.")]
    Unsupported,
    #[error("The verified projection contains an invalid tool or content record.")]
    InvalidRecord,
    #[error("The encrypted client outbox or projection cache could not be persisted.")]
    Persistence,
}
