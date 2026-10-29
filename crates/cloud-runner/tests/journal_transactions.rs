mod support;
use futures::executor::block_on;
use nocterm_cloud_model::{authority::StreamBinding, identity::*, journal::*};
use nocterm_cloud_runner::JournalLimits;
use std::sync::Arc;
use support::*;

#[test]
fn durable_admission_checkpoint_and_exact_outbox_survive_reopen() {
    let root = private_root();
    let cipher = Arc::new(TestCipher::default());
    let store = journal(root.path(), cipher.clone());
    let lease = claim(&store, None);
    let position = commit(
        &store,
        admission(&lease, JournalPosition::default(), acceptance(1, 0)),
    );
    drop(store);
    let store = journal(root.path(), cipher);
    let page = block_on(store.read(stream(), JournalSequence::default(), limits())).unwrap();
    assert_eq!(page.head(), position);
    assert_eq!(page.records().len(), 1);
    assert_eq!(
        page.records()[0].record().payload().as_bytes(),
        b"private prompt plaintext"
    );
    let checkpoint = block_on(store.checkpoint(stream())).unwrap().unwrap();
    assert_eq!(checkpoint.journal_revision(), position.revision());
    let outbox = block_on(store.outbox(stream(), EventSequence::default(), limits())).unwrap();
    assert_eq!(outbox.len(), 1);
    assert_eq!(outbox[0].sealed().as_bytes(), b"exact sealed event");
    let disk = std::fs::read(root.path().join("cloud-runs.sqlite3")).unwrap();
    assert!(
        !disk
            .windows(b"private prompt plaintext".len())
            .any(|part| part == b"private prompt plaintext")
    );
    assert!(
        !disk
            .windows(b"private engine checkpoint".len())
            .any(|part| part == b"private engine checkpoint")
    );
}

#[test]
fn lost_ack_retry_returns_original_receipt_after_writer_renewal() {
    let root = private_root();
    let store = journal(root.path(), Arc::new(TestCipher::default()));
    let lease = claim(&store, None);
    let batch = admission(&lease, JournalPosition::default(), acceptance(1, 0));
    let expected = commit(&store, batch.clone());
    let renewed = claim(&store, Some(lease.epoch()));
    assert_ne!(renewed.epoch(), lease.epoch());
    let CommitOutcome::Duplicate(receipt) = block_on(store.commit(batch)).unwrap() else {
        panic!("duplicate expected")
    };
    assert_eq!(receipt.position(), expected);
    assert_eq!(receipt.lease(), &lease);
}

#[test]
fn control_inspection_returns_original_receipts_and_rechecks_the_actual_writer() {
    let root = private_root();
    let cipher = Arc::new(TestCipher::default());
    let store = journal(root.path(), cipher.clone());
    assert!(block_on(store.head(stream())).unwrap().is_none());
    let lease = claim(&store, None);
    store.require_current(&lease).unwrap();
    let original = acceptance(1, 0);
    let position = commit(
        &store,
        admission(&lease, JournalPosition::default(), original.clone()),
    );
    let other = journal(root.path(), cipher);
    let renewed = claim(&other, Some(lease.epoch()));
    assert!(matches!(
        store.require_current(&lease),
        Err(JournalError::Fenced)
    ));
    other.require_current(&renewed).unwrap();
    let head = block_on(store.head(stream())).unwrap().unwrap();
    assert_eq!(head.lease(), &renewed);
    assert_eq!(head.position(), position);
    let receipt = block_on(store.command_receipt(stream(), original.id()))
        .unwrap()
        .unwrap();
    assert_eq!(receipt.lease(), &lease);
    assert_eq!(receipt.position(), position);
    assert_eq!(receipt.acceptance(), &original);
    let checkpoint = block_on(EngineCheckpointReader::checkpoint(&store, stream()))
        .unwrap()
        .unwrap();
    assert_eq!(checkpoint.journal_revision(), position.revision());
}

#[test]
fn changed_command_content_conflicts_even_when_control_revision_is_stale() {
    let root = private_root();
    let store = journal(root.path(), Arc::new(TestCipher::default()));
    let lease = claim(&store, None);
    commit(
        &store,
        admission(&lease, JournalPosition::default(), acceptance(1, 0)),
    );
    let original = acceptance(1, 0);
    let changed = CommandAcceptance::new(
        original.id(),
        original.kind(),
        fp(90),
        original.issuer().clone(),
        original.expected_control(),
    );
    assert!(matches!(
        block_on(store.commit(admission(&lease, JournalPosition::default(), changed))),
        Err(JournalError::Conflict)
    ));
}

