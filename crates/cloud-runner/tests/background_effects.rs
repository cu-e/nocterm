#[path = "background_effects/current_heads.rs"]
mod current_heads;
#[path = "background_effects/publication_capacity.rs"]
mod publication_capacity;
mod support;
use futures::executor::block_on;
use nocterm_ai::{ExecCommand, TerminalCall};
use nocterm_cloud_model::{authority::*, identity::*, journal::*, run::*, tool::*};
use nocterm_cloud_runner::DurableRunJournal;
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
use support::*;

#[derive(Default)]
struct TestEvents {
    next: AtomicU64,
}
impl JournalEventEncoder for TestEvents {
    fn encode(
        &self,
        lease: &WriterLease,
        next: JournalPosition,
        command: &BackgroundCommand,
        _: BackgroundRecordOutcome,
        work: &dyn PublicationWorkBudget,
    ) -> Result<ResidentPublication, JournalError> {
        let value = self.next.fetch_add(1, Ordering::SeqCst) + 1;
        let mut id = [0xa1; 16];
        id[8..].copy_from_slice(&value.to_be_bytes());
        nocterm_cloud_model::testing::fixture_outbox_with_work(
            lease,
            next,
            EventId::from_bytes(id),
            Some(command.dispatch().intent().authority().run().clone()),
            bytes(&value.to_be_bytes()),
            work,
        )
    }
}
fn target() -> TargetBinding {
    TargetBinding::new(
        fp(20),
        fp(21),
        NamespaceId::new("principal").unwrap(),
        fp(22),
    )
}
fn state(run_id: u8, lease: &WriterLease) -> RunState {
    let original = run();
    let run = RunBinding::new(
        original.scope().clone(),
        original.runner(),
        original.runner_epoch(),
        original.chat(),
        RunId::from_bytes([run_id; 16]),
        original.epoch(),
    );
    RunState::new(run, target(), lease.epoch(), Revision::default())
}
fn record(
    store: &DurableRunJournal,
    lease: &WriterLease,
    position: JournalPosition,
    stamp: CommitStamp,
    kind: JournalRecordKind,
) -> JournalPosition {
    let record =
        JournalRecord::new(RecordContext::Transition(stamp), kind, bytes(b"effect")).unwrap();
    let id = EventId::from_bytes(
        (position.records().get() + 1)
            .to_be_bytes()
            .repeat(2)
            .try_into()
            .unwrap(),
    );
    let outbox = fixture_event(
        lease,
        position.next(1, 1, false).unwrap(),
        id,
        record.context().run().cloned(),
        bytes(b"sealed effect"),
    );
    commit(
        store,
        JournalBatch::new(
            lease.clone(),
            position,
            None,
            vec![record],
            None,
            vec![outbox],
        )
        .unwrap(),
    )
}
fn stamp(state: &RunState) -> CommitStamp {
    CommitStamp::new(
        state.identity().clone(),
        state.writer(),
        state.revision().next().unwrap(),
    )
}
fn planned_intent(state: &RunState, id: u8) -> ToolIntent {
    let authority = ExecutionAuthority::new(
        state.identity().clone(),
        SessionReceipt::new(
            RemoteSessionId::from_bytes([30; 16]),
            SessionEpoch::new(1).unwrap(),
            target(),
            AuthenticationBinding::new(fp(31), fp(32)),
        ),
        GrantId::from_bytes([33; 16]),
        GrantEpoch::new(1).unwrap(),
    );
    let call = TerminalCall::ExecCommand(ExecCommand {
        terminal_id: "terminal".into(),
        program: "sleep".into(),
        args: vec!["10".into()],
        stdin: None,
        timeout_ms: None,
        yield_ms: None,
    });
    ToolIntent::new(
        IntentId::from_bytes([id; 16]),
        StepId::from_bytes([id; 16]),
        authority,
        fp(id),
        &call,
    )
    .unwrap()
}
fn dispatch(
    store: &DurableRunJournal,
    lease: &WriterLease,
    state: &mut RunState,
    position: &mut JournalPosition,
    id: u8,
) -> BackgroundCommand {
    let intent = planned_intent(state, id);
    state
        .prepare_intent(intent.clone(), state.revision())
        .unwrap();
    let accepted = stamp(state);
    *position = record(
        store,
        lease,
        *position,
        accepted.clone(),
        JournalRecordKind::Intent(Box::new(intent.clone())),
    );
    state.intent_committed(intent.id(), accepted).unwrap();
    let dispatched = stamp(state);
    *position = record(
        store,
        lease,
        *position,
        dispatched.clone(),
        JournalRecordKind::Dispatched {
            intent: intent.id(),
        },
    );
    let permit = state.dispatch_committed(intent.id(), dispatched).unwrap();
    BackgroundCommand::new(permit, "terminal".into(), format!("job-{id}")).unwrap()
}
fn current(store: &DurableRunJournal) -> JournalPosition {
    block_on(store.read(stream(), JournalSequence::default(), limits()))
        .unwrap()
        .head()
}
fn pending(store: &DurableRunJournal) -> Vec<UnresolvedTargetEffect> {
    block_on(store.unresolved_target(stream().scope().clone(), target(), 16)).unwrap()
}

