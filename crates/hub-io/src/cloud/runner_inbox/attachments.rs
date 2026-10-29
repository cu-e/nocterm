//! Actual purpose-bound capsule/proof ownership keyed by the original command.
use super::super::{RunnerHubConnection, RunnerHubError};
use super::{commands, material_store::RunnerMaterialStore};
use nocterm_cloud_model::{
    coordinator::{CommandAcknowledgement, CoordinatorError, ProviderOffer, SessionConsent},
    identity::{CloudChatId, CommandId, Timestamp},
    journal::{JournalCatalogBinding, PublicationAllocation, PublicationWorkBudget},
    login::{LoginConsentBinding, LoginConsentWindow, LoginError, LoginProofRequest},
    tool::ToolTimer,
};
use nocterm_core::{
    SharedCause,
    allocation::{Allocated, AllocationLease},
};
use nocterm_hub_model::cloud::{
    payload as wire,
    provider::{BootDescriptor, BootWindow, ProviderLeaseContext, SignedBootDescriptor},
};
use nocterm_keys::cloud::RunnerBootKey;
use nocterm_ssh_keys::login::{
    InitialHostProof, LoginSignature, UserauthLogin, login_key_from_blob,
};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

pub(super) enum PreparedAttachment {
    Provider {
        context: ProviderLeaseContext,
        capsule: Vec<u8>,
        offer: ProviderOffer,
    },
    Ssh {
        payload: Vec<u8>,
        consent: SessionConsent,
    },
}
enum Attachment {
    Provider {
        context: ProviderLeaseContext,
        capsule: Vec<u8>,
        offer: ProviderOffer,
    },
    Ssh {
        request: LoginProofRequest,
        signature: Option<LoginSignature>,
        consent: SessionConsent,
    },
}
struct Staged {
    command: CommandAcknowledgement,
    body: Attachment,
    generation: u64,
    allocation: AllocationLease,
}
struct StageState {
    entries: BTreeMap<(CloudChatId, CommandId), Staged>,
    next: u64,
}
pub(super) struct AttachmentTicket {
    key: (CloudChatId, CommandId),
    generation: u64,
    created: bool,
}

/// Public boot facts remain opaque native data; no SDK key or generic signer
/// crosses this interface. Devices still verify the actual R attestation.
#[derive(Clone)]
pub struct RunnerProviderBootOffer {
    binding: JournalCatalogBinding,
    descriptor: SignedBootDescriptor,
}
impl RunnerProviderBootOffer {
    pub fn binding(&self) -> &JournalCatalogBinding {
        &self.binding
    }
    pub fn boot_id(&self) -> [u8; 16] {
        self.descriptor.descriptor().boot_id()
    }
    pub fn recipient(&self) -> [u8; 32] {
        self.descriptor.descriptor().recipient()
    }
    pub fn expires(&self) -> Result<Timestamp, CoordinatorError> {
        commands::timestamp(self.descriptor.descriptor().window().expires_at_ms)
    }
}

