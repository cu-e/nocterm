mod support;
use nocterm_ai::TerminalCall;
use nocterm_cloud_model::{
    engine::EngineCheckpoint, identity::*, journal::*, payload::OpaqueBytes, run::*,
};
use support::*;

fn observed_head(
    chat: u8,
    head: Option<HeadAnchor>,
    floor: u64,
    deleted: bool,
) -> Result<CurrentChatHead, JournalError> {
    CurrentChatHead::new(
        run(1, chat, 1, 1).stream(),
        ChatKeyEpoch::new(7).unwrap(),
        CurrentHeadFacts {
            writer: Some(WriterEpoch::new(2).unwrap()),
            control: ChatControlRevision::from_u64(u64::from(head.is_some())),
            deleted,
            head,
            event_floor: EventSequence::from_u64(floor),
            snapshot: None,
        },
    )
}
fn observed_anchor(kind: HeadAnchorKind, id: u8) -> HeadAnchor {
    HeadAnchor::new(
        kind,
        HeadRecordPosition::new(
            WriterEpoch::new(1).unwrap(),
            EventSequence::from_u64(1),
            JournalSequence::from_u64(2),
            StreamRevision::from_u64(2),
        )
        .unwrap(),
        EventId::from_bytes([id; 16]),
        Some(run(1, 1, 1, 1)),
        fingerprint(8),
    )
}

#[test]
fn current_heads_keep_actual_material_generation_and_original_event_writer() {
    let empty = observed_head(1, None, 0, false).unwrap();
    assert!(empty.facts().head.is_none());
    assert_eq!(empty.key_epoch().get(), 7);
    assert_eq!(empty.stream().runner_epoch().get(), 1);
    let current =
        observed_head(1, Some(observed_anchor(HeadAnchorKind::Event, 3)), 1, false).unwrap();
    assert_eq!(current.facts().writer.unwrap().get(), 2);
    assert_eq!(
        current
            .facts()
            .head
            .as_ref()
            .unwrap()
            .position()
            .writer()
            .get(),
        1
    );
}

#[test]
fn current_heads_require_real_snapshot_anchors_for_retained_gaps() {
    let head = observed_anchor(HeadAnchorKind::Event, 3);
    assert!(observed_head(1, Some(head.clone()), 2, false).is_err());
    // Key retirement may leave a tombstone and a floor after its exact last event.
    assert!(observed_head(1, Some(head.clone()), 2, true).is_ok());
    // A different snapshot id at the same stream position cannot replace a head.
    let mut facts = observed_head(1, Some(head), 1, false)
        .unwrap()
        .facts()
        .clone();
    facts.snapshot = Some(observed_anchor(HeadAnchorKind::Snapshot, 4));
    assert!(
        CurrentChatHead::new(
            run(1, 1, 1, 1).stream(),
            ChatKeyEpoch::new(7).unwrap(),
            facts
        )
        .is_err()
    );
    // Matching descriptive run ids in another chat do not bind an anchor to it.
    assert!(observed_head(2, Some(observed_anchor(HeadAnchorKind::Event, 3)), 1, false).is_err());
}