#[test]
fn starting_receipt_does_not_resolve_the_background_mutation() {
    let root = private_root();
    let store = journal(root.path(), Arc::new(TestCipher::default()))
        .with_background_events(Arc::new(TestEvents::default()), publication_budgets())
        .unwrap();
    let lease = claim(&store, None);
    let mut state = state(1, &lease);
    let mut position = JournalPosition::default();
    let job = dispatch(&store, &lease, &mut state, &mut position, 1);
    block_on(store.retain(job.clone())).unwrap();
    position = current(&store);
    let done = stamp(&state);
    let outcome = EffectOutcome::Known(fp(100));
    record(
        &store,
        &lease,
        position,
        done.clone(),
        JournalRecordKind::Outcome {
            intent: job.dispatch().intent().id(),
            outcome,
        },
    );
    state
        .outcome_committed(job.dispatch().intent().id(), done, outcome)
        .unwrap();
    assert_eq!(pending(&store).len(), 1);
    block_on(store.finish(job, BackgroundCommandOutcome::ObservedExit(0))).unwrap();
    assert!(pending(&store).is_empty());
}

#[test]
fn retained_job_without_event_encryption_is_refused_before_dispatch_poll() {
    let root = private_root();
    let store = journal(root.path(), Arc::new(TestCipher::default()));
    let lease = claim(&store, None);
    let mut state = state(1, &lease);
    let mut position = JournalPosition::default();
    let job = dispatch(&store, &lease, &mut state, &mut position, 1);
    assert!(matches!(
        block_on(store.retain(job)),
        Err(ToolExecutionError::UnsupportedCapability)
    ));
    assert_eq!(current(&store), position);
}

#[test]
fn unknown_old_run_effect_survives_reopen_and_route_aliases() {
    let root = private_root();
    let cipher = Arc::new(TestCipher::default());
    let store = journal(root.path(), cipher.clone())
        .with_background_events(Arc::new(TestEvents::default()), publication_budgets())
        .unwrap();
    let lease = claim(&store, None);
    let mut state = state(1, &lease);
    let mut position = JournalPosition::default();
    let job = dispatch(&store, &lease, &mut state, &mut position, 1);
    block_on(store.retain(job.clone())).unwrap();
    block_on(store.finish(job, BackgroundCommandOutcome::Unknown)).unwrap();
    drop(store);
    let store = journal(root.path(), cipher);
    let original = target();
    let alias = TargetBinding::new(
        original.host(),
        fp(99),
        original.principal().clone(),
        original.workspace(),
    );
    let unresolved =
        block_on(store.unresolved_target(stream().scope().clone(), alias, 16)).unwrap();
    assert_eq!(unresolved.len(), 1);
    assert!(unresolved[0].unknown());
    assert_eq!(
        unresolved[0].intent().authority().run().run(),
        RunId::from_bytes([1; 16])
    );
    assert_eq!(unresolved[0].background(), Some(("terminal", "job-1")));
}

