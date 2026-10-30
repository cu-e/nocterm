mod support;
use futures::executor::block_on;
use nocterm_cloud_model::{identity::*, journal::*};
use nocterm_cloud_runner::JournalLimits;
use std::sync::Arc;
use support::*;

#[test]
fn compaction_requires_delivered_outbox_and_retains_original_command_receipts() {
    let root = private_root();
    let cipher = Arc::new(TestCipher::default());
    let store = journal(root.path(), cipher.clone());
    let lease = claim(&store, None);
    let original = admission(&lease, JournalPosition::default(), acceptance(1, 0));
    let position = commit(&store, original.clone());
    let snapshot =
        JournalSnapshot::new(stream(), position, bytes(b"signed encrypted snapshot")).unwrap();
    assert!(matches!(
        block_on(store.compact(lease.clone(), position, snapshot.clone())),
        Err(JournalError::Conflict)
    ));
    let snapshot = finish_snapshot(&store, &lease);
    block_on(store.compact(lease.clone(), position, snapshot.snapshot().clone())).unwrap();
    drop(store);
    let store = journal(root.path(), cipher);
    assert!(matches!(
        block_on(store.read(stream(), JournalSequence::default(), limits())),
        Err(JournalError::Gap { .. })
    ));
    assert!(matches!(
        block_on(store.outbox(stream(), EventSequence::default(), limits())),
        Err(JournalError::EventGap { .. })
    ));
    let retained = block_on(store.outbox(stream(), position.events(), limits())).unwrap();
    assert_eq!(retained.len(), 1);
    assert_eq!(retained[0].anchor().kind(), HeadAnchorKind::Snapshot);
    assert!(matches!(
        block_on(store.commit(original)),
        Ok(CommitOutcome::Duplicate(_))
    ));
    let snapshot = block_on(store.snapshot(stream())).unwrap().unwrap();
    assert_eq!(snapshot.at(), position);
    assert!(snapshot.payload().as_bytes().starts_with(b"NCRA\x02"));
}

#[test]
fn snapshot_record_quota_allows_one_maintenance_marker_at_full_single_chat_capacity() {
    let root = private_root();
    let store = open_journal(
        root.path(),
        Arc::new(TestCipher::default()),
        JournalLimits::new(16 * 1024 * 1024, 8 * 1024 * 1024, 2, 1).unwrap(),
    )
    .unwrap();
    let lease = claim(&store, None);
    let first = commit(
        &store,
        admission(&lease, JournalPosition::default(), acceptance(1, 0)),
    );
    let second = commit(&store, admission(&lease, first, acceptance(2, 1)));
    assert!(matches!(
        block_on(store.commit(admission(&lease, second, acceptance(3, 2)))),
        Err(JournalError::Capacity)
    ));
    let archive = finish_snapshot(&store, &lease);
    assert_eq!(archive.at(), second);
    let staged = block_on(store.head(stream())).unwrap().unwrap().position();
    let third = commit(&store, admission(&lease, staged, acceptance(3, 2)));
    assert_eq!(third.records().get(), 4);
    assert!(matches!(
        block_on(store.commit(admission(&lease, third, acceptance(4, 3)))),
        Err(JournalError::Capacity)
    ));
    let page = block_on(store.read(stream(), second.records(), limits())).unwrap();
    assert_eq!(page.records().len(), 2);
    assert!(matches!(
        page.records()[0].record().kind(),
        JournalRecordKind::PublicSnapshot
    ));
    assert_eq!(page.head(), third);
}