/// This same owner must be injected into the coordinator's provider and SSH
/// consent consumers. Staged bytes and their real allocation survive admission
/// until the consumer takes and drops the opaque original-command handle.
pub struct RunnerCommandAttachments {
    connection: Arc<RunnerHubConnection>,
    material: Arc<RunnerMaterialStore>,
    timer: Arc<dyn ToolTimer>,
    boot: RunnerBootKey,
    offer: RunnerProviderBootOffer,
    maximum: usize,
    work: Arc<dyn PublicationWorkBudget>,
    staged: Mutex<StageState>,
    observed: Mutex<Option<Timestamp>>,
}
impl RunnerCommandAttachments {
    pub fn new(
        connection: Arc<RunnerHubConnection>,
        material: Arc<RunnerMaterialStore>,
        timer: Arc<dyn ToolTimer>,
        maximum: usize,
        work: Arc<dyn PublicationWorkBudget>,
    ) -> Result<Arc<Self>, CoordinatorError> {
        if maximum == 0
            || maximum > 256
            || connection.binding() != material.binding()
            || !connection.shares_identity(material.identity())
        {
            return Err(CoordinatorError::InvalidCommand);
        }
        material.identity().with_keys(|_| ()).map_err(backend)?;
        let now = i64::try_from(timer.now().as_millis()).map_err(|_| CoordinatorError::Expired)?;
        let expires = now
            .checked_add(30 * 60 * 1000)
            .ok_or(CoordinatorError::Expired)?;
        let boot = RunnerBootKey::generate().map_err(backend)?;
        let descriptor = BootDescriptor::new(
            super::super::metadata_crypto::namespace(material.binding()).map_err(backend)?,
            super::super::metadata_crypto::runner(material.binding()),
            nocterm_keys::random_bytes::<16>().map_err(backend)?,
            boot.public_key().to_bytes(),
            BootWindow {
                issued_at_ms: now,
                expires_at_ms: expires,
            },
        )
        .map_err(backend)?;
        let signed = material
            .identity()
            .with_keys(|keys| keys.signing().attest_boot(descriptor))
            .map_err(backend)?
            .map_err(backend)?;
        Ok(Arc::new(Self {
            connection,
            offer: RunnerProviderBootOffer {
                binding: material.binding().clone(),
                descriptor: signed,
            },
            material,
            timer,
            boot,
            maximum,
            work,
            staged: Mutex::new(StageState {
                entries: BTreeMap::new(),
                next: 0,
            }),
            observed: Mutex::new(None),
        }))
    }
    pub fn boot_offer(&self) -> RunnerProviderBootOffer {
        self.offer.clone()
    }
    pub(super) fn shares_owners(
        &self,
        connection: &Arc<RunnerHubConnection>,
        material: &Arc<RunnerMaterialStore>,
        timer: &Arc<dyn ToolTimer>,
    ) -> bool {
        Arc::ptr_eq(&self.connection, connection)
            && Arc::ptr_eq(&self.material, material)
            && Arc::ptr_eq(&self.timer, timer)
    }
    pub(super) fn check_current(&self) -> Result<Timestamp, CoordinatorError> {
        self.connection.check_source().map_err(source_error)?;
        let now = self.timer.now();
        let mut observed = self
            .observed
            .lock()
            .map_err(|_| CoordinatorError::NativeTaskFailed)?;
        if observed.is_some_and(|old| now < old) {
            return Err(CoordinatorError::Expired);
        }
        *observed = Some(now);
        Ok(now)
    }
    pub(super) fn reserve(&self, raw_payload: usize) -> Result<AllocationLease, CoordinatorError> {
        // Original sealed/proof bytes coexist with the parsed native proof and
        // bounded metadata. This is a separate supplied owner: retained capsules
        // must not hold the coordinator's urgent command slots after admission.
        let bytes = raw_payload
            .checked_mul(2)
            .and_then(|n| n.checked_add(32 * 1024))
            .ok_or(CoordinatorError::Overloaded)?;
        self.work
            .try_reserve(PublicationAllocation::new(bytes, 1)?)
            .map_err(CoordinatorError::from)
    }
    pub(super) fn stage(
        &self,
        command: CommandAcknowledgement,
        body: PreparedAttachment,
        allocation: AllocationLease,
    ) -> Result<AttachmentTicket, CoordinatorError> {
        let now = self.check_current()?;
        self.prune(now)?;
        if !self.material.binding().contains(command.stream()) {
            return Err(CoordinatorError::StaleSource);
        }
        let body = match body {
            PreparedAttachment::Provider {
                context,
                capsule,
                offer,
            } => {
                context
                    .require_current(
                        i64::try_from(now.as_millis()).map_err(|_| CoordinatorError::Expired)?,
                    )
                    .map_err(backend)?;
                if context.boot() != self.offer.descriptor.descriptor()
                    || context.boot().recipient() != self.boot.public_key().to_bytes()
                {
                    return Err(CoordinatorError::NeedsProviderAccess);
                }
                Attachment::Provider {
                    context,
                    capsule,
                    offer,
                }
            }
            PreparedAttachment::Ssh { payload, consent } => {
                let answer = wire::CloudCommandPayload::from_canonical_bytes(
                    nocterm_hub_model::cloud::CommandKind::AnswerSshConsent,
                    &payload,
                )
                .map_err(backend)?;
                let wire::CloudCommandPayload::AnswerSshConsent(answer) = answer else {
                    return Err(CoordinatorError::InvalidCommand);
                };
                let (request, signature) = verify_ssh(&command, &consent, &answer, now)?;
                Attachment::Ssh {
                    request,
                    signature,
                    consent,
                }
            }
        };
        let mut staged = self
            .staged
            .lock()
            .map_err(|_| CoordinatorError::NativeTaskFailed)?;
        let key = key(&command);
        if let Some(original) = staged.entries.get(&key) {
            if original.command.acceptance() != command.acceptance() {
                return Err(CoordinatorError::Conflict);
            }
            return Ok(AttachmentTicket {
                key,
                generation: original.generation,
                created: false,
            });
        }
        if staged.entries.len() >= self.maximum {
            return Err(CoordinatorError::Overloaded);
        }
        staged.next = staged
            .next
            .checked_add(1)
            .ok_or(CoordinatorError::Overloaded)?;
        let generation = staged.next;
        staged.entries.insert(
            key,
            Staged {
                command,
                body,
                generation,
                allocation,
            },
        );
        Ok(AttachmentTicket {
            key,
            generation,
            created: true,
        })
    }
    pub(super) fn discard_ticket(&self, ticket: AttachmentTicket) {
        if !ticket.created {
            return;
        }
        let removed = self.staged.lock().ok().and_then(|mut state| {
            if state
                .entries
                .get(&ticket.key)
                .is_some_and(|entry| entry.generation == ticket.generation)
            {
                state.entries.remove(&ticket.key)
            } else {
                None
            }
        });
        // The supplied budget's actual keepalive may run Drop callbacks.
        drop(removed);
    }
    pub(super) fn prune(&self, now: Timestamp) -> Result<(), CoordinatorError> {
        for _ in 0..self.maximum {
            let removed = {
                let mut state = self
                    .staged
                    .lock()
                    .map_err(|_| CoordinatorError::NativeTaskFailed)?;
                let key = state
                    .entries
                    .iter()
                    .find(|(_, entry)| match &entry.body {
                        Attachment::Provider { offer, .. } => now >= offer.expires(),
                        Attachment::Ssh { consent, .. } => now >= consent.expires(),
                    })
                    .map(|(key, _)| *key);
                key.and_then(|key| state.entries.remove(&key))
            };
            let Some(entry) = removed else {
                break;
            };
            drop(entry);
        }
        Ok(())
    }
    pub fn take_provider(
        self: &Arc<Self>,
        command: &CommandAcknowledgement,
        offer: &ProviderOffer,
    ) -> Result<Option<RunnerStagedProvider>, CoordinatorError> {
        self.check_current()?;
        let mut staged = self
            .staged
            .lock()
            .map_err(|_| CoordinatorError::NativeTaskFailed)?;
        let Some(entry) = staged.entries.get(&key(command)) else {
            return Ok(None);
        };
        if entry.command.acceptance() != command.acceptance() {
            return Err(CoordinatorError::Conflict);
        }
        let Attachment::Provider {
            offer: original, ..
        } = &entry.body
        else {
            return Err(CoordinatorError::InvalidCommand);
        };
        if original.reference() != offer.reference()
            || original.context() != offer.context()
            || original.boot() != offer.boot()
            || original.provider() != offer.provider()
            || original.models() != offer.models()
            || original.expires() != offer.expires()
        {
            return Err(CoordinatorError::Conflict);
        }
        let entry = staged
            .entries
            .remove(&key(command))
            .ok_or(CoordinatorError::Conflict)?;
        drop(staged);
        let Attachment::Provider {
            context,
            capsule,
            offer,
        } = entry.body
        else {
            return Err(CoordinatorError::InvalidCommand);
        };
        Ok(Some(RunnerStagedProvider {
            owner: self.clone(),
            command: entry.command,
            context,
            capsule,
            offer,
            _allocation: entry.allocation,
        }))
    }
    pub fn take_ssh(
        self: &Arc<Self>,
        command: &CommandAcknowledgement,
        consent: &SessionConsent,
    ) -> Result<Option<RunnerSshAnswer>, CoordinatorError> {
        self.check_current()?;
        let mut staged = self
            .staged
            .lock()
            .map_err(|_| CoordinatorError::NativeTaskFailed)?;
        let Some(entry) = staged.entries.get(&key(command)) else {
            return Ok(None);
        };
        if entry.command.acceptance() != command.acceptance() {
            return Err(CoordinatorError::Conflict);
        }
        let Attachment::Ssh {
            consent: original, ..
        } = &entry.body
        else {
            return Err(CoordinatorError::InvalidCommand);
        };
        if original.run() != consent.run()
            || original.attempt() != consent.attempt()
            || original.nonce() != consent.nonce()
            || original.target() != consent.target()
            || original.authentication() != consent.authentication()
            || original.session() != consent.session()
            || original.algorithm() != consent.algorithm()
            || original.approved() != consent.approved()
            || original.expires() != consent.expires()
        {
            return Err(CoordinatorError::Conflict);
        }
        let entry = staged
            .entries
            .remove(&key(command))
            .ok_or(CoordinatorError::Conflict)?;
        drop(staged);
        let Attachment::Ssh {
            request,
            signature,
            consent,
        } = entry.body
        else {
            return Err(CoordinatorError::InvalidCommand);
        };
        Ok(Some(RunnerSshAnswer {
            owner: self.clone(),
            command: entry.command,
            request,
            signature,
            consent,
            _allocation: entry.allocation,
        }))
    }
}