#[test]
fn completing_one_job_never_clears_another_old_run_fence() {
    let root = private_root();
    let store = journal(root.path(), Arc::new(TestCipher::default()))
        .with_background_events(Arc::new(TestEvents::default()), publication_budgets())
        .unwrap();
    let lease = claim(&store, None);
    let mut first = state(1, &lease);
    let mut position = JournalPosition::default();
    let a = dispatch(&store, &lease, &mut first, &mut position, 1);
    block_on(store.retain(a.clone())).unwrap();
    position = current(&store);
    let mut second = state(2, &lease);
    let b = dispatch(&store, &lease, &mut second, &mut position, 2);
    block_on(store.retain(b.clone())).unwrap();
    block_on(store.finish(a, BackgroundCommandOutcome::Unknown)).unwrap();
    block_on(store.finish(b, BackgroundCommandOutcome::ObservedExit(2))).unwrap();
    let remaining = pending(&store);
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].intent().id(), IntentId::from_bytes([1; 16]));
    assert!(remaining[0].unknown());
}

#[test]
fn job_linkage_and_observed_exit_are_idempotent_and_substitution_conflicts() {
    let root = private_root();
    let store = journal(root.path(), Arc::new(TestCipher::default()))
        .with_background_events(Arc::new(TestEvents::default()), publication_budgets())
        .unwrap();
    let lease = claim(&store, None);
    let mut state = state(1, &lease);
    let mut position = JournalPosition::default();
    let job = dispatch(&store, &lease, &mut state, &mut position, 1);
    block_on(store.retain(job.clone())).unwrap();
    let retained = current(&store);
    block_on(store.retain(job.clone())).unwrap();
    assert_eq!(current(&store), retained);
    let changed =
        BackgroundCommand::new(job.dispatch().clone(), "terminal".into(), "changed".into())
            .unwrap();
    assert!(block_on(store.retain(changed)).is_err());
    block_on(store.finish(job.clone(), BackgroundCommandOutcome::ObservedExit(7))).unwrap();
    let finished = current(&store);
    block_on(store.finish(job.clone(), BackgroundCommandOutcome::ObservedExit(7))).unwrap();
    assert_eq!(current(&store), finished);
    assert!(block_on(store.finish(job, BackgroundCommandOutcome::ObservedExit(0))).is_err());
}

#[test]
fn tampered_index_filter_cannot_hide_an_authenticated_active_effect() {
    let root = private_root();
    let store = journal(root.path(), Arc::new(TestCipher::default()))
        .with_background_events(Arc::new(TestEvents::default()), publication_budgets())
        .unwrap();
    let lease = claim(&store, None);
    let mut state = state(1, &lease);
    let mut position = JournalPosition::default();
    let job = dispatch(&store, &lease, &mut state, &mut position, 1);
    block_on(store.retain(job)).unwrap();
    let db = rusqlite::Connection::open(root.path().join("cloud-runs.sqlite3")).unwrap();
    db.execute("UPDATE effects SET state=3", []).unwrap();
    drop(db);
    assert!(matches!(
        block_on(store.unresolved_target(stream().scope().clone(), target(), 16)),
        Err(JournalError::Corrupt)
    ));
}

