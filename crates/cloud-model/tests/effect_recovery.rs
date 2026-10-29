mod support;
use nocterm_ai::{ExecCommand, TerminalCall};
use nocterm_cloud_model::{
    authority::{AuthorityError, RunBinding},
    identity::*,
    run::*,
};
use support::*;

fn write() -> TerminalCall {
    TerminalCall::ExecCommand(ExecCommand {
        terminal_id: "t".into(),
        program: "printf".into(),
        args: vec!["literal argument".into()],
        stdin: None,
        timeout_ms: None,
        yield_ms: None,
    })
}
fn state() -> RunState {
    RunState::new(
        run(1, 1, 1, 1),
        target(1),
        WriterEpoch::new(1).unwrap(),
        Revision::default(),
    )
}
fn stamp(state: &RunState) -> CommitStamp {
    CommitStamp::new(
        state.identity().clone(),
        state.writer(),
        state.revision().next().unwrap(),
    )
}
fn intent(binding: &RunBinding, id: u8, call: &TerminalCall) -> ToolIntent {
    ToolIntent::new(
        IntentId::from_bytes([id; 16]),
        StepId::from_bytes([id; 16]),
        authority(binding.clone()),
        fingerprint(id),
        call,
    )
    .unwrap()
}
fn dispatch(state: &mut RunState, id: u8, call: &TerminalCall) -> DispatchPermit {
    let request = intent(state.identity(), id, call);
    let id = request.id();
    state.prepare_intent(request, state.revision()).unwrap();
    state.intent_committed(id, stamp(state)).unwrap();
    state.dispatch_committed(id, stamp(state)).unwrap()
}
fn unknown(state: &mut RunState, id: u8) -> UnknownEffect {
    dispatch(state, id, &write());
    state
        .outcome_committed(
            IntentId::from_bytes([id; 16]),
            stamp(state),
            EffectOutcome::Unknown,
        )
        .unwrap();
    state.unknown().unwrap().clone()
}

#[test]
fn dispatch_is_released_only_after_both_matching_durable_barriers() {
    let mut state = state();
    let request = intent(state.identity(), 1, &write());
    state
        .prepare_intent(request.clone(), state.revision())
        .unwrap();
    assert!(matches!(
        state.dispatch_committed(request.id(), stamp(&state)),
        Err(RunError::WrongEffectStage)
    ));
    assert_eq!(state.revision(), Revision::default());
    state.intent_committed(request.id(), stamp(&state)).unwrap();
    assert!(matches!(
        state.outcome_committed(
            request.id(),
            stamp(&state),
            EffectOutcome::Known(fingerprint(1))
        ),
        Err(RunError::WrongEffectStage)
    ));
    let permit = state
        .dispatch_committed(request.id(), stamp(&state))
        .unwrap();
    assert_eq!(permit.intent(), &request);
    assert_eq!(permit.commit().revision(), state.revision());
    state
        .outcome_committed(
            request.id(),
            stamp(&state),
            EffectOutcome::Known(fingerprint(1)),
        )
        .unwrap();
    assert_eq!(state.pending_stage(), None);
}

#[test]
fn a_durable_pre_dispatch_decline_neither_dispatches_nor_creates_uncertainty() {
    let mut state = state();
    let request = intent(state.identity(), 1, &write());
    state
        .prepare_intent(request.clone(), state.revision())
        .unwrap();
    assert!(matches!(
        state.decline_committed(request.id(), stamp(&state)),
        Err(RunError::WrongEffectStage)
    ));
    state.intent_committed(request.id(), stamp(&state)).unwrap();
    state
        .decline_committed(request.id(), stamp(&state))
        .unwrap();
    assert_eq!(state.pending_stage(), None);
    assert_eq!(state.unknown(), None);
    assert!(matches!(
        state.dispatch_committed(request.id(), stamp(&state)),
        Err(RunError::StaleIntent)
    ));
    assert!(
        state
            .prepare_intent(intent(state.identity(), 2, &write()), state.revision())
            .is_ok()
    );
}

#[test]
fn declining_a_read_does_not_clear_another_unknown_mutation() {
    let mut state = state();
    let original = unknown(&mut state, 1);
    let read = intent(state.identity(), 2, &TerminalCall::ListTerminals);
    state
        .prepare_intent(read.clone(), state.revision())
        .unwrap();
    state.intent_committed(read.id(), stamp(&state)).unwrap();
    state.decline_committed(read.id(), stamp(&state)).unwrap();
    assert_eq!(state.unknown(), Some(&original));
    assert_eq!(state.phase(), RunPhase::OutcomeUnknown);
}