#[test]
fn tombstone_fences_new_work_but_preserves_original_delete_receipt() {
    let root = private_root();
    let cipher = Arc::new(TestCipher::default());
    let store = journal(root.path(), cipher.clone());
    let lease = claim(&store, None);
    let base = acceptance(9, 0);
    let deletion = CommandAcceptance::new(
        base.id(),
        CommandKind::DeleteChat,
        base.digest(),
        base.issuer().clone(),
        base.expected_control(),
    );
    let records = vec![
        JournalRecord::new(
            RecordContext::Chat,
            JournalRecordKind::CommandAccepted {
                id: deletion.id(),
                kind: deletion.kind(),
            },
            bytes(b"delete command"),
        )
        .unwrap(),
        JournalRecord::new(
            RecordContext::Chat,
            JournalRecordKind::Tombstone {
                command: deletion.id(),
            },
            bytes(b"tombstone"),
        )
        .unwrap(),
    ];
    let outbox = (1..=2)
        .map(|sequence| {
            fixture_event(
                &lease,
                JournalPosition::new(
                    StreamRevision::from_u64(1),
                    ChatControlRevision::from_u64(1),
                    JournalSequence::from_u64(sequence),
                    EventSequence::from_u64(sequence),
                ),
                EventId::from_bytes([sequence as u8; 16]),
                None,
                bytes(b"sealed deletion"),
            )
        })
        .collect();
    let batch = JournalBatch::new(
        lease.clone(),
        JournalPosition::default(),
        Some(deletion),
        records,
        None,
        outbox,
    )
    .unwrap();
    let position = commit(&store, batch.clone());
    drop(store);
    let store = journal(root.path(), cipher);
    assert!(matches!(
        block_on(store.commit(batch)),
        Ok(CommitOutcome::Duplicate(_))
    ));
    assert!(matches!(
        block_on(store.commit(admission(&lease, position, acceptance(10, 1)))),
        Err(JournalError::Fenced)
    ));
    assert!(matches!(
        block_on(store.claim_writer(
            WriterClaim::new(stream(), Some(lease.epoch()), lease.epoch().next().unwrap()).unwrap()
        )),
        Err(JournalError::Fenced)
    ));
}

#[test]
fn missing_middle_record_and_changed_ciphertext_are_never_skipped() {
    let root = private_root();
    let store = journal(root.path(), Arc::new(TestCipher::default()));
    let lease = claim(&store, None);
    let position = commit(
        &store,
        admission(&lease, JournalPosition::default(), acceptance(1, 0)),
    );
    commit(&store, admission(&lease, position, acceptance(2, 1)));
    let db = rusqlite::Connection::open(root.path().join("cloud-runs.sqlite3")).unwrap();
    db.execute("DELETE FROM entries WHERE sequence=1", [])
        .unwrap();
    assert!(matches!(
        block_on(store.read(stream(), JournalSequence::default(), limits())),
        Err(JournalError::Gap { .. })
    ));
    db.execute("UPDATE entries SET body=zeroblob(length(body))", [])
        .unwrap();
    assert!(matches!(
        block_on(store.read(stream(), JournalSequence::from_u64(1), limits())),
        Err(JournalError::Corrupt)
    ));
}

#[test]
fn native_mid_transaction_write_failure_rolls_back_the_whole_admission() {
    let root = private_root();
    let store = journal(root.path(), Arc::new(TestCipher::default()));
    let lease = claim(&store, None);
    let db = rusqlite::Connection::open(root.path().join("cloud-runs.sqlite3")).unwrap();
    db.execute_batch("CREATE TRIGGER reject_outbox BEFORE INSERT ON outbox BEGIN SELECT RAISE(ABORT,'injected write failure'); END;").unwrap();
    let result = block_on(store.commit(admission(
        &lease,
        JournalPosition::default(),
        acceptance(1, 0),
    )));
    let Err(JournalError::Storage(cause)) = result else {
        panic!("expected original SQLite failure")
    };
    assert!(cause.downcast_ref::<rusqlite::Error>().is_some());
    assert_eq!(
        block_on(store.read(stream(), JournalSequence::default(), limits()))
            .unwrap()
            .head(),
        JournalPosition::default()
    );
    assert!(block_on(store.checkpoint(stream())).unwrap().is_none());
    db.execute_batch("DROP TRIGGER reject_outbox").unwrap();
    commit(
        &store,
        admission(&lease, JournalPosition::default(), acceptance(1, 0)),
    );
}