#[test]
fn tampered_chat_scope_cannot_hide_a_durable_unresolved_effect() {
    let root = private_root();
    let store = journal(root.path(), Arc::new(TestCipher::default()))
        .with_background_events(Arc::new(TestEvents::default()), publication_budgets())
        .unwrap();
    let lease = claim(&store, None);
    let mut state = state(1, &lease);
    let mut position = JournalPosition::default();
    let job = dispatch(&store, &lease, &mut state, &mut position, 1);
    block_on(store.retain(job)).unwrap();
    assert_eq!(pending(&store).len(), 1);
    let db = rusqlite::Connection::open(root.path().join("cloud-runs.sqlite3")).unwrap();
    db.execute("UPDATE chats SET scope=zeroblob(32)", [])
        .unwrap();
    drop(db);
    let remaining = block_on(store.unresolved_target(stream().scope().clone(), target(), 16));
    assert!(
        matches!(remaining, Err(JournalError::Corrupt)),
        "tampered routing must fail closed: {remaining:?}"
    );
}

#[test]
fn writer_replacement_reports_the_old_job_as_uncertain_and_fences_old_completion() {
    let root = private_root();
    let store = journal(root.path(), Arc::new(TestCipher::default()))
        .with_background_events(Arc::new(TestEvents::default()), publication_budgets())
        .unwrap();
    let lease = claim(&store, None);
    let mut state = state(1, &lease);
    let mut position = JournalPosition::default();
    let job = dispatch(&store, &lease, &mut state, &mut position, 1);
    block_on(store.retain(job.clone())).unwrap();
    claim(&store, Some(lease.epoch()));
    let remaining = pending(&store);
    assert_eq!(remaining.len(), 1);
    assert!(remaining[0].unknown());
    assert!(matches!(
        block_on(store.finish(job, BackgroundCommandOutcome::ObservedExit(0))),
        Err(ToolExecutionError::Revoked)
    ));
    assert_eq!(pending(&store).len(), 1);
}

#[test]
fn observed_background_exit_may_race_a_later_starting_receipt_without_reopening_its_fence() {
    let root = private_root();
    let store = journal(root.path(), Arc::new(TestCipher::default()))
        .with_background_events(Arc::new(TestEvents::default()), publication_budgets())
        .unwrap();
    let lease = claim(&store, None);
    let mut state = state(1, &lease);
    let mut position = JournalPosition::default();
    let job = dispatch(&store, &lease, &mut state, &mut position, 1);
    block_on(store.retain(job.clone())).unwrap();
    block_on(store.finish(job.clone(), BackgroundCommandOutcome::ObservedExit(0))).unwrap();
    position = current(&store);
    let done = stamp(&state);
    let outcome = EffectOutcome::Known(fp(100));
    record(
        &store,
        &lease,
        position,
        done,
        JournalRecordKind::Outcome {
            intent: job.dispatch().intent().id(),
            outcome,
        },
    );
    assert!(pending(&store).is_empty());
}

#[test]
fn a_replacement_writer_cannot_report_a_stale_foreground_success() {
    let root = private_root();
    let store = journal(root.path(), Arc::new(TestCipher::default()));
    let lease = claim(&store, None);
    let mut state = state(1, &lease);
    let mut position = JournalPosition::default();
    let job = dispatch(&store, &lease, &mut state, &mut position, 1);
    let renewed = claim(&store, Some(lease.epoch()));
    let changed = CommitStamp::new(
        state.identity().clone(),
        renewed.epoch(),
        state.revision().next().unwrap(),
    );
    let record = JournalRecord::new(
        RecordContext::Transition(changed),
        JournalRecordKind::Outcome {
            intent: job.dispatch().intent().id(),
            outcome: EffectOutcome::Known(fp(100)),
        },
        bytes(b"stale success"),
    )
    .unwrap();
    let event = fixture_event(
        &renewed,
        position.next(1, 1, false).unwrap(),
        EventId::from_bytes([99; 16]),
        record.context().run().cloned(),
        bytes(b"event"),
    );
    let result = block_on(store.commit(
        JournalBatch::new(renewed, position, None, vec![record], None, vec![event]).unwrap(),
    ));
    assert!(matches!(result, Err(JournalError::Fenced)));
    assert!(pending(&store)[0].unknown());
}