#[test]
fn current_head_paging_binds_the_complete_revision_root_and_original_source() {
    let stream = run(1, 1, 1, 1).stream();
    let binding = JournalCatalogBinding::new(
        stream.scope().clone(),
        stream.runner(),
        stream.runner_epoch(),
    );
    let first = CurrentHeadsPage::new(
        binding.clone(),
        4,
        fingerprint(5),
        2,
        None,
        vec![observed_head(1, None, 0, false).unwrap()],
        true,
    )
    .unwrap();
    let cursor = first.next_cursor().unwrap();
    assert_eq!(cursor.seen(), 1);
    assert!(
        CurrentHeadsPage::new(
            binding.clone(),
            4,
            fingerprint(5),
            2,
            Some(cursor.clone()),
            vec![observed_head(2, None, 0, false).unwrap()],
            false
        )
        .unwrap()
        .next_cursor()
        .is_none()
    );
    for (revision, root) in [(5, fingerprint(5)), (4, fingerprint(6))] {
        assert!(
            CurrentHeadsPage::new(
                binding.clone(),
                revision,
                root,
                2,
                Some(cursor.clone()),
                vec![observed_head(2, None, 0, false).unwrap()],
                false
            )
            .is_err()
        );
    }
    assert!(
        CurrentHeadsPage::new(
            binding.clone(),
            4,
            fingerprint(5),
            2,
            Some(cursor),
            vec![observed_head(1, None, 0, false).unwrap()],
            false
        )
        .is_err()
    );
    let foreign = CurrentChatHead::new(
        run(2, 1, 1, 1).stream(),
        ChatKeyEpoch::new(7).unwrap(),
        observed_head(1, None, 0, false).unwrap().facts().clone(),
    )
    .unwrap();
    assert!(
        CurrentHeadsPage::new(binding, 4, fingerprint(5), 1, None, vec![foreign], false).is_err()
    );
}

fn bytes() -> OpaqueBytes {
    OpaqueBytes::new(vec![1, 2, 3]).unwrap()
}
fn lease() -> WriterLease {
    WriterLease::new(run(1, 1, 1, 1).stream(), WriterEpoch::new(1).unwrap())
}
fn issuer() -> CommandIssuer {
    CommandIssuer::new(
        NamespaceId::new("device").unwrap(),
        fingerprint(1),
        fingerprint(2),
    )
}
fn command(id: u8, digest: u8, expected: u64) -> CommandAcceptance {
    CommandAcceptance::new(
        CommandId::from_bytes([id; 16]),
        CommandKind::SubmitPrompt,
        fingerprint(digest),
        issuer(),
        ChatControlRevision::from_u64(expected),
    )
}
fn accepted(command: &CommandAcceptance) -> JournalRecord {
    JournalRecord::new(
        RecordContext::Run(run(1, 1, 1, 1)),
        JournalRecordKind::CommandAccepted {
            id: command.id(),
            kind: command.kind(),
        },
        bytes(),
    )
    .unwrap()
}
fn published(event: u64, record: u64, id: u8) -> ResidentPublication {
    let entry = OutboxEntry::new(
        HeadAnchor::new(
            HeadAnchorKind::Event,
            HeadRecordPosition::new(
                WriterEpoch::new(1).unwrap(),
                EventSequence::from_u64(event),
                JournalSequence::from_u64(record),
                StreamRevision::from_u64(1),
            )
            .unwrap(),
            EventId::from_bytes([id; 16]),
            None,
            fingerprint(0),
        ),
        ChatKeyEpoch::new(7).unwrap(),
        OutboxDeliveryId::from_bytes([id ^ 0xd1; 16]),
        bytes(),
    );
    let work = PublicationBudget::maintenance(
        nocterm_cloud_model::coordinator::IntakeLimits::default()
            .pool(nocterm_cloud_model::coordinator::IntakeLane::Ordinary),
    );
    ResidentPublication::new(
        lease().stream().clone(),
        entry,
        work.try_reserve(PublicationAllocation::new(128 * 1024, 1).unwrap())
            .unwrap(),
    )
    .unwrap()
}

fn admission(command: CommandAcceptance, expected: JournalPosition) -> JournalBatch {
    JournalBatch::new(
        lease(),
        expected,
        Some(command.clone()),
        vec![accepted(&command)],
        None,
        vec![published(
            expected.events().get() + 1,
            expected.records().get() + 1,
            1,
        )],
    )
    .unwrap()
}
fn budget() -> JournalRecord {
    JournalRecord::new(
        RecordContext::Run(run(1, 1, 1, 1)),
        JournalRecordKind::BudgetReserved {
            step: StepId::from_bytes([1; 16]),
            attempt: AttemptId::from_bytes([1; 16]),
        },
        bytes(),
    )
    .unwrap()
}