fn next_publication(
    maintenance: &nocterm_cloud_runner::DurableSnapshotPublication,
    after: EventSequence,
) -> ResidentPublication {
    block_on(maintenance.next(
        stream(),
        after,
        limits(),
        Arc::new(nocterm_cloud_model::testing::fixture_snapshot_budget()),
    ))
    .unwrap()
    .unwrap()
}
fn assert_active_baseline(
    store: &nocterm_cloud_runner::DurableRunJournal,
    candidate: &SnapshotCandidate,
    root: &std::path::Path,
) {
    let head = block_on(store.current_heads().chat(stream()))
        .unwrap()
        .unwrap();
    assert_eq!(
        head.facts().event_floor,
        candidate.anchor().position().event()
    );
    assert_eq!(head.facts().snapshot.as_ref(), Some(candidate.anchor()));
    let db = rusqlite::Connection::open(root.join("cloud-runs.sqlite3")).unwrap();
    let covered: i64 = candidate.covered().events().get().try_into().unwrap();
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM outbox_deliveries WHERE sequence<=?1",
            [covered],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM outbox_deliveries WHERE sequence>?1",
            [covered],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
}
fn assert_lost_reply_keeps_native_records(
    store: &nocterm_cloud_runner::DurableRunJournal,
    maintenance: &nocterm_cloud_runner::DurableSnapshotPublication,
    transport: &nocterm_cloud_model::testing::ScriptedPublicationTransport,
    first: &ResidentPublication,
) {
    transport.lose_next_reply();
    assert!(block_on(transport.publish(first.clone())).is_err());
    assert_eq!(
        block_on(maintenance.delivered(stream())).unwrap(),
        EventSequence::default()
    );
    // A missing transport reply cannot delete even one covered native record.
    assert_eq!(
        block_on(store.read(stream(), JournalSequence::default(), limits()))
            .unwrap()
            .records()
            .len(),
        2
    );
}
#[test]
fn signed_snapshot_lost_reply_reopens_and_retries_independent_ids_before_prefix_activation() {
    use nocterm_cloud_model::testing::ScriptedPublicationTransport;
    let root = private_root();
    let cipher = Arc::new(TestCipher::default());
    let store = journal(root.path(), cipher.clone());
    let lease = claim(&store, None);
    let covered = commit(
        &store,
        admission(&lease, JournalPosition::default(), acceptance(1, 0)),
    );
    let maintenance = snapshot_publication(&store);
    let current = CleanupLease::for_writer(lease.clone());
    let candidate = block_on(maintenance.prepare(current.clone())).unwrap();
    assert_eq!(
        candidate.anchor().position().event(),
        covered.events().advance(1).unwrap()
    );
    let first = next_publication(&maintenance, EventSequence::default());
    assert_ne!(first.id().as_bytes(), first.delivery_id().as_bytes());
    let transport = ScriptedPublicationTransport::default();
    assert_lost_reply_keeps_native_records(&store, &maintenance, &transport, &first);
    drop(maintenance);
    drop(store);
    let store = journal(root.path(), cipher);
    let maintenance = snapshot_publication(&store);
    let retry = next_publication(&maintenance, EventSequence::default());
    assert_eq!(retry.delivery_id(), first.delivery_id());
    assert_eq!(retry.anchor(), first.anchor());
    assert_eq!(retry.sealed(), first.sealed());
    let receipt = block_on(transport.publish(retry)).unwrap();
    assert_eq!(receipt.cursor().get(), 97);
    block_on(maintenance.record_delivery(current.clone(), receipt)).unwrap();
    let snapshot = next_publication(&maintenance, covered.events());
    assert_eq!(snapshot.anchor(), candidate.anchor());
    assert_ne!(snapshot.id().as_bytes(), snapshot.delivery_id().as_bytes());
    let stored = block_on(transport.publish(snapshot)).unwrap();
    block_on(maintenance.record_delivery(current.clone(), stored.clone())).unwrap();
    let pending = block_on(maintenance.pending(stream())).unwrap().unwrap();
    let wrong = OutboxStored::new(
        stream(),
        stored.delivery_id(),
        *stored.ciphertext(),
        DeliveryCursor::new(candidate.anchor().position().event().get()).unwrap(),
    );
    assert!(matches!(
        block_on(maintenance.record_stored(current.clone(), wrong)),
        Err(JournalError::Conflict)
    ));
    let purged = block_on(transport.purge(pending)).unwrap();
    assert_eq!(purged.through(), 97);
    assert_eq!(purged.stored().cursor().get(), 98);
    assert_ne!(purged.through(), covered.events().get());
    let archive = block_on(maintenance.activate(current.clone(), purged.clone())).unwrap();
    assert_eq!(archive.at(), covered);
    assert_eq!(
        block_on(maintenance.activate(current, purged))
            .unwrap()
            .at(),
        covered
    );
    assert_active_baseline(&store, &candidate, root.path());
    assert!(
        matches!(block_on(store.read(stream(),JournalSequence::default(),limits())),Err(JournalError::Gap { actual,.. }) if actual==covered.records())
    );
}

