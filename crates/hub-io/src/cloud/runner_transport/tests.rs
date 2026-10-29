//! Native delivery identity never substitutes for signed event or retention order.
use super::*;
use crate::cloud::runner_test_support::binding;
use nocterm_cloud_model::{
    authority::RunBinding,
    coordinator::{IntakeLane, IntakeLimits},
    identity::*,
    journal::*,
    payload::OpaqueBytes,
};

fn publication(kind: HeadAnchorKind) -> ResidentPublication {
    let source = binding(2);
    let stream = source.stream(CloudChatId::from_bytes([5; 16]));
    let run = RunBinding::new(
        source.scope().clone(),
        source.runner(),
        source.epoch(),
        stream.chat(),
        RunId::from_bytes([6; 16]),
        RunEpoch::new(3).unwrap(),
    );
    let sealed = OpaqueBytes::new(vec![7; 128]).unwrap();
    let anchor = HeadAnchor::new(
        kind,
        HeadRecordPosition::new(
            WriterEpoch::new(8).unwrap(),
            EventSequence::from_u64(91),
            JournalSequence::from_u64(101),
            StreamRevision::from_u64(9),
        )
        .unwrap(),
        EventId::from_bytes([10; 16]),
        Some(run),
        Fingerprint::from_bytes(Sha256::digest(sealed.as_bytes()).into()),
    );
    let work = PublicationBudget::maintenance(IntakeLimits::default().pool(IntakeLane::Ordinary));
    let allocation = work
        .try_reserve(
            PublicationAllocation::new(
                ResidentPublication::required_bytes(sealed.len()).unwrap(),
                1,
            )
            .unwrap(),
        )
        .unwrap();
    ResidentPublication::new(
        stream,
        OutboxEntry::new(
            anchor,
            ChatKeyEpoch::new(11).unwrap(),
            OutboxDeliveryId::from_bytes([12; 16]),
            sealed,
        ),
        allocation,
    )
    .unwrap()
}

#[test]
fn runner_publication_preserves_original_source_material_run_and_distinct_delivery_id() {
    let original = publication(HeadAnchorKind::Event);
    let metadata = metadata(&original).unwrap();
    assert_eq!(metadata.id(), [12; 16]);
    assert_eq!(metadata.total_bytes(), 128);
    assert_eq!(
        metadata.digest(),
        *original.anchor().ciphertext().as_bytes()
    );
    let context = metadata.binding().record_context().unwrap();
    assert_eq!(context.id(), [10; 16]);
    assert_eq!(context.chat().key_epoch(), 11);
    assert_eq!(context.chat().runner().key_epoch, 4);
    assert_eq!(context.position().writer_epoch, 8);
    assert_eq!(context.position().sequence, 91);
    assert_eq!(context.position().journal_sequence, 101);
    assert_eq!(context.position().revision, 9);
    assert_eq!(context.run().unwrap().expected_epoch, 3);
    let source = original.stream();
    let wrong_run = RunBinding::new(
        binding(9).scope().clone(),
        source.runner(),
        source.runner_epoch(),
        source.chat(),
        RunId::from_bytes([6; 16]),
        RunEpoch::new(3).unwrap(),
    );
    let moved = HeadAnchor::new(
        HeadAnchorKind::Event,
        original.anchor().position(),
        original.id(),
        Some(wrong_run),
        original.anchor().ciphertext(),
    );
    assert!(matches!(
        record_context(source, original.key_epoch(), &moved),
        Err(JournalError::Fenced)
    ));
}

#[test]
fn runner_snapshot_purge_uses_durable_relay_cursor_instead_of_signed_sequence() {
    let original = publication(HeadAnchorKind::Snapshot);
    let covered = JournalPosition::new(
        StreamRevision::from_u64(8),
        ChatControlRevision::from_u64(0),
        JournalSequence::from_u64(100),
        EventSequence::from_u64(90),
    );
    let candidate =
        SnapshotCandidate::new(&original, covered, Fingerprint::from_bytes([14; 32]), 64).unwrap();
    assert!(purge_receipt(&candidate).is_err());
    let receipt = OutboxStored::new(
        original.stream().clone(),
        original.delivery_id(),
        original.anchor().ciphertext(),
        DeliveryCursor::new(18).unwrap(),
    );
    let candidate = candidate.record_stored(receipt).unwrap();
    let stored = purge_receipt(&candidate).unwrap();
    let purged = SnapshotPurged::new(stored, 17).unwrap();
    assert_eq!(purged.through(), 17);
    assert_eq!(original.sequence().get(), 91);
    assert!(SnapshotPurged::new(purged.stored().clone(), 90).is_err());
}
