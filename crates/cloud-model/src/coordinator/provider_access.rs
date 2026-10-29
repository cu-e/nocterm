//! A trusted provider consumer receives one original boot-bound credential.
use super::{CommandAcknowledgement, CoordinatorError, ProviderOffer};
use crate::journal::JournalCatalogBinding;
use nocterm_core::allocation::AllocationLease;

/// The actual native capsule owner verifies the original command, account,
/// runner boot and finite context before synchronously invoking the receiver.
/// Neither model code nor a tool receives this collaborator.
pub trait ProviderCredentialSource: Send + Sync {
    fn binding(&self) -> &JournalCatalogBinding;
    fn check_current(&self) -> Result<(), CoordinatorError>;
    fn consume(
        &self,
        command: &CommandAcknowledgement,
        offer: &ProviderOffer,
        receiver: &mut dyn ProviderCredentialReceiver,
    ) -> Result<(), CoordinatorError>;
}

/// The trusted HTTP adapter copies only into its finite zeroizing memory lease.
/// It retains this SAME actual reservation with the copied credential, including
/// active requests and cancellation. The borrowed input has no durable lifetime.
/// Implementations must reject a second invocation for the same installation.
pub trait ProviderCredentialReceiver {
    fn install(
        &mut self,
        secret: &[u8],
        allocation: AllocationLease,
    ) -> Result<(), CoordinatorError>;
}
