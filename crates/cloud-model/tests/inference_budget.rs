mod support;
use nocterm_cloud_model::{
    engine::{ModelBudget, ModelCallObservation, ModelCallRequest, UsageObservation},
    grant::GrantWindow,
    identity::*,
    inference::*,
};
use support::*;

fn selection() -> ModelSelection {
    ModelSelection::new(
        NamespaceId::new("provider").unwrap(),
        NamespaceId::new("model").unwrap(),
        NamespaceId::new("bounded-policy").unwrap(),
    )
}
fn task() -> InferenceTask {
    InferenceTask::new(
        run(1, 1, 1, 1),
        selection(),
        ModelBudget::new(4, 4096, 1024, 100).unwrap(),
        100,
        GrantWindow::new(Timestamp::from_millis(100), Timestamp::from_millis(1000)).unwrap(),
    )
    .unwrap()
}
fn grant(id: u8) -> InferenceGrant {
    InferenceGrant::new(
        GrantId::from_bytes([id; 16]),
        GrantEpoch::new(1).unwrap(),
        origin(),
        task(),
    )
}
fn request(step: u8, tokens: u64) -> ModelCallRequest {
    ModelCallRequest::new(
        task().run().clone(),
        StepId::from_bytes([step; 16]),
        100,
        tokens,
    )
    .unwrap()
}
fn quote() -> ModelQuote {
    ConservativeModelPricing::new(selection(), 4096, 100, 40)
        .unwrap()
        .quote(&request(1, 50))
        .unwrap()
}
fn observation(tokens: Option<u64>, cost: Option<u64>, bytes: usize) -> ModelCallObservation {
    ModelCallObservation::new(
        UsageObservation {
            output_tokens: tokens,
            cost_micros: cost,
            ..Default::default()
        },
        bytes,
    )
    .unwrap()
}
fn reserve(ledger: &mut InferenceLedger, id: u8, tokens: u64) -> InferenceReservation {
    ledger
        .reserve(
            grant(1),
            AttemptId::from_bytes([id; 16]),
            request(id, tokens),
            quote(),
            Timestamp::from_millis(200 + u64::from(id)),
        )
        .unwrap()
}

#[test]
fn verified_observation_does_not_refund_until_model_action_settlement() {
    let mut ledger = InferenceLedger::new(task());
    let call = reserve(&mut ledger, 1, 50);
    let before = ledger.charge().unwrap();
    ledger
        .observe(
            call.attempt(),
            call.request(),
            observation(Some(20), Some(10), 5),
        )
        .unwrap();
    assert_eq!(ledger.charge().unwrap(), before);
    ledger.finalize(call.attempt()).unwrap();
    let after = ledger.charge().unwrap();
    assert_eq!(
        (
            after.calls(),
            after.input_bytes(),
            after.output_tokens(),
            after.output_bytes(),
            after.spend_micros()
        ),
        (1, 100, 20, 5, 10)
    );
    ledger.finalize(call.attempt()).unwrap();
    assert_eq!(ledger.charge().unwrap(), after);
}

#[test]
fn unknown_usage_and_money_retain_allowance_and_stop_before_another_call() {
    let mut ledger = InferenceLedger::new(task());
    let first = reserve(&mut ledger, 1, 50);
    ledger
        .observe(
            first.attempt(),
            first.request(),
            observation(Some(20), None, 5),
        )
        .unwrap();
    ledger.finalize(first.attempt()).unwrap();
    let second = reserve(&mut ledger, 2, 50);
    ledger
        .observe(
            second.attempt(),
            second.request(),
            observation(None, None, 2),
        )
        .unwrap();
    ledger.finalize(second.attempt()).unwrap();
    let charge = ledger.charge().unwrap();
    assert_eq!(
        (
            charge.output_tokens(),
            charge.spend_micros(),
            charge.output_bytes()
        ),
        (70, 80, 7)
    );
    assert!(matches!(
        ledger.reserve(
            grant(1),
            AttemptId::from_bytes([3; 16]),
            request(3, 30),
            quote(),
            Timestamp::from_millis(203)
        ),
        Err(InferenceError::BudgetExceeded)
    ));
    assert_eq!(ledger.charge().unwrap(), charge);
}