#[test]
fn independent_writer_claim_fences_the_old_connection() {
    let root = private_root();
    let cipher = Arc::new(TestCipher::default());
    let first = journal(root.path(), cipher.clone());
    let lease = claim(&first, None);
    let second = journal(root.path(), cipher);
    let renewed = claim(&second, Some(lease.epoch()));
    assert!(matches!(
        block_on(first.commit(admission(
            &lease,
            JournalPosition::default(),
            acceptance(1, 0)
        ))),
        Err(JournalError::Fenced)
    ));
    commit(
        &second,
        admission(&renewed, JournalPosition::default(), acceptance(1, 0)),
    );
}

#[test]
fn runner_or_key_epoch_change_cannot_fork_the_same_chat() {
    let root = private_root();
    let store = journal(root.path(), Arc::new(TestCipher::default()));
    claim(&store, None);
    let original = stream();
    for (runner, epoch) in [
        (
            RunnerId::from_bytes([99; 16]),
            RunnerKeyEpoch::new(1).unwrap(),
        ),
        (original.runner(), RunnerKeyEpoch::new(2).unwrap()),
    ] {
        let other = StreamBinding::new(original.scope().clone(), runner, epoch, original.chat());
        assert!(matches!(
            block_on(store.claim_writer(
                WriterClaim::new(other, None, WriterEpoch::new(1).unwrap()).unwrap()
            )),
            Err(JournalError::Fenced)
        ));
    }
}

#[test]
fn only_one_distinct_command_can_win_the_same_expected_control_revision() {
    let root = private_root();
    let store = journal(root.path(), Arc::new(TestCipher::default()));
    let lease = claim(&store, None);
    let position = commit(
        &store,
        admission(&lease, JournalPosition::default(), acceptance(1, 0)),
    );
    assert!(matches!(
        block_on(store.commit(admission(&lease, position, acceptance(2, 0)))),
        Err(JournalError::Conflict)
    ));
    assert_eq!(
        block_on(store.read(stream(), JournalSequence::default(), limits()))
            .unwrap()
            .head(),
        position
    );
}

#[test]
fn cipher_failure_rolls_back_command_records_checkpoint_and_outbox() {
    let root = private_root();
    let cipher = Arc::new(TestCipher::default());
    let store = journal(root.path(), cipher.clone());
    let lease = claim(&store, None);
    cipher.fail_sealing(true);
    assert!(matches!(
        block_on(store.commit(admission(
            &lease,
            JournalPosition::default(),
            acceptance(1, 0)
        ))),
        Err(JournalError::Corrupt)
    ));
    cipher.fail_sealing(false);
    let page = block_on(store.read(stream(), JournalSequence::default(), limits())).unwrap();
    assert_eq!(page.head(), JournalPosition::default());
    assert!(page.records().is_empty());
    assert!(block_on(store.checkpoint(stream())).unwrap().is_none());
    assert!(
        block_on(store.outbox(stream(), EventSequence::default(), limits()))
            .unwrap()
            .is_empty()
    );
    commit(
        &store,
        admission(&lease, JournalPosition::default(), acceptance(1, 0)),
    );
}

#[test]
fn storage_quota_refuses_before_any_admission() {
    let root = private_root();
    let store = open_journal(
        root.path(),
        Arc::new(TestCipher::default()),
        JournalLimits::new(1024 * 1024, 1024, 100, 10).unwrap(),
    )
    .unwrap();
    let lease = claim(&store, None);
    assert!(matches!(
        block_on(store.commit(admission(
            &lease,
            JournalPosition::default(),
            acceptance(1, 0)
        ))),
        Err(JournalError::Capacity)
    ));
    assert_eq!(
        block_on(store.read(stream(), JournalSequence::default(), limits()))
            .unwrap()
            .head(),
        JournalPosition::default()
    );
}

#[test]
fn small_read_budget_refuses_an_oversized_first_record() {
    let root = private_root();
    let store = journal(root.path(), Arc::new(TestCipher::default()));
    let lease = claim(&store, None);
    commit(
        &store,
        admission(&lease, JournalPosition::default(), acceptance(1, 0)),
    );
    assert!(matches!(
        block_on(store.read(
            stream(),
            JournalSequence::default(),
            JournalReadLimits::new(1, 10).unwrap()
        )),
        Err(JournalError::Capacity)
    ));
}

