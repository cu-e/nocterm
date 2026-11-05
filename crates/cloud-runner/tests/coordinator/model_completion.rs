//! Local SDK completion does not create a fictitious model attempt or refund.
use super::*;
use nocterm_ai::TerminalCall;
use nocterm_cloud_model::payload::BoundedJson;

pub(super) fn listing_turn() -> ScriptedEngineTurn {
    let step = StepId::from_bytes([0x71; 16]);
    let mut turn = complete();
    turn.action = EngineAction::Tools(
        EngineToolBatch::new(
            step,
            vec![
                EngineToolCall::new(
                    ToolSlot::new(step, 0).unwrap(),
                    Some(NamespaceId::new("listing-call").unwrap()),
                    TerminalCall::ListTerminals,
                )
                .unwrap(),
            ],
        )
        .unwrap(),
    );
    turn
}

#[tokio::test]
async fn exhausted_sdk_turns_complete_without_a_new_reservation_or_model_action() {
    let fixture = Fixture::new();
    for _ in 0..2 {
        fixture.engine.push(listing_turn());
        fixture.tools.push(Ok(
            BoundedJson::new(serde_json::json!({"terminals":[]})).unwrap()
        ));
    }
    fixture.engine.push_local_completion(EngineCompletion {
        stop: StopReason::MaxTurnRequests,
        usage: UsageObservation::default(),
    });
    let run = reconciliation::admitted_with_operations(&fixture, 2).await;
    assert!(fixture.actor.drive_one().await.unwrap());
    assert_eq!(
        fixture.actor.status(&run).await.unwrap(),
        RunStatus::Completed
    );
    assert_eq!(fixture.engine.calls(), 2);
    assert_eq!(fixture.engine.inputs().len(), 3);
    assert_eq!(fixture.tools.requests().len(), 2);
    let page = fixture
        .journal
        .read(stream(), JournalSequence::default(), limits())
        .await
        .unwrap();
    assert_eq!(
        page.records()
            .iter()
            .filter(|stored| matches!(
                stored.record().kind(),
                JournalRecordKind::ModelAction { .. }
            ))
            .count(),
        2
    );
    // Each actual model call has exactly its reservation and observation.
    assert_eq!(
        page.records()
            .iter()
            .filter(|stored| matches!(
                stored.record().kind(),
                JournalRecordKind::BudgetReserved { .. }
            ))
            .count(),
        4
    );
}

#[tokio::test]
async fn unreserved_completion_cannot_invent_an_exhausted_budget_or_provider_turn() {
    for stop in [StopReason::MaxTurnRequests, StopReason::EndTurn] {
        let fixture = Fixture::new();
        fixture.engine.push_local_completion(EngineCompletion {
            stop,
            usage: UsageObservation::default(),
        });
        let run = reconciliation::admitted(&fixture).await;
        assert!(fixture.actor.drive_one().await.is_err());
        assert_eq!(
            fixture.actor.status(&run).await.unwrap(),
            RunStatus::Interrupted
        );
        assert_eq!(fixture.engine.calls(), 0);
        assert!(fixture.tools.requests().is_empty());
        let page = fixture
            .journal
            .read(stream(), JournalSequence::default(), limits())
            .await
            .unwrap();
        assert!(!page.records().iter().any(|stored| matches!(
            stored.record().kind(),
            JournalRecordKind::ModelAction { .. } | JournalRecordKind::BudgetReserved { .. }
        )));
    }
}

