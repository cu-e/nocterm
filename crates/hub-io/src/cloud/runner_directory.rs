//! Signs, seals and publishes the runner's chat summaries to its directory stream
//! (ADR 0135, contract ADR 0008).
//!
//! The coordinator hands over neutral drafts through [`ChatSummaryDirectory`]. This adapter
//! reads the chat's current head (the same facts a head witness reports), maps the draft to
//! the contract's `ChatSummary` with the title cut by `title_from_prompt`, signs it with the
//! runner signing key and seals it from the runner encryption key to the pinned account key.
//! Publication uses the summary's deterministic publication id, so a retry of a staged
//! revision is idempotent at the hub. A hub without capability bit 256 is reported as
//! unavailable and nothing is sealed.
mod mapping;
use super::{
    CloudRunnerIdentity, CloudRunnerIdentityError, RunnerHubPeer, metadata_crypto,
    runner_transport::{native_error, storage_error, transport_error},
};
use mapping::SummarySealer;
use nocterm_cloud_model::journal::{
    ChatSummaryDirectory, ChatSummaryDraft, CurrentCloudHeads, JournalError, JournalFuture,
    PublicationAllocation, SealedChatSummary,
};
use nocterm_core::SharedCause;
use nocterm_hub_client::cloud::CloudUpload;
use nocterm_hub_model::cloud::{
    ChatCoordinate, CloudTextId,
    summary::{
        SUMMARY_FEATURE_PROVIDER_LEASE, SUMMARY_FEATURE_SSH_BINDING, SummaryContext, SummaryHost,
    },
};
use sha2::{Digest, Sha256};
use std::sync::Arc;

#[derive(Debug, thiserror::Error)]
pub enum RunnerDirectoryError {
    #[error("the directory's identity, hub connection or account differ from the runner's pins")]
    Binding,
    #[error("the pinned account public key is invalid")]
    AccountKey(#[source] SharedCause),
    #[error("the managed agent id is not a valid Cloud id")]
    AgentId(#[source] SharedCause),
    #[error(transparent)]
    Identity(#[from] CloudRunnerIdentityError),
}

/// Where this runner's chats run, as their summaries name it.
#[derive(Clone, Debug)]
pub struct RunnerChatHost {
    host: SummaryHost,
    features: u32,
}
impl RunnerChatHost {
    /// The person's own runner, which takes own-provider leases.
    pub fn own() -> Self {
        Self {
            host: SummaryHost::Own,
            features: SUMMARY_FEATURE_PROVIDER_LEASE,
        }
    }
    /// A managed agent's seat; its provider is brokered by the pool host.
    pub fn managed(agent_id: &str) -> Result<Self, RunnerDirectoryError> {
        Ok(Self {
            host: SummaryHost::Managed {
                agent_id: CloudTextId::new(agent_id)
                    .map_err(|error| RunnerDirectoryError::AgentId(SharedCause::new(error)))?,
            },
            features: 0,
        })
    }
    /// This runner takes chat SSH bindings (it states egress): its summaries say so and
    /// list a bound chat's servers (contract ADR 0011, ADR 0139).
    pub fn with_ssh_bindings(mut self) -> Self {
        self.features |= SUMMARY_FEATURE_SSH_BINDING;
        self
    }
}

/// The runner's directory on its hub, bound to the runner's identity and account.
pub struct RunnerChatDirectory {
    peer: RunnerHubPeer,
    sealer: Arc<SummarySealer>,
}
impl RunnerChatDirectory {
    pub fn new(
        peer: RunnerHubPeer,
        identity: Arc<CloudRunnerIdentity>,
        account: [u8; 32],
        heads: Arc<dyn CurrentCloudHeads>,
        host: RunnerChatHost,
    ) -> Result<Self, RunnerDirectoryError> {
        let connection = peer.connection();
        if !connection.shares_identity(&identity) || identity.binding() != connection.binding() {
            return Err(RunnerDirectoryError::Binding);
        }
        let sealer = SummarySealer::new(identity, account, heads, host)?;
        Ok(Self {
            peer,
            sealer: Arc::new(sealer),
        })
    }
}

impl ChatSummaryDirectory for RunnerChatDirectory {
    fn available(&self) -> bool {
        self.peer
            .connection()
            .runner()
            .is_ok_and(|runner| runner.supports_chat_directory())
    }
    fn seal(&self, draft: ChatSummaryDraft) -> JournalFuture<SealedChatSummary> {
        let sealer = self.sealer.clone();
        Box::pin(async move { sealer.seal(draft).await })
    }
    fn publish(&self, sealed: SealedChatSummary) -> JournalFuture<()> {
        let connection = self.peer.connection().clone();
        Box::pin(async move {
            if !connection.binding().contains(sealed.stream()) {
                return Err(JournalError::Fenced);
            }
            let binding = connection.binding().clone();
            let context = SummaryContext::new(
                ChatCoordinate::new(
                    metadata_crypto::namespace(&binding)?,
                    metadata_crypto::runner(&binding),
                    *sealed.stream().chat().as_bytes(),
                    sealed.key_epoch().get(),
                )
                .map_err(storage_error)?,
                sealed.revision(),
            )
            .map_err(storage_error)?;
            let bytes = sealed.ciphertext().as_bytes();
            let _work = connection.work().try_reserve(PublicationAllocation::new(
                CloudUpload::buffer_work_bytes(bytes.len()).map_err(transport_error)?,
                1,
            )?)?;
            let runner = connection.runner().map_err(native_error)?;
            let receipt = runner
                .publish_summary(&context, bytes)
                .await
                .map_err(transport_error)?;
            connection.runner().map_err(native_error)?;
            let id = context.publication_id().map_err(storage_error)?;
            let digest: [u8; 32] = Sha256::digest(bytes).into();
            if receipt.publication_id.as_slice() != id
                || receipt.ciphertext_sha256.as_slice() != digest
                || receipt.cursor == 0
            {
                return Err(JournalError::Corrupt);
            }
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests;
