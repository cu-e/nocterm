//! Native Cloud SDK/crypto ownership; no protocol values cross these ports.
//!
//! Persistent identity is explicit create/open, never a restart-time reset.
//! Enrollment, material installation and network composition are separate owners.
//! Identity, session, material and cursor files share private snapshot locking and
//! durable publication; source binding and codecs remain with their native owners.
mod identity;
pub use identity::{CloudRunnerIdentity, CloudRunnerIdentityError, RunnerPublicIdentity};
mod metadata_crypto;
pub use metadata_crypto::RunnerMetadataCrypto;
mod journal_crypto;
pub use journal_crypto::{RunnerJournalCrypto, RunnerMaterialError};
mod private_snapshot;
mod runner_heads;
mod runner_peer;
mod runner_reclaim;
mod runner_session_file;
mod runner_transport;
pub use runner_heads::{RunnerHeadError, RunnerHeadOutcome, RunnerHeadResponder, RunnerHeadWork};
pub use runner_peer::{
    RunnerCredentialDeadlines, RunnerHubConnection, RunnerHubError, RunnerHubPeer, RunnerHubTrust,
};
pub use runner_reclaim::{ChatRetirementOutcome, RunnerReclaimError, retire_until_compacted};
mod runner_directory;
pub use runner_directory::{RunnerChatDirectory, RunnerChatHost, RunnerDirectoryError};
mod runner_horizons;
mod runner_ssh_certificates;
pub use runner_ssh_certificates::RunnerSshCertificates;
mod runner_retirement;
pub use runner_retirement::RunnerChatRetirement;
#[cfg(test)]
mod runner_test_support;

mod events;
pub use events::{RunnerArgumentFingerprint, RunnerEventEncoder};
mod runner_inbox;
pub use runner_inbox::{
    BootAnnouncement, InboxResume, MaterialStoreError, RunnerCommandAttachments, RunnerInbox,
    RunnerInboxFault, RunnerInboxOptions, RunnerInboxPrincipal, RunnerMaterialStore,
    RunnerProviderBootOffer, RunnerProviderCredentialSource, RunnerSshAnswer, RunnerStagedProvider,
};

pub(crate) mod account_material;
pub use account_material::{
    ACCOUNT_MATERIAL_RESIDENT_BYTES, ACCOUNT_MATERIAL_WORK_BYTES, AccountMaterialError,
    AccountMaterialFailure, AccountMaterialReservations, ChatMaterialPins, CloudAccountMaterial,
    account_material_work_bytes,
};

mod runner_enrolment;
pub use runner_enrolment::{EnrolmentError, enrol};

mod runner_registry;
pub use runner_registry::{
    AccountRunnerRegistry, RUNNER_REGISTRY_RESIDENT_BYTES, RunnerRegistration, RunnerRegistryError,
    RunnerRegistryReservations, TrustedRunnerRoster, runner_registry_work_bytes,
};
pub(crate) mod hub_trust;
pub use hub_trust::{
    AccountCloudTrust, CLOUD_TRUST_RESIDENT_BYTES, CloudRunnerSetup, CloudTrustError,
    CloudTrustReservations, TrustedCloudHub, cloud_trust_work_bytes,
};

mod provider_boot;
pub use provider_boot::{ProviderBootError, provider_boot_work_bytes};
