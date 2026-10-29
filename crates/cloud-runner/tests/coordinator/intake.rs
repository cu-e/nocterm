//! Queue/byte pressure leaves exact-ID retries pending and reserved Stop usable.
use super::*;
use futures::future::AbortHandle;
use std::time::Duration;

fn receive_budget() -> CommandReceiveBudget {
    let commands = IntakePoolLimits::new(2 * 1024 * 1024, 16, 16).unwrap();
    let ack = IntakePoolLimits::new(16 * 1024, 1, 1).unwrap();
    CommandReceiveBudget::new(IntakeLimits::new(commands, commands, commands, ack, ack).unwrap())
}
fn other_chat_submit(id: u8) -> ReceivedCommand {
    let original = submit(id, ChatControlRevision::default());
    let at = original.stream();
    ReceivedCommand::new(
        StreamBinding::new(
            at.scope().clone(),
            at.runner(),
            at.runner_epoch(),
            CloudChatId::from_bytes([0x76; 16]),
        ),
        original.acceptance().clone(),
        original.window(),
        None,
        original.body().clone(),
        None,
    )
    .unwrap()
}
async fn await_receipt(fixture: &Fixture, command: &ReceivedCommand) -> CommandReceipt {
    // The serial native journal deliberately refuses concurrent callers rather
    // than parking an unbounded queue. Observe the actual durable receipt at its
    // shared source boundary instead of stealing the SQLite lock in a hot loop.
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Some(receipt) = fixture.source.replies().iter().find_map(|(ack, reply)| {
                if ack.acceptance().id() != command.acceptance().id() {
                    return None;
                }
                match reply {
                    CommandReply::Accepted(receipt) => Some(receipt.clone()),
                    _ => None,
                }
            }) {
                return receipt;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap()
}
async fn pressure_preserves_stop_and_original_retry(body_pressure: bool) {
    let budget = receive_budget();
    let fixture = Fixture::with_receive_budget(budget.clone());
    fixture
        .actor
        .accept(submit(1, ChatControlRevision::default()))
        .await
        .unwrap();
    let run = fixture.binding().await;
    let withheld = other_chat_submit(2);
    let stop = command(
        3,
        fixture.control().await,
        Some(run.clone()),
        CommandBody::Stop,
    );
    let lane = if body_pressure {
        IntakeLane::Ordinary
    } else {
        IntakeLane::Acknowledgement
    };
    let held = budget
        .try_reserve(lane, budget.limits(lane).bytes(), 0)
        .unwrap();
    let acknowledgements = fixture.source.pause_acknowledgements();
    fixture.source.push(withheld.clone()).unwrap();
    fixture.source.push(stop.clone()).unwrap();
    let actor = fixture.actor.clone();
    let (abort, cancellation) = AbortHandle::new_pair();
    let serving = tokio::spawn(async move { actor.serve(cancellation).await });
    let stopped = await_receipt(&fixture, &stop).await;
    assert_eq!(stopped.acceptance().id(), stop.acceptance().id());
    assert_eq!(
        stopped.position().control(),
        stop.acceptance().expected_control().next().unwrap()
    );
    tokio::time::timeout(Duration::from_secs(5), async {
        while fixture.actor.status(&run).await.unwrap() != RunStatus::Stopped {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(
        fixture
            .journal
            .command_receipt(withheld.stream().clone(), withheld.acceptance().id())
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(fixture.engine.calls(), 0);
    tokio::time::timeout(Duration::from_secs(5), async {
        while !fixture.source.replies().iter().any(|(ack, reply)| {
            ack.acceptance().id() == stop.acceptance().id()
                && matches!(reply, CommandReply::Accepted(_))
        }) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        budget
            .usage(IntakeLane::UrgentAcknowledgement)
            .unwrap()
            .allocations,
        1
    );
    if body_pressure {
        assert_eq!(
            budget.usage(IntakeLane::Ordinary).unwrap().bytes,
            held.bytes()
        );
        assert!(
            fixture
                .source
                .replies()
                .iter()
                .any(
                    |(ack, reply)| ack.acceptance().id() == withheld.acceptance().id()
                        && matches!(reply, CommandReply::RetryLater(RetryReason::Capacity))
                )
        );
    } else {
        assert_eq!(budget.usage(IntakeLane::Ordinary).unwrap().bytes, 0);
        assert!(
            !fixture
                .source
                .replies()
                .iter()
                .any(|(ack, _)| ack.acceptance().id() == withheld.acceptance().id())
        );
    }
    drop(held);
    acknowledgements.release();
    tokio::time::timeout(Duration::from_secs(5), async {
        while budget
            .usage(IntakeLane::Acknowledgement)
            .unwrap()
            .allocations
            != 0
            || budget
                .usage(IntakeLane::UrgentAcknowledgement)
                .unwrap()
                .allocations
                != 0
        {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    fixture.source.push(withheld.clone()).unwrap();
    let receipt = await_receipt(&fixture, &withheld).await;
    // A lost response retries the original immutable ID/digest, without another
    // logical admission or any automatic effect redispatch.
    assert_eq!(
        fixture
            .actor
            .accept(withheld.clone())
            .await
            .unwrap()
            .position(),
        receipt.position()
    );
    abort.abort();
    assert!(matches!(
        serving.await.unwrap(),
        Err(CoordinatorError::Stopped)
    ));
    assert_eq!(
        fixture
            .actor
            .shutdown(nocterm_core::persist::ShutdownDeadline::new(
                Duration::from_secs(5)
            ))
            .await
            .unwrap(),
        DrainReport::Drained
    );
    let history = fixture
        .journal
        .read(
            withheld.stream().clone(),
            JournalSequence::default(),
            limits(),
        )
        .await
        .unwrap();
    assert_eq!(history.records().iter().filter(|record| matches!(record.record().kind(), JournalRecordKind::CommandAccepted{id,..} if *id == withheld.acceptance().id())).count(), 1);
}
#[tokio::test]
async fn reserved_publication_defers_ordinary_without_acceptance_while_stop_and_its_status_commit()
{
    pressure_preserves_stop_and_original_retry(true).await;
}
#[tokio::test]
async fn a_full_ordinary_ack_pool_releases_the_body_and_preserves_reserved_stop() {
    pressure_preserves_stop_and_original_retry(false).await;
}

#[tokio::test]
async fn reserved_control_capacity_wakes_the_original_run_without_another_prompt_or_http_retry() {
    let budget = receive_budget();
    let fixture = Fixture::with_receive_budget(budget.clone());
    let run = waiting_for_finite_grant(&fixture).await;
    let held = budget
        .try_reserve(
            IntakeLane::Ordinary,
            budget.limits(IntakeLane::Ordinary).bytes(),
            0,
        )
        .unwrap();
    fixture.source.push(other_chat_submit(4)).unwrap();
    let grant = inference(3, fixture.control().await, run.clone());
    fixture.source.push(grant.clone()).unwrap();
    let deferred = fixture
        .source
        .receive(budget.clone())
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(
        deferred,
        CommandInput::Deferred {
            reason: RetryReason::Capacity,
            ..
        }
    ));
    drop(deferred);
    let CommandInput::Ready(received) = fixture
        .source
        .receive(budget.clone())
        .await
        .unwrap()
        .unwrap()
    else {
        panic!("finite grant could not use reserved control capacity")
    };
    assert_eq!(received.value().acceptance().id(), grant.acceptance().id());
    fixture.actor.accept_resident(received).await.unwrap();
    assert_eq!(
        budget.usage(IntakeLane::Ordinary).unwrap().bytes,
        held.bytes()
    );
    assert_eq!(fixture.engine.calls(), 0);
    drop(held);
    fixture.engine.push(complete());
    assert!(fixture.actor.drive_one().await.unwrap());
    assert_eq!(
        fixture.actor.status(&run).await.unwrap(),
        RunStatus::Completed
    );
    assert_eq!(fixture.engine.calls(), 1);
    assert_eq!(
        fixture
            .events
            .events()
            .iter()
            .filter(|event| matches!(event, CoordinatorEvent::PromptAccepted { .. }))
            .count(),
        1
    );
    let page = fixture
        .journal
        .read(stream(), JournalSequence::default(), limits())
        .await
        .unwrap();
    assert_eq!(
        page.records()
            .iter()
            .filter(|record| matches!(record.record().kind(),
        JournalRecordKind::CommandAccepted { id, .. } if *id == grant.acceptance().id()))
            .count(),
        1
    );
}

async fn waiting_for_finite_grant(fixture: &Fixture) -> RunBinding {
    fixture.engine.push(complete());
    fixture
        .actor
        .accept(submit(1, ChatControlRevision::default()))
        .await
        .unwrap();
    let run = fixture.binding().await;
    let control = fixture.control().await;
    let tiny = InferenceGrant::new(
        GrantId::from_bytes([2; 16]),
        GrantEpoch::new(1).unwrap(),
        GrantOrigin::new(
            issuer(),
            CommandId::from_bytes([2; 16]),
            control.next().unwrap(),
        ),
        InferenceTask::new(
            run.clone(),
            selection(),
            ModelBudget::new(1, 1, 1, 1).unwrap(),
            1,
            window(),
        )
        .unwrap(),
    );
    fixture
        .actor
        .accept(command(
            2,
            control,
            Some(run.clone()),
            CommandBody::InferenceGrant(tiny),
        ))
        .await
        .unwrap();
    assert!(fixture.actor.drive_one().await.is_err());
    assert_eq!(
        fixture.actor.status(&run).await.unwrap(),
        RunStatus::NeedsGrant
    );
    assert_eq!(fixture.engine.calls(), 0);
    run
}