pub struct RunnerStagedProvider {
    owner: Arc<RunnerCommandAttachments>,
    command: CommandAcknowledgement,
    context: ProviderLeaseContext,
    capsule: Vec<u8>,
    offer: ProviderOffer,
    _allocation: AllocationLease,
}
impl RunnerStagedProvider {
    pub fn command(&self) -> &CommandAcknowledgement {
        &self.command
    }
    pub fn offer(&self) -> &ProviderOffer {
        &self.offer
    }
    /// The capsule remains encrypted and boot-bound; this method exports no key
    /// or generic plaintext credential. Installation is a separate native owner.
    pub fn check_current(&self) -> Result<(), CoordinatorError> {
        let now = self.owner.check_current()?;
        self.context
            .require_current(i64::try_from(now.as_millis()).map_err(|_| CoordinatorError::Expired)?)
            .map_err(backend)?;
        if self.capsule.is_empty() {
            return Err(CoordinatorError::InvalidCommand);
        }
        Ok(())
    }
}

pub struct RunnerSshAnswer {
    owner: Arc<RunnerCommandAttachments>,
    command: CommandAcknowledgement,
    request: LoginProofRequest,
    signature: Option<LoginSignature>,
    consent: SessionConsent,
    _allocation: AllocationLease,
}
impl RunnerSshAnswer {
    pub fn command(&self) -> &CommandAcknowledgement {
        &self.command
    }
    pub fn answer(
        self,
        expected: &LoginProofRequest,
    ) -> Result<Allocated<Option<LoginSignature>>, LoginError> {
        let now = self
            .owner
            .check_current()
            .map_err(|error| LoginError::Credential(SharedCause::new(error)))?;
        if now >= self.consent.expires() || &self.request != expected {
            return Err(LoginError::Mismatch);
        }
        Ok(Allocated::new(self.signature, self._allocation))
    }
}
fn key(command: &CommandAcknowledgement) -> (CloudChatId, CommandId) {
    (command.stream().chat(), command.acceptance().id())
}
fn source_error(error: RunnerHubError) -> CoordinatorError {
    CoordinatorError::Backend(SharedCause::new(error))
}
fn backend(error: impl std::error::Error + Send + Sync + 'static) -> CoordinatorError {
    CoordinatorError::Backend(SharedCause::new(error))
}

