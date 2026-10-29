//! Admission precedes canonical payload and SDK signing/encryption buffers.
use super::{super::RunnerJournalCrypto, encoding_error};
use nocterm_cloud_model::{
    authority::RunBinding,
    identity::{ChatKeyEpoch, EventId, Fingerprint},
    journal::{
        HeadAnchor, HeadAnchorKind, HeadRecordPosition, JournalError, JournalPosition,
        OutboxDeliveryId, OutboxEntry, PublicationAllocation, PublicationWorkBudget,
        ResidentPublication, WriterLease,
    },
    payload::OpaqueBytes,
};
use nocterm_hub_model::cloud::{
    MAX_HEADER_BYTES, MAX_PLAINTEXT_BYTES, RecordContext, RecordHeader, RecordKind,
    events::{CloudEvent, CloudSnapshotPayload},
};
use sha2::{Digest, Sha256};

// SDK framing: two u32 length prefixes, Ed25519 signature, and purpose/version,
// XChaCha nonce and tag. These are size bounds, never a second wire encoder.
const RECORD_FRAME: usize = 8 + 64;
const AEAD_FRAME: usize = 2 + 24 + 16;
// Context/header canonical buffers, hexadecimal AAD and its framed copy have
// bounded metadata-only allocations. Reserve all simultaneously live generations.
const METADATA_WORK: usize = 16 * MAX_HEADER_BYTES;

pub(super) struct RecordSealer<'a> {
    material: &'a RunnerJournalCrypto,
    work: &'a dyn PublicationWorkBudget,
}
impl<'a> RecordSealer<'a> {
    pub(super) fn new(
        material: &'a RunnerJournalCrypto,
        work: &'a dyn PublicationWorkBudget,
    ) -> Self {
        Self { material, work }
    }
    /// Exact canonical body capacity for an outer Snapshot with no invented
    /// active run. Fixed-width sizing identities are never signed or persisted.
    pub(super) fn snapshot_body_capacity(
        &self,
        lease: &WriterLease,
        next: JournalPosition,
        maximum_ciphertext: usize,
    ) -> Result<usize, JournalError> {
        let _metadata = self
            .work
            .try_reserve(PublicationAllocation::new(METADATA_WORK, 1)?)?;
        let position = HeadRecordPosition::new(
            lease.epoch(),
            next.events(),
            next.records(),
            next.revision(),
        )?;
        self.material
            .with_material(lease.stream(), |_, chat, signer| {
                let epoch = ChatKeyEpoch::new(chat.key_epoch()).map_err(encoding_error)?;
                let context = super::super::runner_transport::record_context_parts(
                    lease.stream(),
                    epoch,
                    position,
                    EventId::from_bytes([0; 16]),
                    None,
                )?;
                let sizing =
                    SealSizing::new(&context, RecordKind::Snapshot, signer.public_key(), 0)?;
                Ok(maximum_ciphertext
                    .checked_sub(sizing.ciphertext)
                    .ok_or(JournalError::Capacity)?
                    .min(
                        MAX_PLAINTEXT_BYTES
                            .checked_sub(sizing.plaintext)
                            .ok_or(JournalError::Capacity)?,
                    ))
            })
    }
    pub(super) fn seal(
        &self,
        lease: &WriterLease,
        next: JournalPosition,
        run: Option<&RunBinding>,
        payload: RecordPayload<'_>,
    ) -> Result<ResidentPublication, JournalError> {
        self.seal_bounded(
            lease,
            next,
            run,
            payload,
            nocterm_hub_model::cloud::MAX_ENVELOPE_BYTES,
        )
    }
    pub(super) fn seal_bounded(
        &self,
        lease: &WriterLease,
        next: JournalPosition,
        run: Option<&RunBinding>,
        payload: RecordPayload<'_>,
        maximum_ciphertext: usize,
    ) -> Result<ResidentPublication, JournalError> {
        let work = self.work;
        let _metadata = work.try_reserve(PublicationAllocation::new(METADATA_WORK, 1)?)?;
        let position = HeadRecordPosition::new(
            lease.epoch(),
            next.events(),
            next.records(),
            next.revision(),
        )?;
        let id = EventId::from_bytes(*uuid::Uuid::new_v4().as_bytes());
        let delivery = OutboxDeliveryId::from_bytes(*uuid::Uuid::new_v4().as_bytes());
        self.material
            .with_material(lease.stream(), |key, chat, signer| {
                let epoch = ChatKeyEpoch::new(chat.key_epoch()).map_err(encoding_error)?;
                let context = super::super::runner_transport::record_context_parts(
                    lease.stream(),
                    epoch,
                    position,
                    id,
                    run,
                )?;
                payload.require_context(&context)?;
                let kind = payload.kind();
                let length = payload.canonical_len()?;
                let sizing = SealSizing::new(&context, kind, signer.public_key(), length)?;
                if sizing.ciphertext > maximum_ciphertext {
                    return Err(JournalError::Capacity);
                }
                let _body = work.try_reserve(PublicationAllocation::new(length.max(1), 1)?)?;
                let body = zeroize::Zeroizing::new(payload.canonical_bytes_exact(length)?);
                let sealed_length = sizing.ciphertext;
                let _scratch = sizing.reserve(work)?;
                let allocation = work.try_reserve(PublicationAllocation::new(
                    ResidentPublication::required_bytes(sealed_length)?,
                    1,
                )?)?;
                let sealed = match kind {
                    RecordKind::Event => {
                        nocterm_keys::cloud::seal_event(key, signer, context, &body)
                    }
                    RecordKind::Snapshot => {
                        nocterm_keys::cloud::seal_snapshot(key, signer, context, &body)
                    }
                }
                .map_err(encoding_error)?;
                if sealed.len() != sealed_length {
                    return Err(JournalError::Corrupt);
                }
                let digest = Fingerprint::from_bytes(Sha256::digest(&sealed).into());
                let anchor = HeadAnchor::new(
                    match kind {
                        RecordKind::Event => HeadAnchorKind::Event,
                        RecordKind::Snapshot => HeadAnchorKind::Snapshot,
                    },
                    position,
                    id,
                    run.cloned(),
                    digest,
                );
                let entry = OutboxEntry::new(
                    anchor,
                    epoch,
                    delivery,
                    OpaqueBytes::new(sealed).map_err(encoding_error)?,
                );
                ResidentPublication::new(lease.stream().clone(), entry, allocation)
            })
    }
}
fn sum(parts: &[usize]) -> Result<usize, JournalError> {
    parts.iter().try_fold(0usize, |total, part| {
        total.checked_add(*part).ok_or(JournalError::Capacity)
    })
}