#[test]
fn recovering_unknown_reservations_never_reissues_the_original_or_renamed_step() {
    let mut ledger = InferenceLedger::new(task());
    let first = reserve(&mut ledger, 1, 50);
    let charge = ledger.charge().unwrap();
    let mut recovered = InferenceLedger::from_snapshot(ledger.snapshot());
    for attempt in [first.attempt(), AttemptId::from_bytes([99; 16])] {
        assert!(matches!(
            recovered.reserve(
                grant(2),
                attempt,
                first.request().clone(),
                quote(),
                Timestamp::from_millis(300)
            ),
            Err(InferenceError::Conflict)
        ));
    }
    assert_eq!(recovered.charge().unwrap(), charge);
    assert!(matches!(
        recovered.reserve(
            grant(2),
            AttemptId::from_bytes([3; 16]),
            request(3, 30),
            quote(),
            Timestamp::from_millis(301)
        ),
        Err(InferenceError::BudgetExceeded)
    ));
}

#[test]
fn grant_replacement_does_not_reset_cumulative_spending() {
    let mut ledger = InferenceLedger::new(task());
    let first = reserve(&mut ledger, 1, 50);
    ledger
        .observe(
            first.attempt(),
            first.request(),
            observation(Some(0), None, 0),
        )
        .unwrap();
    ledger.finalize(first.attempt()).unwrap();
    let second = ledger
        .reserve(
            grant(2),
            AttemptId::from_bytes([2; 16]),
            request(2, 50),
            quote(),
            Timestamp::from_millis(202),
        )
        .unwrap();
    ledger
        .observe(
            second.attempt(),
            second.request(),
            observation(Some(0), None, 0),
        )
        .unwrap();
    ledger.finalize(second.attempt()).unwrap();
    assert_eq!(ledger.charge().unwrap().spend_micros(), 80);
    assert!(matches!(
        ledger.reserve(
            grant(3),
            AttemptId::from_bytes([3; 16]),
            request(3, 50),
            quote(),
            Timestamp::from_millis(203)
        ),
        Err(InferenceError::BudgetExceeded)
    ));
}

#[test]
fn observations_are_exactly_idempotent_and_malformed_usage_never_refunds() {
    let mut ledger = InferenceLedger::new(task());
    let call = reserve(&mut ledger, 1, 50);
    let charge = ledger.charge().unwrap();
    for bad in [
        observation(Some(51), None, 1),
        observation(Some(1), Some(41), 1),
        observation(Some(1), None, 1025),
    ] {
        assert!(matches!(
            ledger.observe(call.attempt(), call.request(), bad),
            Err(InferenceError::InvalidObservation)
        ));
    }
    assert!(ledger.snapshot().calls()[0].observation().is_none());
    let known = observation(Some(1), None, 3);
    ledger
        .observe(call.attempt(), call.request(), known.clone())
        .unwrap();
    ledger
        .observe(call.attempt(), call.request(), known)
        .unwrap();
    assert!(matches!(
        ledger.observe(
            call.attempt(),
            call.request(),
            observation(Some(1), None, 4)
        ),
        Err(InferenceError::Conflict)
    ));
    assert_eq!(ledger.charge().unwrap(), charge);
}

#[test]
fn expiry_and_clock_rollback_fence_the_same_task_across_grants_and_restore() {
    let mut expired = InferenceLedger::new(task());
    assert!(matches!(
        expired.reserve(
            grant(1),
            AttemptId::from_bytes([1; 16]),
            request(1, 50),
            quote(),
            Timestamp::from_millis(1000)
        ),
        Err(InferenceError::Expired)
    ));
    assert_eq!(expired.charge().unwrap().calls(), 0);
    let mut ledger = InferenceLedger::new(task());
    reserve(&mut ledger, 1, 50);
    assert!(matches!(
        ledger.check(&grant(1), Timestamp::from_millis(200)),
        Err(InferenceError::ClockRollback)
    ));
    let mut recovered = InferenceLedger::from_snapshot(ledger.snapshot());
    assert!(matches!(
        recovered.check(&grant(2), Timestamp::from_millis(500)),
        Err(InferenceError::ClockRollback)
    ));
}

