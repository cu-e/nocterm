//! Native root metadata survives key erasure and fences stale pages/rows.
use super::*;
use nocterm_cloud_runner::DurableCurrentCloudHeads;

#[test]
fn current_heads_keep_real_material_original_anchor_and_revision_bound_pages() {
    let root = private_root();
    let store = journal(root.path(), Arc::new(TestCipher::default()));
    let first = claim(&store, None);
    let heads = store.current_heads();
    let empty = block_on(heads.chat(stream())).unwrap().unwrap();
    assert_eq!(empty.key_epoch().get(), 7);
    assert_ne!(empty.key_epoch().get(), stream().runner_epoch().get());
    assert_eq!(empty.facts().writer, Some(first.epoch()));
    assert!(empty.facts().head.is_none());
    assert_eq!(empty.facts().event_floor.get(), 0);
    let position = commit(
        &store,
        admission(&first, JournalPosition::default(), acceptance(1, 0)),
    );
    let published = block_on(heads.chat(stream())).unwrap().unwrap();
    let anchor = published.facts().head.as_ref().unwrap().clone();
    assert_eq!(anchor.position().writer(), first.epoch());
    assert_eq!(anchor.position().event(), position.events());
    block_on(store.acknowledge_outbox(first.clone(), position.events())).unwrap();
    assert_eq!(
        block_on(heads.chat(stream()))
            .unwrap()
            .unwrap()
            .facts()
            .head
            .as_ref(),
        Some(&anchor)
    );
    claim_second(&store).unwrap();
    let page = block_on(heads.catalog(None, 1)).unwrap();
    let cursor = page.next_cursor().unwrap();
    let renewed = claim(&store, Some(first.epoch()));
    let current = block_on(heads.chat(stream())).unwrap().unwrap();
    assert_eq!(current.facts().writer, Some(renewed.epoch()));
    assert_eq!(current.facts().head.as_ref(), Some(&anchor));
    assert!(matches!(
        block_on(heads.catalog(Some(cursor), 1)),
        Err(JournalError::Conflict)
    ));
    let fresh = block_on(heads.catalog(None, 64)).unwrap();
    assert_eq!(fresh.total(), 2);
    assert!(fresh.revision() > page.revision());
    assert_ne!(fresh.root(), page.root());
}
fn commit_original_tombstone(
    store: &DurableRunJournal,
    lease: WriterLease,
    position: JournalPosition,
) {
    let tombstone = JournalRecord::new(
        RecordContext::Chat,
        JournalRecordKind::Tombstone {
            command: CommandId::from_bytes([2; 16]),
        },
        bytes(b"tombstone"),
    )
    .unwrap();
    let next = position.next(1, 1, true).unwrap();
    let publication = fixture_event(
        &lease,
        next,
        EventId::from_bytes([2; 16]),
        None,
        bytes(b"actual original tombstone ciphertext"),
    );
    commit(
        store,
        JournalBatch::new(
            lease,
            position,
            None,
            vec![tombstone],
            None,
            vec![publication],
        )
        .unwrap(),
    );
}
#[test]
fn deleted_head_cold_open_needs_no_content_key_and_receipts_never_invent_acceptance() {
    let root = private_root();
    let cipher = Arc::new(TestCipher::default());
    let store = journal(root.path(), cipher.clone());
    let lease = claim(&store, None);
    let position = commit(
        &store,
        admission(&lease, JournalPosition::default(), acceptance(1, 0)),
    );
    commit_original_tombstone(&store, lease, position);
    let heads = store.current_heads();
    let query = OriginalCommandReceiptQuery {
        stream: stream(),
        key_epoch: ChatKeyEpoch::new(7).unwrap(),
        command: CommandId::from_bytes([1; 16]),
        digest: fp(1),
        expected_control: ChatControlRevision::default(),
    };
    assert_eq!(
        block_on(heads.receipt(query.clone())).unwrap(),
        OriginalCommandReceiptObservation::Accepted(position.control())
    );
    let missing = OriginalCommandReceiptQuery {
        command: CommandId::from_bytes([77; 16]),
        ..query.clone()
    };
    assert_eq!(
        block_on(heads.receipt(missing)).unwrap(),
        OriginalCommandReceiptObservation::Unknown
    );
    let deleted = block_on(heads.chat(stream())).unwrap().unwrap();
    assert!(deleted.facts().deleted);
    let anchor = deleted.facts().head.clone();
    cipher.retire_content_key();
    assert_eq!(
        block_on(heads.receipt(query.clone())).unwrap(),
        OriginalCommandReceiptObservation::Unknown
    );
    assert!(matches!(
        DurableRunJournal::open_existing(
            root.path(),
            catalog_binding(),
            cipher,
            Arc::new(TestCipher::default()),
            Arc::new(TestCipher::default()),
            JournalLimits::default()
        ),
        Err(JournalError::KeyUnavailable)
    ));
    let cold = DurableCurrentCloudHeads::open_existing(
        root.path(),
        catalog_binding(),
        Arc::new(TestCipher::default()),
        Arc::new(TestCipher::default()),
        JournalLimits::default(),
    )
    .unwrap();
    let restored = block_on(cold.chat(stream())).unwrap().unwrap();
    assert!(restored.facts().deleted);
    assert_eq!(restored.facts().head, anchor);
    assert_eq!(block_on(cold.catalog(None, 64)).unwrap().total(), 1);
    assert_eq!(
        block_on(cold.receipt(query)).unwrap(),
        OriginalCommandReceiptObservation::Unknown
    );
}
#[test]
fn current_head_cells_refuse_missing_extra_moved_or_corrupt_rows() {
    for corruption in [
        "DELETE FROM current_heads",
        "UPDATE current_heads SET body=zeroblob(length(body))",
        "UPDATE current_heads SET digest=zeroblob(32)",
        "UPDATE current_heads SET revision=revision+1",
        "UPDATE current_heads SET body=zeroblob(9000)",
        "INSERT INTO current_heads(chat,revision,writer,digest,body) VALUES(zeroblob(32),1,1,zeroblob(32),X'01')",
    ] {
        let root = private_root();
        let store = journal(root.path(), Arc::new(TestCipher::default()));
        claim(&store, None);
        let heads = store.current_heads();
        block_on(heads.chat(stream())).unwrap().unwrap();
        sql(root.path(), corruption);
        assert!(
            matches!(
                block_on(heads.catalog(None, 64)),
                Err(JournalError::Corrupt)
            ),
            "{corruption}"
        );
    }
}
#[test]
fn native_only_snapshot_refuses_prefix_purge_before_a_real_signed_baseline_exists() {
    let root = private_root();
    let store = journal(root.path(), Arc::new(TestCipher::default()));
    let lease = claim(&store, None);
    let position = commit(
        &store,
        admission(&lease, JournalPosition::default(), acceptance(1, 0)),
    );
    block_on(store.acknowledge_outbox(lease.clone(), position.events())).unwrap();
    let archive = JournalSnapshot::new(
        stream(),
        position,
        bytes(b"native archive is not a signed Snapshot"),
    )
    .unwrap();
    assert!(matches!(
        block_on(store.compact(lease, position, archive)),
        // No published signed baseline is active, the same refusal every
        // compaction without one receives; no covered record may be purged.
        Err(JournalError::Conflict)
    ));
    assert_eq!(
        block_on(store.read(stream(), JournalSequence::default(), limits()))
            .unwrap()
            .records()
            .len(),
        1
    );
    let current = block_on(store.current_heads().chat(stream()))
        .unwrap()
        .unwrap();
    assert!(current.facts().snapshot.is_none());
    assert_eq!(current.facts().event_floor.get(), 1);
}