#[test]
fn internal_budget_and_progress_commits_do_not_stale_user_control_admission() {
    let internal = JournalBatch::new(
        lease(),
        JournalPosition::default(),
        None,
        vec![budget()],
        None,
        vec![],
    )
    .unwrap();
    let head = internal.next_position().unwrap();
    assert_eq!(head.revision(), StreamRevision::from_u64(1));
    assert_eq!(head.records(), JournalSequence::from_u64(1));
    assert_eq!(head.events(), EventSequence::default());
    assert_eq!(head.control(), ChatControlRevision::default());
    let request = command(1, 1, 0);
    assert!(matches!(
        classify_command(lease().stream(), None, &request, head.control()),
        Ok(CommandAdmission::New)
    ));
    let next = admission(request, head).next_position().unwrap();
    assert_eq!(next.control(), ChatControlRevision::from_u64(1));
    assert_eq!(next.events(), EventSequence::from_u64(1));
    assert_eq!(next.records(), JournalSequence::from_u64(2));
}

#[test]
fn accepted_command_record_and_ciphertext_delivery_are_one_structural_batch() {
    let request = command(1, 1, 0);
    assert!(matches!(
        JournalBatch::new(
            lease(),
            JournalPosition::default(),
            Some(request.clone()),
            vec![accepted(&request)],
            None,
            vec![]
        ),
        Err(JournalError::InvalidBatch)
    ));
    assert!(matches!(
        JournalBatch::new(
            lease(),
            JournalPosition::default(),
            None,
            vec![accepted(&request)],
            None,
            vec![published(1, 1, 1)]
        ),
        Err(JournalError::InvalidBatch)
    ));
    let other = command(2, 1, 0);
    assert!(matches!(
        JournalBatch::new(
            lease(),
            JournalPosition::default(),
            Some(request),
            vec![accepted(&other)],
            None,
            vec![published(1, 1, 1)]
        ),
        Err(JournalError::InvalidBatch)
    ));
}

#[test]
fn duplicate_receipt_precedes_stale_control_check_but_substitution_always_conflicts() {
    let request = command(1, 1, 0);
    let committed =
        JournalCommitReceipt::committed(&admission(request.clone(), JournalPosition::default()))
            .unwrap();
    let previous = committed.command().unwrap();
    let current = ChatControlRevision::from_u64(100);
    let duplicate = classify_command(lease().stream(), Some(previous), &request, current).unwrap();
    let CommandAdmission::Duplicate(receipt) = duplicate else {
        panic!("original receipt expected")
    };
    assert_eq!(receipt.position(), previous.position());
    assert!(matches!(
        classify_command(lease().stream(), Some(previous), &command(1, 2, 0), current),
        Err(JournalError::Conflict)
    ));
    assert!(matches!(
        classify_command(lease().stream(), None, &command(2, 1, 0), current),
        Err(JournalError::Conflict)
    ));
    assert!(matches!(
        classify_command(&run(2, 1, 1, 1).stream(), Some(previous), &request, current),
        Err(JournalError::Corrupt)
    ));
}

#[test]
fn only_one_new_command_can_admit_against_the_same_control_revision() {
    let first = admission(command(1, 1, 0), JournalPosition::default());
    let head = JournalCommitReceipt::committed(&first).unwrap().position();
    assert!(matches!(
        classify_command(lease().stream(), None, &command(2, 2, 0), head.control()),
        Err(JournalError::Conflict)
    ));
    assert!(matches!(
        classify_command(lease().stream(), None, &command(2, 2, 1), head.control()),
        Ok(CommandAdmission::New)
    ));
}