#[test]
fn recovery_rejects_duplicate_attempts_steps_and_future_admission_times() {
    let mut ledger = InferenceLedger::new(task());
    reserve(&mut ledger, 1, 50);
    let saved = ledger.snapshot();
    let call = saved.calls()[0].clone();
    assert!(matches!(
        InferenceSnapshot::new(
            task(),
            vec![call.clone(), call.clone()],
            saved.observed(),
            false
        ),
        Err(InferenceError::InvalidSnapshot)
    ));
    assert!(matches!(
        InferenceSnapshot::new(task(), vec![call], Timestamp::from_millis(200), false),
        Err(InferenceError::InvalidSnapshot)
    ));
}

#[test]
fn configured_pricing_and_exact_model_binding_are_required_before_reservation() {
    let pricing = ConservativeModelPricing::new(selection(), 100, 50, 40).unwrap();
    assert!(pricing.quote(&request(1, 50)).is_ok());
    assert!(matches!(
        pricing.quote(&request(1, 51)),
        Err(InferenceError::PricingLimit)
    ));
    let larger =
        ModelCallRequest::new(task().run().clone(), StepId::from_bytes([1; 16]), 101, 50).unwrap();
    assert!(matches!(
        pricing.quote(&larger),
        Err(InferenceError::PricingLimit)
    ));
    let other = ModelSelection::new(
        NamespaceId::new("other-provider").unwrap(),
        selection().model().clone(),
        selection().pricing().clone(),
    );
    let mut ledger = InferenceLedger::new(task());
    assert!(matches!(
        ledger.reserve(
            grant(1),
            AttemptId::from_bytes([1; 16]),
            request(1, 50),
            ModelQuote::new(other, 40).unwrap(),
            Timestamp::from_millis(200)
        ),
        Err(InferenceError::Binding)
    ));
    assert_eq!(ledger.charge().unwrap().calls(), 0);
}

fn bounded_task(turns: u32, calls: u32) -> InferenceTask {
    InferenceTask::new(
        task().run().clone(),
        selection(),
        ModelBudget::new(turns, 4096, 1024, 100)
            .unwrap()
            .with_calls(calls)
            .unwrap(),
        100,
        task().window(),
    )
    .unwrap()
}
fn bounded_grant(task: InferenceTask) -> InferenceGrant {
    InferenceGrant::new(
        GrantId::from_bytes([1; 16]),
        GrantEpoch::new(1).unwrap(),
        origin(),
        task,
    )
}
#[test]
fn call_and_turn_limits_independently_refuse_the_next_actual_reservation() {
    for (turns, calls) in [(4, 1), (1, 4)] {
        let task = bounded_task(turns, calls);
        let grant = bounded_grant(task.clone());
        let mut ledger = InferenceLedger::new(task);
        let first = ledger
            .reserve(
                grant.clone(),
                AttemptId::from_bytes([1; 16]),
                request(1, 10),
                quote(),
                Timestamp::from_millis(201),
            )
            .unwrap();
        ledger
            .observe(
                first.attempt(),
                first.request(),
                observation(Some(0), Some(0), 0),
            )
            .unwrap();
        ledger.finalize(first.attempt()).unwrap();
        assert_eq!(
            (
                ledger.charge().unwrap().calls(),
                ledger.charge().unwrap().turns()
            ),
            (1, 1)
        );
        assert!(matches!(
            ledger.reserve(
                grant,
                AttemptId::from_bytes([2; 16]),
                request(2, 10),
                quote(),
                Timestamp::from_millis(202)
            ),
            Err(InferenceError::BudgetExceeded)
        ));
        assert_eq!(ledger.snapshot().calls().len(), 1);
    }
}
#[test]
fn snapshots_and_grant_containment_enforce_both_independent_allowances() {
    for (turns, calls) in [(4, 1), (1, 4)] {
        let task = bounded_task(turns, calls);
        let grant = bounded_grant(task.clone());
        let snapshots = (1..=2)
            .map(|id| {
                InferenceCallSnapshot::new(
                    InferenceReservation::from_journal(
                        AttemptId::from_bytes([id; 16]),
                        grant.clone(),
                        request(id, 10),
                        quote(),
                        1,
                        Timestamp::from_millis(200 + u64::from(id)),
                    )
                    .unwrap(),
                    None,
                    false,
                )
                .unwrap()
            })
            .collect();
        assert!(matches!(
            InferenceSnapshot::new(task.clone(), snapshots, Timestamp::from_millis(202), false),
            Err(InferenceError::InvalidSnapshot)
        ));
        let enlarged = bounded_task(
            if turns == 1 { 2 } else { turns },
            if calls == 1 { 2 } else { calls },
        );
        assert!(!task.contains(&enlarged));
    }
}
#[test]
fn legacy_constructor_couples_allowances_and_explicit_calls_are_finite() {
    let budget = ModelBudget::new(3, 4096, 1024, 100).unwrap();
    assert_eq!((budget.calls(), budget.turns()), (3, 3));
    assert!(budget.with_calls(0).is_err());
    assert!(budget.with_calls(1025).is_err());
    let explicit = budget.with_calls(1).unwrap();
    assert_eq!((explicit.calls(), explicit.turns()), (1, 3));
}