#[tokio::test]
async fn reported_usage_progress_is_cumulative_across_two_actual_calls() {
    let first = UsageObservation {
        input_tokens: Some(11),
        output_tokens: Some(7),
        total_tokens: Some(18),
        cached_input_tokens: Some(3),
        thought_tokens: Some(2),
        cost_micros: Some(5),
    };
    for second in [
        UsageObservation {
            input_tokens: Some(4),
            output_tokens: Some(3),
            total_tokens: Some(7),
            cached_input_tokens: Some(0),
            thought_tokens: Some(0),
            cost_micros: Some(7),
        },
        UsageObservation::default(),
        UsageObservation {
            output_tokens: Some(3),
            cost_micros: Some(7),
            ..Default::default()
        },
    ] {
        let fixture = Fixture::new();
        let mut listing = listing_turn();
        listing.observation = ModelCallObservation::new(first, 1).unwrap();
        listing.events.push(EngineEvent::Usage(first));
        fixture.engine.push(listing);
        fixture.tools.push(Ok(
            BoundedJson::new(serde_json::json!({"terminals":[]})).unwrap()
        ));
        let mut completion = complete();
        completion.observation = ModelCallObservation::new(second, 1).unwrap();
        completion.events.push(EngineEvent::Usage(second));
        fixture.engine.push(completion);
        let run = reconciliation::admitted_with_operations(&fixture, 1).await;
        fixture.actor.drive_one().await.unwrap();
        assert_eq!(fixture.engine.calls(), 2);
        let published = fixture
            .events
            .events()
            .into_iter()
            .filter_map(|event| match event {
                CoordinatorEvent::Progress {
                    run: actual,
                    event: EngineEvent::Usage(usage),
                } => {
                    assert_eq!(actual, run);
                    Some(usage)
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        let cumulative = UsageObservation {
            input_tokens: second.input_tokens.map(|v| v + 11),
            output_tokens: second.output_tokens.map(|v| v + 7),
            total_tokens: second.total_tokens.map(|v| v + 18),
            cached_input_tokens: second.cached_input_tokens.map(|v| v + 3),
            thought_tokens: second.thought_tokens.map(|v| v + 2),
            cost_micros: second.cost_micros.map(|v| v + 5),
        };
        assert_eq!(
            published,
            vec![
                UsageObservation::default(),
                first,
                UsageObservation::default(),
                cumulative
            ]
        );
    }
}

fn reported_usage(fixture: &Fixture, run: &RunBinding) -> Vec<UsageObservation> {
    fixture
        .events
        .events()
        .into_iter()
        .filter_map(|event| match event {
            CoordinatorEvent::Progress {
                run: actual,
                event: EngineEvent::Usage(usage),
            } => {
                assert_eq!(&actual, run);
                Some(usage)
            }
            _ => None,
        })
        .collect::<Vec<_>>()
}
fn script_usage_turns(fixture: &Fixture, first: UsageObservation, second: UsageObservation) {
    let mut listing = listing_turn();
    listing.observation = ModelCallObservation::new(first, 1).unwrap();
    listing.events.push(EngineEvent::Usage(first));
    fixture.engine.push(listing);
    fixture.tools.push(Ok(
        BoundedJson::new(serde_json::json!({"terminals":[]})).unwrap()
    ));
    let mut completion = complete();
    completion.observation = ModelCallObservation::new(second, 1).unwrap();
    completion.events.push(EngineEvent::Usage(second));
    fixture.engine.push(completion);
}
/// Counts budget reservations and reports whether any of them changed control.
async fn budget_reservations(fixture: &Fixture) -> (usize, bool) {
    let page = fixture
        .journal
        .read(stream(), JournalSequence::default(), limits())
        .await
        .unwrap();
    let budgets = page
        .records()
        .iter()
        .filter(|stored| {
            matches!(
                stored.record().kind(),
                JournalRecordKind::BudgetReserved { .. }
            )
        })
        .collect::<Vec<_>>();
    (
        budgets.len(),
        budgets
            .iter()
            .any(|stored| stored.record().changes_control()),
    )
}
async fn assert_final_status(fixture: &Fixture, run: &RunBinding, boundary: usize) {
    assert_eq!(
        fixture.actor.status(run).await.unwrap(),
        if boundary == 0 {
            RunStatus::Interrupted
        } else {
            RunStatus::Stopped
        }
    );
}
async fn stop_after_boundary(
    fixture: &Fixture,
    boundary: usize,
    control_before_release: ChatControlRevision,
    run: &RunBinding,
) -> Option<Result<CommandReceipt, CoordinatorError>> {
    if boundary != 0 {
        Some(
            fixture
                .actor
                .accept(command(
                    4,
                    control_before_release,
                    Some(run.clone()),
                    CommandBody::Stop,
                ))
                .await,
        )
    } else {
        None
    }
}
async fn usage_boundary(
    boundary: usize,
    first: UsageObservation,
    second: UsageObservation,
    cumulative: UsageObservation,
) {
    let fixture = Fixture::new();
    script_usage_turns(&fixture, first, second);
    let (entered, release) = if boundary == 2 {
        fixture.engine.pause_after_reconcile_at(2)
    } else {
        fixture.engine.pause_after_reserve_at(2)
    };
    if boundary == 0 {
        fixture
            .engine
            .fail_after_reserve_at(2, EngineError::Unavailable);
    }
    let run = reconciliation::admitted_with_operations(&fixture, 1).await;
    let original_control = fixture.control().await;
    let actor = fixture.actor.clone();
    let worker = tokio::spawn(async move { actor.drive_one().await });
    tokio::time::timeout(std::time::Duration::from_secs(5), entered.wait())
        .await
        .unwrap();
    let usage = || reported_usage(&fixture, &run);
    let mut expected = vec![
        UsageObservation::default(),
        first,
        UsageObservation::default(),
    ];
    if boundary == 2 {
        expected.push(cumulative);
    }
    let observed_before_release = usage();
    let control_before_release = fixture.control().await;
    let (reservations, controlling) = budget_reservations(&fixture).await;
    let stopped = stop_after_boundary(&fixture, boundary, control_before_release, &run).await;
    // Release even if a regression changes the captured facts or Stop result.
    release.release();
    let result = tokio::time::timeout(std::time::Duration::from_secs(5), worker)
        .await
        .unwrap()
        .unwrap();
    assert!(result.is_err());
    assert_eq!(observed_before_release, expected);
    assert_eq!(usage(), expected);
    assert_eq!(control_before_release, original_control);
    assert_eq!(reservations, if boundary == 2 { 4 } else { 3 });
    assert!(!controlling);
    if let Some(result) = stopped {
        result.unwrap();
    }
    assert_final_status(&fixture, &run, boundary).await;
}
#[tokio::test]
async fn reported_usage_budget_ack_publishes_unknown_and_observed_totals_without_sdk_progress() {
    let first = UsageObservation {
        input_tokens: Some(11),
        output_tokens: Some(7),
        total_tokens: Some(18),
        cached_input_tokens: Some(3),
        thought_tokens: Some(2),
        cost_micros: Some(5),
    };
    let second = UsageObservation {
        input_tokens: Some(4),
        output_tokens: Some(3),
        total_tokens: Some(7),
        cached_input_tokens: Some(0),
        thought_tokens: Some(0),
        cost_micros: Some(7),
    };
    let cumulative = UsageObservation {
        input_tokens: Some(15),
        output_tokens: Some(10),
        total_tokens: Some(25),
        cached_input_tokens: Some(3),
        thought_tokens: Some(2),
        cost_micros: Some(12),
    };
    // Actual reserve failure, cancellation after reserve, cancellation after
    // observation ACK: none can rely on a later SDK Usage callback.
    for boundary in 0..3 {
        usage_boundary(boundary, first, second, cumulative).await;
    }
}