#[test]
fn signed_snapshot_activation_preserves_later_tail_control_and_replacement_writer() {
    use nocterm_cloud_model::testing::{ScriptedPublicationTransport, fixture_snapshot_budget};
    let root = private_root();
    let store = journal(root.path(), Arc::new(TestCipher::default()));
    let original = claim(&store, None);
    let covered = commit(
        &store,
        admission(&original, JournalPosition::default(), acceptance(1, 0)),
    );
    let maintenance = snapshot_publication(&store);
    let candidate =
        block_on(maintenance.prepare(CleanupLease::for_writer(original.clone()))).unwrap();
    let staged = block_on(store.head(stream())).unwrap().unwrap().position();
    let later = commit(&store, admission(&original, staged, acceptance(2, 1)));
    let replacement = claim(&store, Some(original.epoch()));
    let current = CleanupLease::for_writer(replacement.clone());
    let transport = ScriptedPublicationTransport::default();
    while block_on(maintenance.delivered(stream())).unwrap() < candidate.anchor().position().event()
    {
        let after = block_on(maintenance.delivered(stream())).unwrap();
        let publication = block_on(maintenance.next(
            stream(),
            after,
            limits(),
            Arc::new(fixture_snapshot_budget()),
        ))
        .unwrap()
        .unwrap();
        assert_eq!(publication.anchor().position().writer(), original.epoch());
        let stored = block_on(transport.publish(publication)).unwrap();
        block_on(maintenance.record_delivery(current.clone(), stored)).unwrap();
    }
    let pending = block_on(maintenance.pending(stream())).unwrap().unwrap();
    let purged = block_on(transport.purge(pending)).unwrap();
    block_on(maintenance.activate(current, purged)).unwrap();
    let native = block_on(store.head(stream())).unwrap().unwrap();
    assert_eq!(native.lease(), &replacement);
    assert_eq!(native.position(), later);
    assert_eq!(native.retained(), covered.records());
    let metadata = block_on(store.current_heads().chat(stream()))
        .unwrap()
        .unwrap();
    assert_eq!(metadata.facts().control, later.control());
    assert_eq!(
        metadata.facts().head.as_ref().unwrap().position().event(),
        later.events()
    );
    assert_eq!(metadata.facts().snapshot.as_ref(), Some(candidate.anchor()));
    assert_eq!(
        metadata
            .facts()
            .snapshot
            .as_ref()
            .unwrap()
            .position()
            .writer(),
        original.epoch()
    );
    let page = block_on(store.read(stream(), covered.records(), limits())).unwrap();
    assert_eq!(page.records().len(), 2);
    assert!(matches!(
        page.records()[0].record().kind(),
        JournalRecordKind::PublicSnapshot
    ));
    assert!(matches!(
        page.records()[1].record().kind(),
        JournalRecordKind::CommandAccepted { .. }
    ));
}