struct SealSizing {
    plaintext: usize,
    ciphertext: usize,
}
impl SealSizing {
    fn new(
        context: &RecordContext,
        kind: RecordKind,
        signer: [u8; 32],
        body: usize,
    ) -> Result<Self, JournalError> {
        // Sizing only: length and digest have fixed width. This unsigned header
        // is never signed or published; the SDK builds the actual content header.
        let header =
            RecordHeader::new(context.clone(), kind, signer, &[]).map_err(encoding_error)?;
        let plaintext = sum(&[
            header.canonical_bytes().map_err(encoding_error)?.len(),
            body,
            RECORD_FRAME,
        ])?;
        if plaintext > MAX_PLAINTEXT_BYTES {
            return Err(JournalError::Capacity);
        }
        Ok(Self {
            plaintext,
            ciphertext: sum(&[plaintext, AEAD_FRAME])?,
        })
    }
    fn reserve(
        &self,
        work: &dyn PublicationWorkBudget,
    ) -> Result<nocterm_core::allocation::AllocationLease, JournalError> {
        // Signed plaintext, AEAD temporary and the Vec->Arc transient copy.
        work.try_reserve(PublicationAllocation::new(
            sum(&[self.plaintext, sum(&[self.plaintext, 16])?, self.ciphertext])?,
            1,
        )?)
    }
}
pub(super) enum RecordPayload<'a> {
    Event(CloudEvent<'a>),
    Snapshot(CloudSnapshotPayload<'a>),
}
impl RecordPayload<'_> {
    fn kind(&self) -> RecordKind {
        match self {
            Self::Event(_) => RecordKind::Event,
            Self::Snapshot(_) => RecordKind::Snapshot,
        }
    }
    fn require_context(&self, context: &RecordContext) -> Result<(), JournalError> {
        match self {
            Self::Event(value) => value.require_context(context),
            Self::Snapshot(value) => value.validate(),
        }
        .map_err(encoding_error)
    }
    fn canonical_len(&self) -> Result<usize, JournalError> {
        match self {
            Self::Event(value) => value.canonical_len(),
            Self::Snapshot(value) => value.canonical_len(),
        }
        .map_err(encoding_error)
    }
    fn canonical_bytes_exact(&self, length: usize) -> Result<Vec<u8>, JournalError> {
        match self {
            Self::Event(value) => value.canonical_bytes_exact(length),
            Self::Snapshot(value) => value.canonical_bytes_exact(length),
        }
        .map_err(encoding_error)
    }
}