#[test]
fn a_declined_undispatched_effect_survives_reopen_without_a_dispatch_or_target_fence() {
    let root = private_root();
    let cipher = Arc::new(TestCipher::default());
    let store = journal(root.path(), cipher.clone());
    let lease = claim(&store, None);
    let mut state = state(9, &lease);
    let intent = planned_intent(&state, 9);
    state
        .prepare_intent(intent.clone(), state.revision())
        .unwrap();
    let accepted = stamp(&state);
    let position = record(
        &store,
        &lease,
        JournalPosition::default(),
        accepted.clone(),
        JournalRecordKind::Intent(Box::new(intent.clone())),
    );
    state.intent_committed(intent.id(), accepted).unwrap();
    let declined = stamp(&state);
    let position = record(
        &store,
        &lease,
        position,
        declined.clone(),
        JournalRecordKind::Declined {
            intent: intent.id(),
        },
    );
    state.decline_committed(intent.id(), declined).unwrap();
    drop(store);
    let store = journal(root.path(), cipher);
    assert!(
        block_on(store.unresolved_target(stream().scope().clone(), target(), 16))
            .unwrap()
            .is_empty()
    );
    let page = block_on(store.read(stream(), JournalSequence::default(), limits())).unwrap();
    assert!(
        matches!(page.records()[1].record().kind(), JournalRecordKind::Declined { intent: id } if *id == intent.id())
    );
    assert_eq!(page.head(), position);
    let attempted = JournalRecord::new(
        RecordContext::Transition(stamp(&state)),
        JournalRecordKind::Dispatched {
            intent: intent.id(),
        },
        bytes(b"cannot dispatch"),
    )
    .unwrap();
    let next = position.next(1, 1, false).unwrap();
    let outbox = fixture_event(
        &lease,
        next,
        EventId::from_bytes([0xa1; 16]),
        attempted.context().run().cloned(),
        bytes(b"refused dispatch"),
    );
    let batch =
        JournalBatch::new(lease, position, None, vec![attempted], None, vec![outbox]).unwrap();
    assert!(matches!(
        block_on(store.commit(batch)),
        Err(JournalError::Conflict)
    ));
    assert_eq!(
        block_on(store.head(stream())).unwrap().unwrap().position(),
        position
    );
}

