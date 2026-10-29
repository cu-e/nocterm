//! Native Cloud SDK/crypto ownership; no protocol values cross these ports.
//!
//! Persistent identity is explicit create/open, never a restart-time reset.
//! Enrollment, material installation and network composition are separate owners.
mod identity;
pub use identity::{CloudRunnerIdentity, CloudRunnerIdentityError, RunnerPublicIdentity};
mod metadata_crypto;
pub use metadata_crypto::RunnerMetadataCrypto;
mod journal_crypto;
pub use journal_crypto::{RunnerJournalCrypto, RunnerMaterialError};
mod runner_heads;
mod runner_peer;
mod runner_session_file;
mod runner_transport;
pub use runner_heads::{RunnerHeadError, RunnerHeadOutcome, RunnerHeadResponder, RunnerHeadWork};
pub use runner_peer::{RunnerHubConnection, RunnerHubError, RunnerHubPeer, RunnerHubTrust};
#[cfg(test)]
mod runner_test_support;

mod events;
pub use events::{RunnerArgumentFingerprint, RunnerEventEncoder};
mod runner_inbox;
pub use runner_inbox::{
    InboxResume, MaterialStoreError, RunnerCommandAttachments, RunnerInbox, RunnerInboxFault,
    RunnerInboxOptions, RunnerInboxPrincipal, RunnerMaterialStore, RunnerProviderBootOffer,
    RunnerSshAnswer, RunnerStagedProvider,
};
