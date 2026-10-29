//! A bounded display window from verified public records, never native frames.
mod window;
use super::{
    encoding_error,
    sealing::{RecordPayload, RecordSealer},
};
use crate::cloud::{RunnerJournalCrypto, runner_transport::record_context};
use nocterm_cloud_model::{
    identity::EventSequence,
    journal::{
        HeadAnchorKind, JournalError, JournalPosition, PublicationAllocation,
        PublicationWorkBudget, ResidentPublication, SnapshotLimits, SnapshotProjectionSource,
        WriterLease,
    },
};
use nocterm_core::allocation::AllocationLease;
use nocterm_hub_model::cloud::{
    MAX_HEADER_BYTES, MAX_ID_BYTES,
    events::{
        AcceptedPrompt, BackgroundJob, BackgroundLifecyclePayload, CloudEvent,
        CloudSnapshotPayload, MAX_EVENT_JSON_BYTES, MAX_SNAPSHOT_EVENTS, ToolIntentPayload,
        ToolObservationPayload,
    },
    payload::{ApprovalPayload, PromptPart},
};
use nocterm_keys::cloud::{OpenedRecord, open_event, open_snapshot};
use sha2::{Digest, Sha256};
use std::mem::size_of;

pub(super) struct SnapshotProjection<'a> {
    material: &'a RunnerJournalCrypto,
    work: &'a dyn PublicationWorkBudget,
}
impl<'a> SnapshotProjection<'a> {
    pub(super) fn new(
        material: &'a RunnerJournalCrypto,
        work: &'a dyn PublicationWorkBudget,
    ) -> Self {
        Self { material, work }
    }
    pub(super) fn encode(
        &self,
        lease: &WriterLease,
        next: JournalPosition,
        source: &SnapshotProjectionSource,
        limits: SnapshotLimits,
        timestamp_ms: u64,
    ) -> Result<ResidentPublication, JournalError> {
        if lease.stream() != source.stream() {
            return Err(JournalError::Fenced);
        }
        if next != source.covered().next(1, 1, false)? {
            return Err(JournalError::InvalidBatch);
        }
        let input = ProjectionInput::open(self.material, self.work, lease, source, limits)?;
        let decoded = input.decode(self.work, source, limits, timestamp_ms)?;
        let sealer = RecordSealer::new(self.material, self.work);
        let publication = decoded.finish(
            |payload| {
                let body_capacity = sealer.snapshot_body_capacity(lease, next, limits.output())?;
                window::WindowSelection::retain(payload, limits.records(), body_capacity, self.work)
            },
            |payload| {
                sealer.seal_bounded(
                    lease,
                    next,
                    None,
                    RecordPayload::Snapshot(payload),
                    limits.output(),
                )
            },
        )?;
        if publication.key_epoch() != source.material() {
            return Err(JournalError::Fenced);
        }
        Ok(publication)
    }
}