#[test]
fn event_cursor_is_independent_when_one_record_publishes_multiple_events() {
    let root = private_root();
    let store = journal(root.path(), Arc::new(TestCipher::default()));
    let lease = claim(&store, None);
    let base = admission(&lease, JournalPosition::default(), acceptance(1, 0));
    let mut outbox = base.outbox().to_vec();
    outbox.push(fixture_event(
        &lease,
        JournalPosition::new(
            StreamRevision::from_u64(1),
            ChatControlRevision::from_u64(1),
            JournalSequence::from_u64(1),
            EventSequence::from_u64(2),
        ),
        EventId::from_bytes([2; 16]),
        Some(run()),
        bytes(b"second sealed event"),
    ));
    let batch = JournalBatch::new(
        lease,
        base.expected(),
        base.command().cloned(),
        base.records().to_vec(),
        base.checkpoint().cloned(),
        outbox,
    )
    .unwrap();
    let position = commit(&store, batch);
    assert_eq!(position.records().get(), 1);
    assert_eq!(position.events().get(), 2);
    assert_eq!(
        block_on(store.read(stream(), JournalSequence::default(), limits()))
            .unwrap()
            .head(),
        position
    );
    assert_eq!(
        block_on(store.outbox(stream(), EventSequence::default(), limits()))
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn replacing_derived_checkpoint_releases_its_previous_storage_charge() {
    let root = private_root();
    let store = open_journal(
        root.path(),
        Arc::new(TestCipher::default()),
        JournalLimits::new(4 * 1024 * 1024, 300 * 1024, 100, 10).unwrap(),
    )
    .unwrap();
    let lease = claim(&store, None);
    let mut position = JournalPosition::default();
    for index in 0..6 {
        let base = admission(&lease, position, acceptance(index + 1, u64::from(index)));
        let checkpoint = nocterm_cloud_model::engine::EngineCheckpoint::new(
            run(),
            NamespaceId::new("test-engine/1").unwrap(),
            position.revision().next().unwrap(),
            bytes(&vec![index; 128 * 1024]),
        )
        .unwrap();
        position = commit(
            &store,
            JournalBatch::new(
                lease.clone(),
                position,
                base.command().cloned(),
                base.records().to_vec(),
                Some(checkpoint),
                base.outbox().to_vec(),
            )
            .unwrap(),
        );
    }
    assert_eq!(position.records().get(), 6);
}

#[test]
fn writer_fence_reads_committed_authority_while_the_same_driver_is_publishing() {
    let root = private_root();
    let cipher = Arc::new(TestCipher::default());
    let store = journal(root.path(), cipher.clone());
    let lease = claim(&store, None);
    let batch = admission(&lease, JournalPosition::default(), acceptance(1, 0));
    let (entered, release) = cipher.pause_next_seal();
    let writer = store.clone();
    let work = std::thread::spawn(move || block_on(writer.commit(batch)));
    entered
        .recv_timeout(std::time::Duration::from_secs(5))
        .unwrap();
    let authority = store.require_current(&lease);
    release.send(()).unwrap();
    work.join().unwrap().unwrap();
    authority.unwrap();
    store.require_current(&lease).unwrap();
}

#[test]
fn a_busy_authority_reader_refuses_without_treating_contention_as_a_valid_writer() {
    let root = private_root();
    let store = journal(root.path(), Arc::new(TestCipher::default()));
    let lease = claim(&store, None);
    let blocker = rusqlite::Connection::open(root.path().join("cloud-runs.sqlite3")).unwrap();
    blocker.execute_batch("BEGIN EXCLUSIVE").unwrap();
    let result = store.require_current(&lease);
    blocker.execute_batch("ROLLBACK").unwrap();
    assert!(matches!(result, Err(JournalError::Busy)));
    store.require_current(&lease).unwrap();
}

#[test]
fn writer_and_readonly_fence_share_head_rollback_highwater() {
    let root = private_root();
    let store = journal(root.path(), Arc::new(TestCipher::default()));
    let lease = claim(&store, None);
    let database = rusqlite::Connection::open(root.path().join("cloud-runs.sqlite3")).unwrap();
    let old = database
        .query_row(
            "SELECT writer,revision,records,events,control,retained,deleted,charge,body FROM chats",
            [],
            |row| {
                (0..9)
                    .map(|index| row.get::<_, rusqlite::types::Value>(index))
                    .collect::<Result<Vec<_>, _>>()
            },
        )
        .unwrap();
    commit(
        &store,
        admission(&lease, JournalPosition::default(), acceptance(1, 0)),
    );
    database.execute("UPDATE chats SET writer=?1,revision=?2,records=?3,events=?4,control=?5,retained=?6,deleted=?7,charge=?8,body=?9", rusqlite::params_from_iter(old)).unwrap();
    assert!(matches!(
        store.require_current(&lease),
        Err(JournalError::Corrupt)
    ));
    assert!(matches!(
        block_on(store.head(stream())),
        Err(JournalError::Corrupt)
    ));
}
