//! Authenticated native outbox plaintext; public binding codecs remain SDK-owned.
use super::storage::SealedCell;
use nocterm_cloud_model::{
    identity::{CommandId, Fingerprint, WriterEpoch},
    projection::{ProjectionError, RecordKind, ports::ConversationCommand},
};
use nocterm_hub_model::cloud::{
    ChatCoordinate, JournalClass, JournalCoordinate,
    relay::{MAX_RELAY_BINDING_BYTES, PublicationMetadata, RelayBinding},
};
use nocterm_keys::cloud::ChatKey;
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

const MAGIC: &[u8; 8] = b"NCDOM001";
const FIXED: usize = 8 + 32 + 32 + 8 + 16 + 4 + 4;

pub(super) struct StoredCommand {
    pub intent: Fingerprint,
    pub semantic: [u8; 32],
    pub control: u64,
    pub metadata: PublicationMetadata,
    pub ciphertext: Vec<u8>,
}
impl StoredCommand {
    pub(super) fn seal(
        &self,
        key: &ChatKey,
        chat: ChatCoordinate,
        id: CommandId,
        writer: WriterEpoch,
    ) -> Result<Vec<u8>, ProjectionError> {
        let binding = self
            .metadata
            .binding()
            .canonical_bytes()
            .map_err(|_| ProjectionError::InvalidRecord(RecordKind::Outbox))?;
        let mut plaintext = Zeroizing::new(Vec::with_capacity(
            FIXED + binding.len() + self.ciphertext.len(),
        ));
        plaintext.extend_from_slice(MAGIC);
        plaintext.extend_from_slice(self.intent.as_bytes());
        plaintext.extend_from_slice(&self.semantic);
        plaintext.extend_from_slice(&self.control.to_be_bytes());
        plaintext.extend_from_slice(&self.metadata.id());
        field(&mut plaintext, &binding)?;
        field(&mut plaintext, &self.ciphertext)?;
        let coordinate = coordinate(chat, id, writer)?;
        key.seal_journal(&coordinate, &plaintext)
            .map_err(|_| ProjectionError::Persistence)
    }
    pub(super) fn open(
        key: &ChatKey,
        chat: ChatCoordinate,
        id: CommandId,
        upper: WriterEpoch,
        cell: &SealedCell,
    ) -> Result<Self, ProjectionError> {
        let writer = WriterEpoch::new(cell.writer)
            .map_err(|_| ProjectionError::InvalidRecord(RecordKind::Outbox))?;
        if writer > upper {
            return Err(ProjectionError::Rollback);
        }
        let plaintext = key
            .open_journal(&coordinate(chat, id, writer)?, &cell.ciphertext)
            .map_err(|_| ProjectionError::InvalidRecord(RecordKind::Outbox))?;
        let mut input = Input(&plaintext);
        if input.take(8)? != MAGIC {
            return Err(ProjectionError::InvalidRecord(RecordKind::Outbox));
        }
        let intent = Fingerprint::from_bytes(input.array()?);
        let semantic = input.array()?;
        let control = u64::from_be_bytes(input.array()?);
        let publication = input.array()?;
        let binding = RelayBinding::from_canonical_bytes(input.field(MAX_RELAY_BINDING_BYTES)?)
            .map_err(|_| ProjectionError::InvalidRecord(RecordKind::Outbox))?;
        let ciphertext = input.field(nocterm_hub_model::cloud::MAX_ENVELOPE_BYTES)?;
        if !input.0.is_empty()
            || binding
                .command_route()
                .is_none_or(|route| route.command_id != *id.as_bytes())
        {
            return Err(ProjectionError::InvalidRecord(RecordKind::Outbox));
        }
        let metadata = PublicationMetadata::new(
            publication,
            binding,
            u32::try_from(ciphertext.len()).map_err(|_| ProjectionError::Capacity)?,
            Sha256::digest(ciphertext).into(),
        )
        .map_err(|_| ProjectionError::InvalidRecord(RecordKind::Outbox))?;
        Ok(Self {
            intent,
            semantic,
            control,
            metadata,
            ciphertext: ciphertext.to_vec(),
        })
    }
}
fn coordinate(
    chat: ChatCoordinate,
    id: CommandId,
    writer: WriterEpoch,
) -> Result<JournalCoordinate, ProjectionError> {
    // Local Outbox slot, not a runner event/transaction claim. Zero local counters
    // are fixed schema coordinates; the actual captured writer is never invented.
    JournalCoordinate::new(
        chat,
        JournalClass::Outbox,
        *id.as_bytes(),
        0,
        0,
        writer.get(),
    )
    .map_err(|_| ProjectionError::InvalidRecord(RecordKind::Outbox))
}
pub(super) fn field(output: &mut Vec<u8>, bytes: &[u8]) -> Result<(), ProjectionError> {
    output.extend_from_slice(
        &u32::try_from(bytes.len())
            .map_err(|_| ProjectionError::Capacity)?
            .to_be_bytes(),
    );
    output.extend_from_slice(bytes);
    Ok(())
}
pub(super) struct Input<'a>(pub(super) &'a [u8]);
impl<'a> Input<'a> {
    pub(super) fn take(&mut self, count: usize) -> Result<&'a [u8], ProjectionError> {
        if count > self.0.len() {
            return Err(ProjectionError::InvalidRecord(RecordKind::Outbox));
        }
        let (field, rest) = self.0.split_at(count);
        self.0 = rest;
        Ok(field)
    }
    pub(super) fn array<const N: usize>(&mut self) -> Result<[u8; N], ProjectionError> {
        self.take(N)?
            .try_into()
            .map_err(|_| ProjectionError::InvalidRecord(RecordKind::Outbox))
    }
    pub(super) fn field(&mut self, maximum: usize) -> Result<&'a [u8], ProjectionError> {
        let length = usize::try_from(u32::from_be_bytes(self.array()?))
            .map_err(|_| ProjectionError::Capacity)?;
        if length > maximum {
            return Err(ProjectionError::Capacity);
        }
        self.take(length)
    }
}