#[test]
fn snapshot_publication_type_preflight_rejects_text_and_malformed_routing_before_decode() {
    use nocterm_cloud_model::coordinator::IntakePoolLimits;
    for mutation in 0..6 {
        let root = private_root();
        let cipher = Arc::new(TestCipher::default());
        let store = journal(root.path(), cipher.clone());
        let lease = claim(&store, None);
        commit(
            &store,
            admission(&lease, JournalPosition::default(), acceptance(1, 0)),
        );
        let db = rusqlite::Connection::open(root.path().join("cloud-runs.sqlite3")).unwrap();
        match mutation {
            // Below SQLite's connection cap but above the 12 MiB metadata lease:
            // length(TEXT) would materialize it before the 1 MiB decode refusal.
            0 => {
                db.execute("UPDATE outbox SET body=?1", ["x".repeat(20 * 1024 * 1024)])
                    .unwrap();
            }
            1 => {
                db.execute("UPDATE outbox SET record='invalid'", [])
                    .unwrap();
            }
            2 => {
                db.execute("UPDATE outbox SET id='abcdefghijklmnop'", [])
                    .unwrap();
            }
            3 => {
                db.execute("UPDATE outbox SET delivery='abcdefghijklmnop'", [])
                    .unwrap();
            }
            4 => {
                db.execute("UPDATE outbox SET sequence='invalid'", [])
                    .unwrap();
            }
            _ => {
                db.execute("UPDATE outbox SET writer='invalid'", [])
                    .unwrap();
            }
        }
        let opens = cipher.outbox_opens();
        let work = Arc::new(PublicationBudget::maintenance(
            IntakePoolLimits::new(1024 * 1024, 4, 4).unwrap(),
        ));
        let publication = snapshot_publication(&store);
        // Corrupt must precede capacity, lookup or decrypt: the complete shared
        // type/length preflight runs before payload length/reservation/body work.
        assert!(
            matches!(
                block_on(publication.next(
                    stream(),
                    EventSequence::default(),
                    JournalReadLimits::new(1, 1024 * 1024).unwrap(),
                    work,
                )),
                Err(JournalError::Corrupt)
            ),
            "mutation {mutation}"
        );
        assert_eq!(cipher.outbox_opens(), opens);
    }
    assert_active_archive_type_preflight();
}
fn assert_active_archive_type_preflight() {
    use nocterm_cloud_model::{coordinator::IntakePoolLimits, testing::ScriptedSnapshotEncoder};
    let root = private_root();
    let store = journal(root.path(), Arc::new(TestCipher::default()));
    let lease = claim(&store, None);
    commit(
        &store,
        admission(&lease, JournalPosition::default(), acceptance(1, 0)),
    );
    let (_, purged) = finish_snapshot_with_receipt(&store, &lease);
    let db = rusqlite::Connection::open(root.path().join("cloud-runs.sqlite3")).unwrap();
    db.execute(
        "UPDATE snapshots SET body=?1",
        ["x".repeat(20 * 1024 * 1024)],
    )
    .unwrap();
    let work =
        PublicationBudget::maintenance(IntakePoolLimits::new(16 * 1024 * 1024, 4, 4).unwrap());
    let publication = nocterm_cloud_runner::DurableSnapshotPublication::new(
        store,
        Arc::new(ScriptedSnapshotEncoder),
        Arc::new(work),
        SnapshotLimits::new(4096, 64 * 1024 * 1024, 8 * 1024 * 1024).unwrap(),
    );
    // After the 12 MiB metadata reservation, raw archive decoding cannot fit.
    // The existing-candidate branch must reject TEXT before length()/Capacity.
    assert!(matches!(
        block_on(publication.activate(CleanupLease::for_writer(lease), purged)),
        Err(JournalError::Corrupt)
    ));
}