#[test]
fn unknown_mutation_fences_new_call_ids_but_reads_cannot_clear_the_fence() {
    let mut state = state();
    let original = unknown(&mut state, 1);
    let mutation = intent(state.identity(), 2, &write());
    assert!(matches!(
        state.prepare_intent(mutation, state.revision()),
        Err(RunError::ReconciliationRequired)
    ));
    let read = dispatch(&mut state, 3, &TerminalCall::ListTerminals);
    state
        .outcome_committed(
            read.intent().id(),
            stamp(&state),
            EffectOutcome::Known(fingerprint(3)),
        )
        .unwrap();
    assert_eq!(state.unknown(), Some(&original));
    assert_eq!(state.phase(), RunPhase::OutcomeUnknown);
    assert!(matches!(
        state.complete(stamp(&state)),
        Err(RunError::ReconciliationRequired)
    ));
}

#[test]
fn only_explicit_reconciliation_of_the_original_intent_can_release_fresh_writes() {
    let mut current = state();
    let original = unknown(&mut current, 1);
    let wrong = unknown(&mut state(), 9);
    let receipt = |original, commit| ReconciliationReceipt {
        command: CommandId::from_bytes([7; 16]),
        commit,
        original,
        decision: ReconciliationDecision::Observed(fingerprint(8)),
    };
    assert!(matches!(
        current.reconcile(receipt(wrong, stamp(&current))),
        Err(RunError::StaleReconciliation)
    ));
    assert_eq!(current.unknown(), Some(&original));
    current
        .reconcile(receipt(original, stamp(&current)))
        .unwrap();
    assert_eq!(current.phase(), RunPhase::Running);
    assert_eq!(current.unknown(), None);
    let fresh = intent(current.identity(), 2, &write());
    assert!(current.prepare_intent(fresh, current.revision()).is_ok());
}

#[test]
fn interruption_and_new_epoch_preserve_uncertainty_and_refuse_old_completions() {
    let mut state = state();
    let pending = dispatch(&mut state, 1, &write());
    let stale = stamp(&state);
    state.interrupt(stamp(&state)).unwrap();
    let original = state.unknown().unwrap().clone();
    let resumed = CommitStamp::new(
        state.identity().with_epoch(RunEpoch::new(2).unwrap()),
        WriterEpoch::new(2).unwrap(),
        state.revision().next().unwrap(),
    );
    state.resume(resumed).unwrap();
    assert_eq!(state.phase(), RunPhase::OutcomeUnknown);
    assert_eq!(state.unknown(), Some(&original));
    assert!(matches!(
        state.outcome_committed(
            pending.intent().id(),
            stale,
            EffectOutcome::Known(fingerprint(1))
        ),
        Err(RunError::Fenced)
    ));
    let fresh = intent(state.identity(), 9, &write());
    assert!(matches!(
        state.prepare_intent(fresh, state.revision()),
        Err(RunError::ReconciliationRequired)
    ));
}

#[test]
fn recovery_classifies_possible_dispatch_without_ever_reissuing_an_effect() {
    for stage in [
        EffectStage::Prepared,
        EffectStage::IntentCommitted,
        EffectStage::DispatchCommitted,
    ] {
        let binding = run(1, 1, 1, 1);
        let effect = RecoveredEffect {
            intent: intent(&binding, 1, &write()),
            stage,
        };
        let recovered = RunState::recover(
            binding,
            target(1),
            WriterEpoch::new(1).unwrap(),
            Revision::from_u64(5),
            RecoveredRunEffects {
                pending: Some(effect),
                unresolved: None,
            },
        )
        .unwrap();
        assert_eq!(recovered.phase(), RunPhase::Interrupted);
        assert_eq!(recovered.pending_stage(), None);
        assert_eq!(
            recovered.unknown().is_some(),
            stage == EffectStage::DispatchCommitted
        );
    }
}