#[test]
fn outbox_references_cannot_skip_positions_or_substitute_duplicate_event_ids() {
    assert!(
        HeadRecordPosition::new(
            WriterEpoch::new(1).unwrap(),
            EventSequence::from_u64(1),
            JournalSequence::default(),
            StreamRevision::from_u64(1)
        )
        .is_err()
    );
    let request = command(1, 1, 0);
    for entries in [
        vec![published(2, 1, 1)],
        vec![published(1, 2, 1)],
        vec![published(1, 1, 1), published(2, 1, 1)],
    ] {
        assert!(matches!(
            JournalBatch::new(
                lease(),
                JournalPosition::default(),
                Some(request.clone()),
                vec![accepted(&request)],
                None,
                entries
            ),
            Err(JournalError::InvalidBatch)
        ));
    }
}

#[test]
fn checkpoint_must_match_its_original_run_and_the_committed_stream_position() {
    let request = command(1, 1, 0);
    for (scope, revision) in [(1, 0), (2, 1)] {
        let checkpoint = EngineCheckpoint::new(
            run(scope, 1, 1, 1),
            NamespaceId::new("fixture-v1").unwrap(),
            StreamRevision::from_u64(revision),
            bytes(),
        )
        .unwrap();
        assert!(matches!(
            JournalBatch::new(
                lease(),
                JournalPosition::default(),
                Some(request.clone()),
                vec![accepted(&request)],
                Some(checkpoint),
                vec![published(1, 1, 1)]
            ),
            Err(JournalError::InvalidBatch)
        ));
    }
    let checkpoint = EngineCheckpoint::new(
        run(1, 1, 1, 1),
        NamespaceId::new("fixture-v1").unwrap(),
        StreamRevision::from_u64(1),
        bytes(),
    )
    .unwrap();
    assert!(
        JournalBatch::new(
            lease(),
            JournalPosition::default(),
            Some(request.clone()),
            vec![accepted(&request)],
            Some(checkpoint),
            vec![published(1, 1, 1)]
        )
        .is_ok()
    );
}

fn intent_record(account: u8, writer: u64, revision: u64) -> JournalRecord {
    let binding = run(account, 1, 1, 1);
    let intent = ToolIntent::new(
        IntentId::from_bytes([1; 16]),
        StepId::from_bytes([1; 16]),
        authority(binding.clone()),
        fingerprint(1),
        &TerminalCall::ListTerminals,
    )
    .unwrap();
    JournalRecord::new(
        RecordContext::Transition(CommitStamp::new(
            binding,
            WriterEpoch::new(writer).unwrap(),
            Revision::from_u64(revision),
        )),
        JournalRecordKind::Intent(Box::new(intent)),
        bytes(),
    )
    .unwrap()
}

#[test]
fn per_run_commits_never_borrow_stream_revisions_or_another_account_writer() {
    for (record, expected) in [
        (intent_record(2, 1, 1), false),
        (intent_record(1, 2, 1), true),
    ] {
        let result = JournalBatch::new(
            lease(),
            JournalPosition::default(),
            None,
            vec![record],
            None,
            vec![published(1, 1, 1)],
        );
        if expected {
            assert!(matches!(result, Err(JournalError::Fenced)));
        } else {
            assert!(matches!(result, Err(JournalError::InvalidBatch)));
        }
    }
    let position = JournalPosition::new(
        StreamRevision::from_u64(100),
        ChatControlRevision::from_u64(9),
        JournalSequence::from_u64(120),
        EventSequence::from_u64(80),
    );
    let batch = JournalBatch::new(
        lease(),
        position,
        None,
        vec![intent_record(1, 1, 1)],
        None,
        vec![published(81, 121, 1)],
    )
    .unwrap();
    let receipt = JournalCommitReceipt::committed(&batch).unwrap();
    assert_eq!(receipt.position().revision(), StreamRevision::from_u64(101));
    assert_eq!(
        receipt.position().control(),
        ChatControlRevision::from_u64(9)
    );
    assert_eq!(receipt.run_commits()[0].revision(), Revision::from_u64(1));
}