fn delete_chat(store: &DurableRunJournal, lease: &WriterLease) {
    let position = current(store);
    let record = JournalRecord::new(
        RecordContext::Chat,
        JournalRecordKind::Tombstone {
            command: CommandId::from_bytes([200; 16]),
        },
        bytes(b"deleted"),
    )
    .unwrap();
    let event = fixture_event(
        lease,
        position.next(1, 1, true).unwrap(),
        EventId::from_bytes([201; 16]),
        None,
        bytes(b"sealed delete"),
    );
    commit(
        store,
        JournalBatch::new(
            lease.clone(),
            position,
            None,
            vec![record],
            None,
            vec![event],
        )
        .unwrap(),
    );
}
#[test]
fn deleted_chat_accepts_only_original_retained_terminal_cleanup_without_resurrection() {
    let root = private_root();
    let store = journal(root.path(), Arc::new(TestCipher::default()))
        .with_background_events(Arc::new(TestEvents::default()), publication_budgets())
        .unwrap();
    let lease = claim(&store, None);
    let mut state = state(1, &lease);
    let mut position = JournalPosition::default();
    let job = dispatch(&store, &lease, &mut state, &mut position, 1);
    block_on(store.retain(job.clone())).unwrap();
    delete_chat(&store, &lease);
    assert!(
        block_on(store.finish(job.clone(), BackgroundCommandOutcome::ObservedExit(0))).is_err()
    );
    let cleanup = store.cleanup();
    let view = CleanupLease::for_writer(lease.clone());
    block_on(cleanup.finish_background(
        view.clone(),
        job.clone(),
        BackgroundCommandOutcome::Unknown,
    ))
    .unwrap();
    assert_eq!(pending(&store).len(), 1);
    block_on(cleanup.finish_background(view, job, BackgroundCommandOutcome::ObservedExit(0)))
        .unwrap();
    assert!(pending(&store).is_empty());
    assert!(block_on(store.head(stream())).unwrap().unwrap().deleted());
    assert!(
        block_on(store.claim_writer(
            WriterClaim::new(stream(), Some(lease.epoch()), lease.epoch().next().unwrap()).unwrap()
        ))
        .is_err()
    );
}
#[test]
fn current_cleanup_writer_can_ack_an_observed_old_job_but_old_writer_cannot() {
    let root = private_root();
    let store = journal(root.path(), Arc::new(TestCipher::default()))
        .with_background_events(Arc::new(TestEvents::default()), publication_budgets())
        .unwrap();
    let lease = claim(&store, None);
    let mut state = state(1, &lease);
    let mut position = JournalPosition::default();
    let job = dispatch(&store, &lease, &mut state, &mut position, 1);
    block_on(store.retain(job.clone())).unwrap();
    let renewed = claim(&store, Some(lease.epoch()));
    assert!(
        block_on(store.cleanup().finish_background(
            CleanupLease::for_writer(lease),
            job.clone(),
            BackgroundCommandOutcome::ObservedExit(0)
        ))
        .is_err()
    );
    block_on(store.cleanup().finish_background(
        CleanupLease::for_writer(renewed),
        job.clone(),
        BackgroundCommandOutcome::ObservedExit(0),
    ))
    .unwrap();
    assert!(pending(&store).is_empty());
    let page = block_on(store.read(stream(), JournalSequence::default(), limits())).unwrap();
    assert_eq!(
        page.records().last().unwrap().record().context().run(),
        Some(job.dispatch().intent().authority().run())
    );
}
fn declaration(
    store: &DurableRunJournal,
    lease: CleanupLease,
    original: ToolIntent,
    id: u8,
) -> CleanupReconciliation {
    let expected = current(store);
    let command = CommandAcceptance::new(
        CommandId::from_bytes([id; 16]),
        CommandKind::ResolveApproval,
        fp(id),
        CommandIssuer::new(NamespaceId::new("device").unwrap(), fp(50), fp(51)),
        expected.control(),
    );
    let declaration = CleanupDeclaration::new(
        command,
        original,
        fp(70),
        bytes(b"verified original M admission"),
        bytes(b"account declared known; not an observed remote exit"),
    )
    .unwrap();
    let outbox = (1..=2)
        .map(|i| {
            fixture_event(
                lease.writer(),
                JournalPosition::new(
                    expected.revision().next().unwrap(),
                    expected.control().next().unwrap(),
                    expected.records().advance(i).unwrap(),
                    expected.events().advance(i).unwrap(),
                ),
                EventId::from_bytes([id + i as u8; 16]),
                Some(declaration.original().authority().run().clone()),
                bytes(b"exact sealed declaration"),
            )
        })
        .collect();
    CleanupReconciliation::new(lease, expected, declaration, outbox).unwrap()
}
#[test]
fn deleted_restart_reconciles_exact_original_intent_with_one_immutable_receipt() {
    let root = private_root();
    let cipher = Arc::new(TestCipher::default());
    let store = journal(root.path(), cipher.clone())
        .with_background_events(Arc::new(TestEvents::default()), publication_budgets())
        .unwrap();
    let lease = claim(&store, None);
    let mut state = state(1, &lease);
    let mut position = JournalPosition::default();
    let job = dispatch(&store, &lease, &mut state, &mut position, 1);
    block_on(store.retain(job.clone())).unwrap();
    delete_chat(&store, &lease);
    drop(store);
    let store = journal(root.path(), cipher);
    let cleanup = store.cleanup();
    let view = block_on(cleanup.claim_cleanup(
        WriterClaim::new(stream(), Some(lease.epoch()), lease.epoch().next().unwrap()).unwrap(),
    ))
    .unwrap();
    let intent = job.dispatch().intent();
    let wrong = ToolIntent::from_journal(
        intent.id(),
        intent.step(),
        intent.authority().clone(),
        fp(99),
        true,
    );
    assert!(block_on(cleanup.reconcile(declaration(&store, view.clone(), wrong, 210))).is_err());
    assert_eq!(pending(&store).len(), 1);
    let batch = declaration(&store, view.clone(), intent.clone(), 220);
    let first = block_on(cleanup.reconcile(batch.clone())).unwrap();
    let CommitOutcome::Committed(receipt) = first else {
        panic!("first declaration must commit")
    };
    assert!(pending(&store).is_empty());
    let next = block_on(
        cleanup.claim_cleanup(
            WriterClaim::new(
                stream(),
                Some(view.writer().epoch()),
                view.writer().epoch().next().unwrap(),
            )
            .unwrap(),
        ),
    )
    .unwrap();
    assert!(next.writer().epoch() > view.writer().epoch());
    let duplicate = block_on(cleanup.reconcile(batch)).unwrap();
    let CommitOutcome::Duplicate(original) = duplicate else {
        panic!("must return original receipt")
    };
    assert_eq!(original.position(), receipt.position());
    assert!(block_on(store.head(stream())).unwrap().unwrap().deleted());
    assert!(block_on(store.retain(job)).is_err());
}