fn verify_ssh(
    command: &CommandAcknowledgement,
    consent: &SessionConsent,
    answer: &wire::SshConsentAnswer<'_>,
    now: Timestamp,
) -> Result<(LoginProofRequest, Option<LoginSignature>), CoordinatorError> {
    if answer.binding.run.id != *consent.run().run().as_bytes()
        || answer.binding.run.expected_epoch != consent.run().epoch().get()
        || consent.run().stream() != *command.stream()
        || now >= consent.expires()
        || now.as_millis() < answer.binding.issued_ms
        || <[u8; 32]>::from(Sha256::digest(answer.binding.user_key_blob))
            != *consent.authentication().key().as_bytes()
        || <[u8; 32]>::from(Sha256::digest(answer.proof.host_key_blob))
            != *consent.target().host().as_bytes()
    {
        return Err(CoordinatorError::InvalidCommand);
    }
    let user = login_key_from_blob(answer.binding.user_key_blob).map_err(backend)?;
    let host = login_key_from_blob(answer.proof.host_key_blob).map_err(backend)?;
    let login = UserauthLogin::parse(answer.proof.userauth_payload.to_vec()).map_err(backend)?;
    let binding = LoginConsentBinding::new(
        consent.run().clone(),
        consent.target().clone(),
        consent.authentication().clone(),
        consent.attempt(),
        *consent.nonce().as_bytes(),
        LoginConsentWindow::new(
            Timestamp::from_millis(answer.binding.issued_ms),
            consent.expires(),
        )
        .map_err(backend)?,
        user,
    )
    .map_err(backend)?;
    let proof = InitialHostProof::new(
        answer.proof.host_key_blob.to_vec(),
        answer.proof.host_signature_blob.to_vec(),
    )
    .map_err(backend)?;
    let request = LoginProofRequest::new(binding, proof, login);
    request.verify(&host).map_err(backend)?;
    let signature = match &answer.decision {
        wire::SshConsentDecision::Deny => None,
        wire::SshConsentDecision::Approve { signature_blob } => {
            let signature = LoginSignature::new(signature_blob.to_vec()).map_err(backend)?;
            signature.verify(request.login()).map_err(backend)?;
            Some(signature)
        }
    };
    Ok((request, signature))
}
