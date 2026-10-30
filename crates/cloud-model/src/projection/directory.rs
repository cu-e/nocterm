//! Account-captured discovery of existing conversations. Plain row metadata never
//! grants authority; only the opaque original entry can open its verified source.
use super::{ProjectionError, ports::{AllocationRequirements, CapturedConversationSource}};
use futures::future::BoxFuture;
use nocterm_core::allocation::AllocationLease;
use std::sync::Arc;

/// Captures synchronously before a directory request. A later sign-in creates a
/// different owner; it cannot revive pages or actions returned by this capture.
pub trait ConversationDirectoryProvider: Send + Sync {
    fn capture_requirements(&self) -> AllocationRequirements;
    fn capture(&self, resident: AllocationLease)
        -> Result<Arc<dyn CapturedConversationDirectory>, ProjectionError>;
}
pub trait CapturedConversationDirectory: Send + Sync {
    fn check_current(&self) -> Result<(), ProjectionError>;
    fn ended(&self) -> BoxFuture<'static, ()>;
    fn work_requirements(&self) -> Result<AllocationRequirements, ProjectionError>;
    fn page_requirements(&self) -> AllocationRequirements;
    fn first(&self, work: AllocationLease, resident: AllocationLease)
        -> BoxFuture<'static, Result<Arc<dyn ConversationDirectoryPage>, ProjectionError>>;
}
/// One bounded, authenticated catalog page. Continuation retains the actual
/// original revision/cursor proof. Callers must drop old pages to reclaim quota.
pub trait ConversationDirectoryPage: Send + Sync {
    fn entries(&self) -> Result<&[Arc<dyn ConversationDirectoryEntry>], ProjectionError>;
    fn has_more(&self) -> bool;
    fn next(&self, work: AllocationLease, resident: AllocationLease)
        -> BoxFuture<'static, Result<Arc<dyn ConversationDirectoryPage>, ProjectionError>>;
}
pub trait ConversationDirectoryEntry: Send + Sync {
    fn label(&self) -> &str;
    fn check_current(&self) -> Result<(), ProjectionError>;
    fn work_requirements(&self) -> Result<AllocationRequirements, ProjectionError>;
    fn resident_requirements(&self) -> AllocationRequirements;
    fn open(&self, work: AllocationLease, resident: AllocationLease)
        -> BoxFuture<'static, Result<Arc<dyn CapturedConversationSource>, ProjectionError>>;
}