#[test]
fn autonomous_background_exit_does_not_make_an_exact_original_run_stop_stale() {
    use nocterm_cloud_model::{
        coordinator::{CommandBody, ReceivedCommand},
        grant::GrantWindow,
    };
    let root = private_root();
    let store = journal(root.path(), Arc::new(TestCipher::default()))
        .with_background_events(Arc::new(TestEvents::default()), publication_budgets())
        .unwrap();
    let lease = claim(&store, None);
    let mut state = state(1, &lease);
    let mut position = JournalPosition::default();
    let job = dispatch(&store, &lease, &mut state, &mut position, 1);
    block_on(store.retain(job.clone())).unwrap();
    let before = current(&store);
    let original = CommandAcceptance::new(
        CommandId::from_bytes([240; 16]),
        CommandKind::Stop,
        fp(240),
        CommandIssuer::new(NamespaceId::new("device").unwrap(), fp(50), fp(51)),
        before.control(),
    );
    let stop = ReceivedCommand::new(
        stream(),
        original.clone(),
        GrantWindow::new(Timestamp::from_millis(1), Timestamp::from_millis(100)).unwrap(),
        Some(state.identity().clone()),
        CommandBody::Stop,
        None,
    )
    .unwrap();
    block_on(store.finish(job, BackgroundCommandOutcome::ObservedExit(0))).unwrap();
    let after = current(&store);
    assert_eq!(before.control(), after.control());
    assert!(after.records() > before.records());
    assert!(after.events() > before.events());
    let record = JournalRecord::new(
        RecordContext::Run(stop.run().unwrap().clone()),
        JournalRecordKind::CommandAccepted {
            id: original.id(),
            kind: CommandKind::Stop,
        },
        bytes(b"original exact-run stop M semantic frame"),
    )
    .unwrap();
    let event = fixture_event(
        &lease,
        after.next(1, 1, true).unwrap(),
        EventId::from_bytes([241; 16]),
        record.context().run().cloned(),
        bytes(b"sealed stop receipt"),
    );
    let batch = JournalBatch::new(
        lease,
        after,
        Some(original),
        vec![record],
        None,
        vec![event],
    )
    .unwrap();
    let result = block_on(store.commit(batch)).unwrap();
    assert!(matches!(result, CommitOutcome::Committed(_)));
}