#[test]
fn a_public_anchor_with_substituted_ciphertext_or_material_never_commits() {
    for substitute_material in [false, true] {
        let root = private_root();
        let store = journal(root.path(), Arc::new(TestCipher::default()));
        let lease = claim(&store, None);
        let heads = store.current_heads();
        let before = block_on(heads.catalog(None, 64)).unwrap();
        let original = admission(&lease, JournalPosition::default(), acceptance(1, 0));
        let event = &original.outbox()[0];
        let anchor = event.anchor();
        let replacement = HeadAnchor::new(
            anchor.kind(),
            anchor.position(),
            anchor.id(),
            anchor.run().cloned(),
            if substitute_material {
                anchor.ciphertext()
            } else {
                fp(99)
            },
        );
        let event = OutboxEntry::new(
            replacement,
            ChatKeyEpoch::new(if substitute_material { 8 } else { 7 }).unwrap(),
            event.delivery_id(),
            event.sealed().clone(),
        );
        let event = ResidentPublication::new(
            lease.stream().clone(),
            event,
            nocterm_cloud_model::testing::fixture_publication_budget()
                .try_reserve(PublicationAllocation::new(128 * 1024, 1).unwrap())
                .unwrap(),
        )
        .unwrap();
        let invalid = JournalBatch::new(
            lease,
            original.expected(),
            original.command().cloned(),
            original.records().to_vec(),
            original.checkpoint().cloned(),
            vec![event],
        )
        .unwrap();
        assert!(matches!(
            block_on(store.commit(invalid)),
            Err(JournalError::InvalidBatch)
        ));
        let after = block_on(heads.catalog(None, 64)).unwrap();
        assert_eq!(before.revision(), after.revision());
        assert_eq!(before.root(), after.root());
        assert!(after.heads()[0].facts().head.is_none());
        assert_eq!(
            block_on(store.head(stream())).unwrap().unwrap().position(),
            JournalPosition::default()
        );
    }
}

