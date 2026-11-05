//! Neutral drafts mapped to the contract summary, signed and sealed to the account.
use super::{RunnerChatHost, RunnerDirectoryError};
use crate::cloud::{CloudRunnerIdentity, metadata_crypto, runner_transport::storage_error};
use nocterm_cloud_model::{
    identity::Timestamp,
    journal::{
        ChatActivity, ChatSummaryDraft, CurrentChatHead, CurrentCloudHeads, JournalError,
        SealedChatSummary, SummarySsh,
    },
    payload::OpaqueBytes,
};
use nocterm_core::SharedCause;
use nocterm_hub_model::cloud::{
    ChatCoordinate, CloudTextId,
    ssh::SignedSshBinding,
    summary::{
        BoundServer, ChatState, ChatSummary, SummaryContext, SummaryHead, SummaryModel,
        bound_servers, title_from_prompt,
    },
};
use nocterm_keys::{PublicKey, cloud::seal_summary};
use sha2::{Digest, Sha256};
use std::sync::Arc;

pub(super) struct SummarySealer {
    identity: Arc<CloudRunnerIdentity>,
    account: PublicKey,
    heads: Arc<dyn CurrentCloudHeads>,
    host: RunnerChatHost,
}
impl SummarySealer {
    pub(super) fn new(
        identity: Arc<CloudRunnerIdentity>,
        account: [u8; 32],
        heads: Arc<dyn CurrentCloudHeads>,
        host: RunnerChatHost,
    ) -> Result<Self, RunnerDirectoryError> {
        if <[u8; 32]>::from(Sha256::digest(account))
            != *identity.binding().scope().account().key().as_bytes()
        {
            return Err(RunnerDirectoryError::Binding);
        }
        let account = PublicKey::from_bytes(&account)
            .map_err(|error| RunnerDirectoryError::AccountKey(SharedCause::new(error)))?;
        identity.public_identity()?;
        Ok(Self {
            identity,
            account,
            heads,
            host,
        })
    }
    pub(super) async fn seal(
        &self,
        draft: ChatSummaryDraft,
    ) -> Result<SealedChatSummary, JournalError> {
        let binding = self.identity.binding().clone();
        if !binding.contains(draft.stream()) {
            return Err(JournalError::Fenced);
        }
        let heads = self.heads.clone();
        let stream = draft.stream().clone();
        // The native head read is blocking SQLite work.
        let head =
            tokio::task::spawn_blocking(move || futures::executor::block_on(heads.chat(stream)))
                .await
                .map_err(storage_error)??
                .ok_or(JournalError::NotFound)?;
        if head.stream() != draft.stream() {
            return Err(JournalError::Corrupt);
        }
        let context = SummaryContext::new(
            ChatCoordinate::new(
                metadata_crypto::namespace(&binding)?,
                metadata_crypto::runner(&binding),
                *head.stream().chat().as_bytes(),
                head.key_epoch().get(),
            )
            .map_err(storage_error)?,
            draft.revision(),
        )
        .map_err(storage_error)?;
        let summary = self.summary(&draft, &head, millis(draft.at())?)?;
        let sealed = self
            .identity
            .with_keys(|keys| {
                let signed = keys
                    .signing()
                    .attest_summary(context, summary)
                    .map_err(storage_error)?;
                seal_summary(keys.encryption(), &self.account, &signed).map_err(storage_error)
            })
            .map_err(storage_error)??;
        SealedChatSummary::new(
            draft.stream().clone(),
            head.key_epoch(),
            draft.revision(),
            draft.activity() == ChatActivity::Deleted,
            OpaqueBytes::new(sealed)?,
        )
    }
    fn summary(
        &self,
        draft: &ChatSummaryDraft,
        head: &CurrentChatHead,
        now_ms: i64,
    ) -> Result<ChatSummary, JournalError> {
        let facts = head.facts();
        let model = draft
            .model()
            .map(|model| {
                Ok::<_, JournalError>(SummaryModel {
                    provider: CloudTextId::new(model.provider().as_str()).map_err(storage_error)?,
                    model: CloudTextId::new(model.model().as_str()).map_err(storage_error)?,
                })
            })
            .transpose()?;
        Ok(ChatSummary {
            head: facts.head.as_ref().map(|anchor| SummaryHead {
                sequence: anchor.position().event().get(),
                ciphertext_digest: *anchor.ciphertext().as_bytes(),
            }),
            control_revision: facts.control.get(),
            created_at_ms: millis(draft.created_at())?,
            last_activity_ms: millis(draft.last_activity())?,
            title: title_from_prompt(draft.prompt_line().unwrap_or_default()),
            host: self.host.host.clone(),
            model,
            state: state(draft.activity()),
            pending_approvals: draft.pending_approvals(),
            servers: draft
                .ssh()
                .map_or(Ok(Vec::new()), |ssh| servers(ssh, now_ms))?,
            features: self.host.features,
        })
    }
}

/// The bound servers of the chat's current binding with the state its last report gives
/// them (contract ADR 0011, ADR 0139).
fn servers(ssh: &SummarySsh, now_ms: i64) -> Result<Vec<BoundServer>, JournalError> {
    let signed =
        SignedSshBinding::from_canonical_bytes(ssh.binding.as_bytes()).map_err(storage_error)?;
    let update = crate::cloud::events::ssh::update(&ssh.report)?;
    Ok(bound_servers(signed.binding(), &update, now_ms))
}

fn millis(at: Timestamp) -> Result<i64, JournalError> {
    i64::try_from(at.as_millis()).map_err(|_| JournalError::Capacity)
}

pub(super) fn state(activity: ChatActivity) -> ChatState {
    match activity {
        ChatActivity::Idle => ChatState::Idle,
        ChatActivity::Running => ChatState::Running,
        ChatActivity::WaitingApproval => ChatState::WaitingApproval,
        ChatActivity::NeedsGrant => ChatState::NeedsGrant,
        ChatActivity::NeedsProvider => ChatState::NeedsProvider,
        ChatActivity::NeedsSsh => ChatState::NeedsSsh,
        ChatActivity::Failed => ChatState::Failed,
        ChatActivity::Deleted => ChatState::Deleted,
    }
}