struct ChargedRecord<'a> {
    original: &'a ResidentPublication,
    opened: OpenedRecord,
    _allocation: AllocationLease,
}
struct ProjectionInput<'a> {
    records: Vec<ChargedRecord<'a>>,
    _allocation: AllocationLease,
}
impl<'a> ProjectionInput<'a> {
    fn open(
        material: &RunnerJournalCrypto,
        work: &dyn PublicationWorkBudget,
        lease: &WriterLease,
        source: &'a SnapshotProjectionSource,
        limits: SnapshotLimits,
    ) -> Result<Self, JournalError> {
        if source.records().len() > limits.records() {
            return Err(JournalError::Capacity);
        }
        let count = source.records().len() + usize::from(source.previous().is_some());
        let allocation = reserve(work, count.checked_mul(size_of::<ChargedRecord<'_>>()))?;
        let mut records: Vec<ChargedRecord<'a>> = Vec::with_capacity(count);
        let mut after = EventSequence::default();
        let mut bytes = 0usize;
        for (index, publication) in source
            .previous()
            .into_iter()
            .chain(source.records())
            .enumerate()
        {
            bytes = bytes
                .checked_add(publication.sealed().len())
                .ok_or(JournalError::Capacity)?;
            if bytes > limits.bytes() {
                return Err(JournalError::Capacity);
            }
            let baseline = index == 0 && source.previous().is_some();
            validate_original(publication, source, lease, after, baseline)?;
            if records
                .iter()
                .any(|record| record.original.id() == publication.id())
            {
                return Err(JournalError::Corrupt);
            }
            if let Some(previous) = records.last() {
                let before = previous.original.anchor().position();
                let next = publication.anchor().position();
                if next.writer() < before.writer()
                    || next.journal() < before.journal()
                    || next.revision() < before.revision()
                {
                    return Err(JournalError::Corrupt);
                }
            }
            records.push(open_original(material, work, publication, baseline)?);
            after = publication.sequence();
        }
        if after != source.covered().events() {
            return Err(JournalError::EventGap {
                expected: source.covered().events(),
                actual: after,
            });
        }
        Ok(Self {
            records,
            _allocation: allocation,
        })
    }
    fn decode(
        &self,
        work: &dyn PublicationWorkBudget,
        source: &SnapshotProjectionSource,
        limits: SnapshotLimits,
        timestamp_ms: u64,
    ) -> Result<DecodedProjection<CloudSnapshotPayload<'_>>, JournalError> {
        let prior = if source.previous().is_some() {
            MAX_SNAPSHOT_EVENTS
        } else {
            0
        };
        let maximum = prior
            .checked_add(source.records().len())
            .ok_or(JournalError::Capacity)?;
        let largest = self
            .records
            .iter()
            .map(|record| record.original.sealed().len())
            .max()
            .unwrap_or(0);
        let allocation = reserve(work, decode_bytes(prior, maximum, largest))?;
        let mut payload = CloudSnapshotPayload {
            control_revision: source.covered().control().get(),
            timestamp_ms,
            omitted_earlier: false,
            events: Vec::with_capacity(maximum),
        };
        let mut order = ProjectionOrder::default();
        for record in &self.records {
            let signed = record.opened.record().map_err(encoding_error)?;
            if record.original.anchor().kind() == HeadAnchorKind::Snapshot {
                let baseline =
                    CloudSnapshotPayload::from_record(&signed).map_err(encoding_error)?;
                order.advance(baseline.control_revision, baseline.timestamp_ms, &payload)?;
                payload.omitted_earlier = baseline.omitted_earlier;
                payload.events.extend(baseline.events);
            } else {
                let event = CloudEvent::from_record(&signed).map_err(encoding_error)?;
                order.advance(event.control_revision, event.timestamp_ms, &payload)?;
                payload.events.push(event);
            }
        }
        // Limits are rechecked even when the caller constructed its source using
        // a different ceiling. Window reduction below never splits an event.
        if source.records().len() > limits.records() {
            return Err(JournalError::Capacity);
        }
        Ok(DecodedProjection::new(payload, allocation))
    }
}
fn validate_original(
    publication: &ResidentPublication,
    source: &SnapshotProjectionSource,
    lease: &WriterLease,
    after: EventSequence,
    baseline: bool,
) -> Result<(), JournalError> {
    let position = publication.anchor().position();
    let expected = if baseline {
        HeadAnchorKind::Snapshot
    } else {
        HeadAnchorKind::Event
    };
    if publication.stream() != source.stream()
        || publication.key_epoch() != source.material()
        || publication.anchor().kind() != expected
        || publication.sealed().is_empty()
        || position.writer() > lease.epoch()
        || position.journal() > source.covered().records()
        || position.revision() > source.covered().revision()
        || (baseline && publication.anchor().run().is_some())
        || (!baseline && publication.sequence() != after.advance(1)?)
        || publication.anchor().ciphertext().as_bytes().as_slice()
            != Sha256::digest(publication.sealed().as_bytes()).as_slice()
    {
        return Err(JournalError::Corrupt);
    }
    Ok(())
}
fn open_original<'a>(
    material: &RunnerJournalCrypto,
    work: &dyn PublicationWorkBudget,
    original: &'a ResidentPublication,
    baseline: bool,
) -> Result<ChargedRecord<'a>, JournalError> {
    // The SDK decrypt owns <=ciphertext.len() plaintext capacity. Metadata covers
    // context/header parse+canonical buffers, hex AAD and its framed copy, with
    // the same finite header generation ceiling as RecordSealer. No payload clone.
    let allocation = reserve(
        work,
        original.sealed().len().checked_add(16 * MAX_HEADER_BYTES),
    )?;
    let opened = material.with_material(original.stream(), |key, chat, signing| {
        if chat.key_epoch() != original.key_epoch().get() {
            return Err(JournalError::Fenced);
        }
        let context = record_context(original.stream(), original.key_epoch(), original.anchor())?;
        if context.chat() != chat {
            return Err(JournalError::Corrupt);
        }
        if baseline {
            open_snapshot(
                key,
                &signing.public_key(),
                &context,
                original.sealed().as_bytes(),
            )
        } else {
            open_event(
                key,
                &signing.public_key(),
                &context,
                original.sealed().as_bytes(),
            )
        }
        .map_err(encoding_error)
    })?;
    Ok(ChargedRecord {
        original,
        opened,
        _allocation: allocation,
    })
}
pub(super) struct DecodedProjection<T> {
    payload: T,
    _allocation: AllocationLease,
}
impl<T> DecodedProjection<T> {
    pub(super) fn new(payload: T, allocation: AllocationLease) -> Self {
        Self {
            payload,
            _allocation: allocation,
        }
    }
    pub(super) fn finish<R>(
        mut self,
        prepare: impl FnOnce(&mut T) -> Result<(), JournalError>,
        consume: impl FnOnce(T) -> Result<R, JournalError>,
    ) -> Result<R, JournalError> {
        // Field drop order remains intact through every fallible preparation.
        prepare(&mut self.payload)?;
        let Self {
            payload,
            _allocation,
        } = self;
        // The callee owns/destroys its payload before this caller's lease drops.
        consume(payload)
    }
}
#[derive(Default)]
struct ProjectionOrder {
    control: u64,
    timestamp: u64,
}
impl ProjectionOrder {
    fn advance(
        &mut self,
        control: u64,
        timestamp: u64,
        head: &CloudSnapshotPayload<'_>,
    ) -> Result<(), JournalError> {
        if control < self.control
            || timestamp < self.timestamp
            || control > head.control_revision
            || timestamp > head.timestamp_ms
        {
            return Err(JournalError::Corrupt);
        }
        self.control = control;
        self.timestamp = timestamp;
        Ok(())
    }
}
fn reserve(
    work: &dyn PublicationWorkBudget,
    bytes: Option<usize>,
) -> Result<AllocationLease, JournalError> {
    work.try_reserve(PublicationAllocation::new(
        bytes.ok_or(JournalError::Capacity)?.max(1),
        1,
    )?)
}
fn decode_bytes(prior: usize, maximum: usize, largest: usize) -> Option<usize> {
    // SDK prior-snapshot collect may retain a geometrically grown event Vec;
    // our destination Vec has exact capacity prior+new. Both coexist on transfer.
    let event_slots = prior.checked_mul(2)?.checked_add(maximum)?;
    // Per-event closed decoder allocations: one largest boxed payload (including
    // a nested background job), <=256 argv refs with collect growth, <=128 prompt
    // parts, and <=6 owned 128-byte identifiers. All content strings/images borrow.
    let boxed = [
        size_of::<AcceptedPrompt<'_>>(),
        size_of::<ToolIntentPayload<'_>>(),
        size_of::<ApprovalPayload>(),
        size_of::<ToolObservationPayload<'_>>() + size_of::<BackgroundJob<'_>>(),
        size_of::<BackgroundLifecyclePayload<'_>>(),
    ]
    .into_iter()
    .max()?;
    let argv = 512usize.checked_mul(size_of::<&str>())?;
    let prompt = 128usize.checked_mul(size_of::<PromptPart<'_>>())?;
    let identifiers = 6usize.checked_mul(MAX_ID_BYTES)?;
    let per_event = boxed
        .checked_add(argv)?
        .checked_add(prompt)?
        .checked_add(identifiers)?;
    // At most one serde_json parser is live. Its input is bounded by both the
    // 4MiB JSON field cap and the largest actual ciphertext (plaintext is smaller).
    // Geometric scratch growth is covered by twice that input; not allocator/RSS.
    event_slots
        .checked_mul(size_of::<CloudEvent<'_>>())?
        .checked_add(maximum.checked_mul(per_event)?)?
        .checked_add(largest.min(MAX_EVENT_JSON_BYTES).checked_mul(2)?)
}
