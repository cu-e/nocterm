//! Exact request ceilings produce a native diagnostic without granting another attempt.
use super::*;

fn notice(events: &[CoordinatorEvent]) -> Option<(usize, &str)> {
    events
        .iter()
        .enumerate()
        .find_map(|(index, event)| match event {
            CoordinatorEvent::Progress {
                event: EngineEvent::Text(text),
                ..
            } if text.text().starts_with("Nocterm error:") => Some((index, text.text())),
            _ => None,
        })
}

#[tokio::test]
async fn an_oversized_model_input_publishes_a_native_recovery_message_before_interruption() {
    let fixture = Fixture::new();
    fixture
        .engine
        .push_failure(EngineError::RequestTooLarge { limit: 1_048_576 });
    let run = reconciliation::admitted(&fixture).await;
    assert!(matches!(
        fixture.actor.drive_one().await,
        Err(CoordinatorError::Engine(EngineError::RequestTooLarge {
            limit: 1_048_576
        }))
    ));
    let events = fixture.events.events();
    let (diagnostic, text) =
        notice(&events).expect("native diagnosis reaches the normal transcript");
    assert!(text.contains("1048576 bytes") && text.contains("history, tools and image encoding"));
    assert!(text.contains("smaller image") && text.contains("new chat"));
    let interrupted = events
        .iter()
        .position(|event| {
            matches!(
                event,
                CoordinatorEvent::Status {
                    status: RunStatus::Interrupted,
                    ..
                }
            )
        })
        .unwrap();
    assert!(diagnostic < interrupted);
    assert_eq!(
        fixture.actor.status(&run).await.unwrap(),
        RunStatus::Interrupted
    );
    assert!(fixture.tools.requests().is_empty());
    let page = fixture
        .journal
        .read(stream(), JournalSequence::default(), limits())
        .await
        .unwrap();
    assert!(
        page.records()
            .iter()
            .any(|stored| matches!(stored.record().kind(), JournalRecordKind::Progress { .. }))
    );
    assert!(page.records().iter().all(|stored| !matches!(
        stored.record().kind(),
        JournalRecordKind::BudgetReserved { .. }
    )));
    assert_eq!(fixture.engine.calls(), 0);
}

#[tokio::test]
async fn a_request_over_the_operator_pricing_ceiling_reports_that_exact_limit() {
    let fixture = Fixture::new();
    let mut turn = complete();
    turn.request_bytes = 4097; // The trusted operator ceiling is checked before reserving the run.
    fixture.engine.push(turn);
    let run = reconciliation::admitted(&fixture).await;
    assert!(fixture.actor.drive_one().await.is_err());
    assert!(
        notice(&fixture.events.events())
            .unwrap()
            .1
            .contains("4096 bytes")
    );
    assert_eq!(
        fixture.actor.status(&run).await.unwrap(),
        RunStatus::Interrupted
    );
    assert_eq!(fixture.engine.calls(), 0);
}

#[tokio::test]
async fn a_failed_diagnostic_publication_still_records_terminal_status_and_retires_the_run() {
    let fixture = Fixture::new();
    fixture.engine.push(model_completion::listing_turn());
    fixture
        .tools
        .push(Ok(nocterm_cloud_model::payload::BoundedJson::new(
            serde_json::json!({"terminals": []}),
        )
        .unwrap()));
    fixture
        .engine
        .push_failure(EngineError::RequestTooLarge { limit: 1024 });
    let (entered, release) = fixture.engine.pause_after_reconcile_at(1);
    let run = reconciliation::admitted(&fixture).await;
    let driving = fixture.actor.drive_one();
    tokio::pin!(driving);
    tokio::select! {
        result = &mut driving => panic!("the first completed call must wait: {result:?}"),
        () = entered.wait() => {},
    }
    // Usage was already committed; the next Progress is the size diagnostic after tools.
    fixture.events.fail_next_progress();
    release.release();
    assert!(driving.await.is_err());
    assert!(
        notice(&fixture.events.events()).is_none(),
        "the diagnostic was not published"
    );
    assert_eq!(
        fixture.actor.status(&run).await.unwrap(),
        RunStatus::Interrupted
    );
    assert!(
        fixture
            .tools
            .retirements()
            .contains(&nocterm_cloud_model::tool::RunRetirement::Interrupted)
    );
    let page = fixture
        .journal
        .read(stream(), JournalSequence::default(), limits())
        .await
        .unwrap();
    assert!(matches!(
        page.records().last().unwrap().record().kind(),
        JournalRecordKind::RunStatus
    ));
}

#[tokio::test]
async fn an_unrelated_budget_failure_is_not_misreported_as_an_oversized_request() {
    let fixture = Fixture::new();
    fixture.engine.push_failure(EngineError::BudgetExceeded);
    let run = reconciliation::admitted(&fixture).await;
    assert!(fixture.actor.drive_one().await.is_err());
    assert!(notice(&fixture.events.events()).is_none());
    assert_eq!(
        fixture.actor.status(&run).await.unwrap(),
        RunStatus::Interrupted
    );
}