#[test]
fn partial_empty_root_restore_cannot_remove_the_first_non_control_public_tail() {
    let root = private_root();
    let cipher = Arc::new(TestCipher::default());
    let store = journal(root.path(), cipher.clone());
    let lease = claim(&store, None);
    let saved = RootStateSnapshot::capture(root.path());
    let next = JournalPosition::default().next(1, 1, false).unwrap();
    let status = JournalRecord::new(
        RecordContext::Run(run()),
        JournalRecordKind::RunStatus,
        bytes(b"actual host run status"),
    )
    .unwrap();
    let event = fixture_event(
        &lease,
        next,
        EventId::from_bytes([55; 16]),
        Some(run()),
        bytes(b"signed status fixture"),
    );
    commit(
        &store,
        JournalBatch::new(
            lease,
            JournalPosition::default(),
            None,
            vec![status],
            None,
            vec![event],
        )
        .unwrap(),
    );
    assert_eq!(
        block_on(store.head(stream()))
            .unwrap()
            .unwrap()
            .position()
            .control()
            .get(),
        0
    );
    drop(store);
    saved.restore_root_rows(root.path());
    cipher.retire_content_key();
    assert!(matches!(
        DurableCurrentCloudHeads::open_existing(
            root.path(),
            catalog_binding(),
            Arc::new(TestCipher::default()),
            Arc::new(TestCipher::default()),
            JournalLimits::default()
        ),
        Err(JournalError::Corrupt)
    ));
}
#[test]
fn an_ack_header_reseal_is_linked_even_when_the_public_anchor_does_not_change() {
    let root = private_root();
    let store = journal(root.path(), Arc::new(TestCipher::default()));
    let lease = claim(&store, None);
    let position = commit(
        &store,
        admission(&lease, JournalPosition::default(), acceptance(1, 0)),
    );
    let before = block_on(store.current_heads().catalog(None, 64)).unwrap();
    let saved = RootStateSnapshot::capture(root.path());
    block_on(store.acknowledge_outbox(lease, position.events())).unwrap();
    let after = block_on(store.current_heads().catalog(None, 64)).unwrap();
    assert!(after.revision() > before.revision());
    assert_eq!(
        after.heads()[0].facts().head,
        before.heads()[0].facts().head
    );
    drop(store);
    saved.restore_root_rows(root.path());
    assert!(matches!(
        DurableCurrentCloudHeads::open_existing(
            root.path(),
            catalog_binding(),
            Arc::new(TestCipher::default()),
            Arc::new(TestCipher::default()),
            JournalLimits::default()
        ),
        Err(JournalError::Corrupt)
    ));
}
#[test]
fn oversized_or_wrong_typed_root_metadata_is_refused_before_any_cipher_open() {
    for corruption in [
        "UPDATE account_catalog SET body=zeroblob(8388608)",
        "UPDATE account_catalog SET body=CAST(zeroblob(8388608) AS TEXT)",
        "UPDATE account_catalog SET revision=CAST(zeroblob(8388608) AS TEXT)",
        "UPDATE chats SET chat=zeroblob(8388608)",
        "UPDATE chats SET chat_id=CAST(zeroblob(8388608) AS TEXT)",
        "UPDATE chats SET binding=zeroblob(8388608)",
        "UPDATE chats SET writer=CAST(zeroblob(8388608) AS TEXT)",
        "UPDATE chats SET key_epoch=zeroblob(8388608)",
        "UPDATE chats SET header_digest=zeroblob(8388608)",
        "UPDATE chats SET body=zeroblob(8388608)",
        "UPDATE current_heads SET digest=zeroblob(8388608)",
        "UPDATE current_heads SET revision=CAST(zeroblob(8388608) AS TEXT)",
        "UPDATE current_heads SET body=CAST(zeroblob(8388608) AS TEXT)",
    ] {
        let root = private_root();
        let catalog = Arc::new(TestCipher::default());
        let cells = Arc::new(TestCipher::default());
        let store = DurableRunJournal::create_new(
            root.path(),
            catalog_binding(),
            Arc::new(TestCipher::default()),
            catalog.clone(),
            cells.clone(),
            JournalLimits::default(),
        )
        .unwrap();
        claim(&store, None);
        let heads = store.current_heads();
        let before = (catalog.metadata_opens(), cells.metadata_opens());
        let corruptor = rusqlite::Connection::open(root.path().join("cloud-runs.sqlite3")).unwrap();
        // Model offline corruption; production constraints/caps stay unchanged.
        corruptor
            .pragma_update(None, "foreign_keys", false)
            .unwrap();
        corruptor
            .pragma_update(None, "ignore_check_constraints", true)
            .unwrap();
        corruptor.execute_batch(corruption).unwrap();
        drop(corruptor);
        assert!(
            matches!(
                block_on(heads.catalog(None, 64)),
                Err(JournalError::Corrupt)
            ),
            "{corruption}"
        );
        assert_eq!(
            (catalog.metadata_opens(), cells.metadata_opens()),
            before,
            "preflight ran after cipher open: {corruption}"
        );
    }
}