#[test]
fn scope_revision_writer_and_intent_substitution_refuse_without_advancing_state() {
    let mut state = state();
    let foreign = intent(&run(2, 1, 1, 1), 1, &write());
    assert!(matches!(
        state.prepare_intent(foreign, state.revision()),
        Err(RunError::Authority(AuthorityError::WrongScope))
    ));
    let original = intent(state.identity(), 1, &write());
    assert!(matches!(
        state.prepare_intent(original.clone(), Revision::from_u64(10)),
        Err(RunError::Conflict)
    ));
    state
        .prepare_intent(original.clone(), state.revision())
        .unwrap();
    let fenced = CommitStamp::new(
        state.identity().clone(),
        WriterEpoch::new(2).unwrap(),
        Revision::from_u64(1),
    );
    assert!(matches!(
        state.intent_committed(original.id(), fenced),
        Err(RunError::Fenced)
    ));
    assert!(matches!(
        state.intent_committed(IntentId::from_bytes([2; 16]), stamp(&state)),
        Err(RunError::StaleIntent)
    ));
    assert_eq!(state.revision(), Revision::default());
    assert_eq!(state.pending_stage(), Some(EffectStage::Prepared));
}

#[test]
fn completion_is_terminal_and_exhausted_revisions_never_release_dispatch() {
    let mut completed = state();
    completed.complete(stamp(&completed)).unwrap();
    assert_eq!(completed.phase(), RunPhase::Completed);
    assert!(matches!(
        completed.prepare_intent(
            intent(completed.identity(), 1, &write()),
            completed.revision()
        ),
        Err(RunError::WrongPhase)
    ));
    let mut full = RunState::new(
        run(1, 1, 1, 1),
        target(1),
        WriterEpoch::new(1).unwrap(),
        Revision::from_u64(u64::MAX),
    );
    assert!(matches!(
        full.prepare_intent(intent(full.identity(), 1, &write()), full.revision()),
        Err(RunError::InvalidIdentity(IdentityError::Exhausted))
    ));
    assert_eq!(full.pending_stage(), None);
}

#[test]
fn replay_keeps_original_unknown_after_resume_and_a_later_read() {
    let mut previous = state();
    let unknown = unknown(&mut previous, 1);
    previous.interrupt(stamp(&previous)).unwrap();
    let resumed = CommitStamp::new(
        previous.identity().with_epoch(RunEpoch::new(2).unwrap()),
        WriterEpoch::new(2).unwrap(),
        previous.revision().next().unwrap(),
    );
    previous.resume(resumed).unwrap();
    let read = intent(previous.identity(), 2, &TerminalCall::ListTerminals);
    let mut recovered = RunState::recover(
        previous.identity().clone(),
        target(1),
        previous.writer(),
        previous.revision(),
        RecoveredRunEffects {
            pending: Some(RecoveredEffect {
                intent: read,
                stage: EffectStage::DispatchCommitted,
            }),
            unresolved: Some(unknown.intent().clone()),
        },
    )
    .unwrap();
    assert_eq!(recovered.unknown(), Some(&unknown));
    let resumed = CommitStamp::new(
        recovered.identity().with_epoch(RunEpoch::new(3).unwrap()),
        WriterEpoch::new(3).unwrap(),
        recovered.revision().next().unwrap(),
    );
    recovered.resume(resumed).unwrap();
    assert!(matches!(
        recovered.prepare_intent(
            intent(recovered.identity(), 3, &write()),
            recovered.revision()
        ),
        Err(RunError::ReconciliationRequired)
    ));
    recovered
        .reconcile(ReconciliationReceipt {
            command: CommandId::from_bytes([9; 16]),
            commit: stamp(&recovered),
            original: unknown,
            decision: ReconciliationDecision::Observed(fingerprint(9)),
        })
        .unwrap();
    assert!(
        recovered
            .prepare_intent(
                intent(recovered.identity(), 3, &write()),
                recovered.revision()
            )
            .is_ok()
    );
}

#[test]
fn replay_refuses_foreign_future_or_overlapping_unresolved_mutations() {
    let binding = run(1, 1, 1, 2);
    for original in [run(2, 1, 1, 1), run(1, 1, 1, 3), run(1, 1, 9, 1)] {
        assert!(
            RunState::recover(
                binding.clone(),
                target(1),
                WriterEpoch::new(1).unwrap(),
                Revision::default(),
                RecoveredRunEffects {
                    pending: None,
                    unresolved: Some(intent(&original, 1, &write()))
                }
            )
            .is_err()
        );
    }
    let invalid = RecoveredRunEffects {
        pending: Some(RecoveredEffect {
            intent: intent(&binding, 2, &write()),
            stage: EffectStage::DispatchCommitted,
        }),
        unresolved: Some(intent(
            &binding.with_epoch(RunEpoch::new(1).unwrap()),
            1,
            &write(),
        )),
    };
    assert!(matches!(
        RunState::recover(
            binding,
            target(1),
            WriterEpoch::new(1).unwrap(),
            Revision::default(),
            invalid
        ),
        Err(RunError::InvalidRecovery)
    ));
}
