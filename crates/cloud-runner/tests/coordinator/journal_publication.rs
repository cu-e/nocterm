//! Cancelling an awaiter cannot release a publication still running in SQLite.
use super::*;

#[tokio::test]
async fn a_cancelled_command_waiter_keeps_actual_publication_charged_and_serialized() {
    let fixture = Fixture::new();
    fixture
        .actor
        .accept(submit(1, ChatControlRevision::default()))
        .await
        .unwrap();
    let run = fixture.binding().await;
    let command = inference(2, fixture.control().await, run);
    let (entered, release) = fixture.cipher.pause_next_seal();
    let actor = fixture.actor.clone();
    let original = command.clone();
    let waiting = tokio::spawn(async move { actor.accept(original).await });
    tokio::task::spawn_blocking(move || entered.recv_timeout(std::time::Duration::from_secs(5)))
        .await
        .unwrap()
        .unwrap();
    waiting.abort();
    assert!(waiting.await.unwrap_err().is_cancelled());
    assert_eq!(fixture.storage.in_flight_writes(), 1);
    // Grants are control commands: they are charged to the urgent lane, never
    // to ordinary prompt admission.
    assert!(command.body().high_priority());
    assert!(
        fixture
            .receive_budget
            .usage(IntakeLane::Urgent)
            .unwrap()
            .bytes
            >= command.reservation_bytes()
    );
    let actor = fixture.actor.clone();
    let retry = command.clone();
    let mut next = tokio::spawn(async move { actor.accept(retry).await });
    let blocked = tokio::time::timeout(std::time::Duration::from_millis(25), &mut next).await;
    let active = fixture.storage.in_flight_writes();
    // Always release the native gate before assertions, including on regression.
    release.send(()).unwrap();
    assert!(
        blocked.is_err(),
        "the second publication must await physical completion"
    );
    assert_eq!(active, 1);
    let receipt = next.await.unwrap().unwrap();
    assert_eq!(receipt.acceptance(), command.acceptance());
    assert_eq!(fixture.storage.in_flight_writes(), 0);
    assert_eq!(
        fixture
            .receive_budget
            .usage(IntakeLane::Urgent)
            .unwrap()
            .bytes,
        0
    );
    let page = fixture
        .journal
        .read(stream(), JournalSequence::default(), limits())
        .await
        .unwrap();
    assert_eq!(page.records().iter().filter(|stored| matches!(stored.record().kind(), JournalRecordKind::CommandAccepted { id, .. } if *id == command.acceptance().id())).count(), 1);
    assert_eq!(fixture.engine.calls(), 0);
}

#[tokio::test]
async fn an_abandoned_admission_waiter_still_installs_its_committed_grant_and_wakes_the_run() {
    let fixture = Fixture::new();
    fixture
        .actor
        .accept(submit(1, ChatControlRevision::default()))
        .await
        .unwrap();
    let run = fixture.binding().await;
    let command = inference(2, fixture.control().await, run.clone());
    let (entered, release) = fixture.cipher.pause_next_seal();
    let actor = fixture.actor.clone();
    let waiting = tokio::spawn(async move { actor.accept(command).await });
    tokio::task::spawn_blocking(move || entered.recv_timeout(std::time::Duration::from_secs(5)))
        .await
        .unwrap()
        .unwrap();
    waiting.abort();
    assert!(waiting.await.unwrap_err().is_cancelled());
    release.send(()).unwrap();
    fixture.engine.push(complete());
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while !fixture.actor.drive_one().await.unwrap() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        fixture.actor.status(&run).await.unwrap(),
        RunStatus::Completed
    );
    assert_eq!(fixture.engine.calls(), 1);
}