#[test]
fn writer_claims_and_batch_limits_refuse_invalid_or_wrapping_coordinates() {
    assert!(matches!(
        WriterClaim::new(lease().stream().clone(), None, WriterEpoch::new(2).unwrap()),
        Err(JournalError::Fenced)
    ));
    assert!(matches!(
        WriterClaim::new(
            lease().stream().clone(),
            Some(WriterEpoch::new(1).unwrap()),
            WriterEpoch::new(3).unwrap()
        ),
        Err(JournalError::Fenced)
    ));
    assert!(
        WriterClaim::new(
            lease().stream().clone(),
            Some(WriterEpoch::new(1).unwrap()),
            WriterEpoch::new(2).unwrap()
        )
        .is_ok()
    );
    assert!(matches!(
        JournalBatch::new(
            lease(),
            JournalPosition::default(),
            None,
            vec![budget(); MAX_JOURNAL_RECORDS + 1],
            None,
            vec![]
        ),
        Err(JournalError::Capacity)
    ));
    assert!(matches!(
        JournalPosition::new(
            StreamRevision::from_u64(u64::MAX),
            ChatControlRevision::default(),
            JournalSequence::default(),
            EventSequence::default()
        )
        .next(1, 0, false),
        Err(JournalError::Identity(IdentityError::Exhausted))
    ));
}

fn stored(account: u8, sequence: u64) -> StoredRecord {
    StoredRecord::new(
        run(account, 1, 1, 1).stream(),
        JournalSequence::from_u64(sequence),
        StreamRevision::from_u64(1),
        JournalRecord::new(
            RecordContext::Run(run(account, 1, 1, 1)),
            JournalRecordKind::Progress {
                step: StepId::from_bytes([1; 16]),
            },
            bytes(),
        )
        .unwrap(),
    )
    .unwrap()
}

#[test]
fn scoped_reader_refuses_gap_cross_owner_and_empty_truncated_prefixes() {
    let head = JournalPosition::new(
        StreamRevision::from_u64(1),
        ChatControlRevision::default(),
        JournalSequence::from_u64(2),
        EventSequence::default(),
    );
    let limit = JournalReadLimits::new(10, 1024 * 1024).unwrap();
    assert!(matches!(
        JournalPage::new(
            lease().stream().clone(),
            JournalSequence::default(),
            head,
            vec![stored(1, 2)],
            limit
        ),
        Err(JournalError::Gap { .. })
    ));
    assert!(matches!(
        JournalPage::new(
            lease().stream().clone(),
            JournalSequence::default(),
            head,
            vec![stored(2, 1)],
            limit
        ),
        Err(JournalError::Corrupt)
    ));
    assert!(matches!(
        JournalPage::new(
            lease().stream().clone(),
            JournalSequence::default(),
            head,
            vec![],
            limit
        ),
        Err(JournalError::Gap { .. })
    ));
    let page = JournalPage::new(
        lease().stream().clone(),
        JournalSequence::default(),
        head,
        vec![stored(1, 1), stored(1, 2)],
        limit,
    )
    .unwrap();
    assert_eq!(page.next(), head.records());
    assert_eq!(page.records().len(), 2);
}

#[test]
fn reader_capacity_and_known_head_rollback_are_explicit_refusals() {
    assert!(matches!(
        JournalReadLimits::new(0, 1),
        Err(JournalError::Capacity)
    ));
    assert!(matches!(
        JournalReadLimits::new(1, MAX_TRANSACTION_BYTES + 1),
        Err(JournalError::Capacity)
    ));
    let tiny = JournalReadLimits::new(1, 1).unwrap();
    let head = JournalPosition::new(
        StreamRevision::from_u64(1),
        ChatControlRevision::default(),
        JournalSequence::from_u64(1),
        EventSequence::default(),
    );
    assert!(matches!(
        JournalPage::new(
            lease().stream().clone(),
            JournalSequence::default(),
            head,
            vec![stored(1, 1)],
            tiny
        ),
        Err(JournalError::Capacity)
    ));
    assert!(matches!(
        JournalPage::new(
            lease().stream().clone(),
            JournalSequence::from_u64(2),
            head,
            vec![],
            tiny
        ),
        Err(JournalError::Gap { .. })
    ));
}