#[test]
fn reported_usage_sums_original_observations_and_propagates_each_unknown_field() {
    let full = UsageObservation {
        input_tokens: Some(11),
        output_tokens: Some(7),
        total_tokens: Some(18),
        cached_input_tokens: Some(3),
        thought_tokens: Some(2),
        cost_micros: Some(5),
    };
    let zero = UsageObservation {
        input_tokens: Some(0),
        output_tokens: Some(0),
        total_tokens: Some(0),
        cached_input_tokens: Some(0),
        thought_tokens: Some(0),
        cost_micros: Some(0),
    };
    for second in [
        full,
        UsageObservation::default(),
        zero,
        UsageObservation {
            output_tokens: Some(3),
            cost_micros: Some(7),
            ..Default::default()
        },
    ] {
        let mut ledger = InferenceLedger::new(task());
        assert_eq!(
            ledger.reported_usage().unwrap(),
            UsageObservation::default()
        );
        let first = reserve(&mut ledger, 1, 50);
        ledger
            .observe(
                first.attempt(),
                first.request(),
                ModelCallObservation::new(full, 1).unwrap(),
            )
            .unwrap();
        // Provider reports are visible independently of conservative settlement.
        assert_eq!(ledger.reported_usage().unwrap(), full);
        assert_eq!(ledger.charge().unwrap().spend_micros(), 40);
        ledger.finalize(first.attempt()).unwrap();
        let call = reserve(&mut ledger, 2, 50);
        assert_eq!(
            ledger.reported_usage().unwrap(),
            UsageObservation::default()
        );
        ledger
            .observe(
                call.attempt(),
                call.request(),
                ModelCallObservation::new(second, 1).unwrap(),
            )
            .unwrap();
        let expected = UsageObservation {
            input_tokens: second.input_tokens.map(|v| v + 11),
            output_tokens: second.output_tokens.map(|v| v + 7),
            total_tokens: second.total_tokens.map(|v| v + 18),
            cached_input_tokens: second.cached_input_tokens.map(|v| v + 3),
            thought_tokens: second.thought_tokens.map(|v| v + 2),
            cost_micros: second.cost_micros.map(|v| v + 5),
        };
        assert_eq!(ledger.reported_usage().unwrap(), expected);
        assert_eq!(
            InferenceLedger::from_snapshot(ledger.snapshot())
                .reported_usage()
                .unwrap(),
            expected
        );
    }
    let mut ledger = InferenceLedger::new(task());
    let first = reserve(&mut ledger, 1, 50);
    ledger
        .observe(
            first.attempt(),
            first.request(),
            ModelCallObservation::new(zero, 0).unwrap(),
        )
        .unwrap();
    assert_eq!(ledger.reported_usage().unwrap(), zero);
}

#[test]
fn reported_usage_overflow_is_typed_and_never_changes_conservative_charge() {
    let mut ledger = InferenceLedger::new(task());
    for (id, input) in [(1, u64::MAX), (2, 1)] {
        let call = reserve(&mut ledger, id, 50);
        ledger
            .observe(
                call.attempt(),
                call.request(),
                ModelCallObservation::new(
                    UsageObservation {
                        input_tokens: Some(input),
                        output_tokens: Some(0),
                        cost_micros: Some(0),
                        ..Default::default()
                    },
                    0,
                )
                .unwrap(),
            )
            .unwrap();
        ledger.finalize(call.attempt()).unwrap();
    }
    let charge = ledger.charge().unwrap();
    assert!(matches!(
        ledger.reported_usage(),
        Err(InferenceError::UsageOverflow)
    ));
    assert_eq!(ledger.charge().unwrap(), charge);
}