/// Native intent equality is separate from the normative SDK semantic digest.
pub(super) fn intent_fingerprint(command: &ConversationCommand) -> Fingerprint {
    let mut hash = Sha256::new();
    hash.update(b"nocterm-native-conversation-command-v1");
    let stream = command.coordinates.binding.stream();
    let prep = command.preparation;
    let policy = prep.policy();
    let mut part = |value: &[u8]| {
        hash.update((value.len() as u64).to_be_bytes());
        hash.update(value);
    };
    for value in [
        stream.scope().hub().instance().as_str().as_bytes(),
        stream.scope().hub().authority().as_bytes(),
        stream.scope().account().id().as_str().as_bytes(),
        stream.scope().account().key().as_bytes(),
        stream.runner().as_bytes(),
        stream.chat().as_bytes(),
        command.id.as_bytes(),
        policy.as_bytes(),
    ] {
        part(value);
    }
    hash.update(stream.runner_epoch().get().to_be_bytes());
    hash.update(command.coordinates.binding.writer().get().to_be_bytes());
    hash.update(command.coordinates.control.get().to_be_bytes());
    hash.update(prep.issued().as_millis().to_be_bytes());
    hash.update(prep.expires().as_millis().to_be_bytes());
    match &command.coordinates.run {
        None => hash.update([0]),
        Some(run) => {
            hash.update([1]);
            hash.update(run.run().as_bytes());
            hash.update(run.epoch().get().to_be_bytes());
        }
    }
    match &command.action {
        nocterm_cloud_model::projection::ConversationIntent::Prompt(text) => {
            hash.update([1]);
            hash.update(text.as_bytes());
        }
        nocterm_cloud_model::projection::ConversationIntent::Stop => hash.update([2]),
        nocterm_cloud_model::projection::ConversationIntent::AnswerApproval(request) => {
            approval_fingerprint(&mut hash, request)
        }
        nocterm_cloud_model::projection::ConversationIntent::GrantInference => hash.update([5]),
        nocterm_cloud_model::projection::ConversationIntent::GrantProvider(access) => {
            hash.update([4]);
            hash.update(access.grant());
            hash.update(access.epoch().to_be_bytes());
            hash.update(access.expires().as_millis().to_be_bytes());
            hash.update(Sha256::digest(access.capsule()));
        }
    }
    Fingerprint::from_bytes(hash.finalize().into())
}

fn approval_fingerprint(
    hash: &mut Sha256,
    request: &nocterm_cloud_model::projection::ApprovalRequest,
) {
    hash.update([3]);
    hash.update(request.answer().intent().as_bytes());
    hash.update(request.answer().arguments().as_bytes());
    hash.update(request.expires().as_millis().to_be_bytes());
    let authority = request.answer().authority();
    for value in [
        authority.session().target().host(),
        authority.session().target().route(),
        authority.session().target().workspace(),
        authority.session().authentication().key(),
        authority.session().authentication().credential_revision(),
    ] {
        hash.update(value.as_bytes());
    }
    let principal = authority.session().target().principal().as_str();
    hash.update((principal.len() as u64).to_be_bytes());
    hash.update(principal.as_bytes());
    hash.update(authority.session().id().as_bytes());
    hash.update(authority.session().epoch().get().to_be_bytes());
    hash.update(authority.grant().as_bytes());
    hash.update(authority.grant_epoch().get().to_be_bytes());
    match request.answer().choice() {
        nocterm_cloud_model::coordinator::ApprovalChoice::Approve => hash.update([1]),
        nocterm_cloud_model::coordinator::ApprovalChoice::Deny => hash.update([2]),
        nocterm_cloud_model::coordinator::ApprovalChoice::ReconcileKnown(digest) => {
            hash.update([3]);
            hash.update(digest.as_bytes());
        }
    }
}
