//! Native compaction preserves complete history and never trusts only an SDK checkpoint.
use super::*;

async fn completed(fixture: &Fixture) -> RunBinding {
    fixture.engine.push(complete());
    fixture
        .actor
        .accept(submit(1, ChatControlRevision::default()))
        .await
        .unwrap();
    let run = fixture.binding().await;
    fixture
        .actor
        .accept(inference(2, fixture.control().await, run.clone()))
        .await
        .unwrap();
    fixture.actor.drive_one().await.unwrap();
    run
}
async fn acknowledged(fixture: &Fixture) -> JournalHead {
    let head = fixture.journal.head(stream()).await.unwrap().unwrap();
    fixture.actor.flush_outbox(stream()).await.unwrap();
    head
}
#[tokio::test]
async fn acknowledged_native_snapshot_restores_completed_runs_and_the_next_prompt_history() {
    let mut fixture = Fixture::new();
    let original = completed(&fixture).await;
    let head = acknowledged(&fixture).await;
    let snapshot = fixture.actor.compact(stream()).await.unwrap();
    assert_eq!(snapshot.at(), head.position());
    assert!(matches!(
        fixture
            .journal
            .read(stream(), JournalSequence::default(), limits())
            .await,
        Err(JournalError::Gap { .. })
    ));
    fixture.restart();
    fixture.actor.recover().await.unwrap();
    assert_eq!(
        fixture.actor.status(&original).await.unwrap(),
        RunStatus::Completed
    );
    assert!(!fixture.actor.drive_one().await.unwrap());
    assert_eq!(fixture.engine.calls(), 1);
    let receipt = fixture
        .actor
        .accept(submit(3, fixture.control().await))
        .await
        .unwrap();
    // The next run is available through its durable PromptAccepted publication;
    // historical original records are now physically held only in the archive.
    let next = fixture
        .events
        .events()
        .into_iter()
        .rev()
        .find_map(|event| match event {
            CoordinatorEvent::PromptAccepted { command, run, .. }
                if command.id() == receipt.acceptance().id() =>
            {
                Some(run)
            }
            _ => None,
        })
        .unwrap();
    fixture
        .actor
        .accept(inference(4, fixture.control().await, next.clone()))
        .await
        .unwrap();
    fixture.engine.push(complete());
    fixture.actor.drive_one().await.unwrap();
    let configs = fixture.engine.configs();
    let history = &configs.last().unwrap().history;
    assert!(history.messages().iter().any(|message|matches!(message,EngineHistoryMessage::Assistant(assistant) if assistant.text()=="answer")));
    assert_eq!(fixture.engine.calls(), 2);
    acknowledged(&fixture).await;
    fixture.actor.compact(stream()).await.unwrap();
    fixture.restart();
    fixture.actor.recover().await.unwrap();
    assert_eq!(
        fixture.actor.status(&next).await.unwrap(),
        RunStatus::Completed
    );
    assert!(!fixture.actor.drive_one().await.unwrap());
    assert_eq!(fixture.engine.calls(), 2);
}
#[tokio::test]
async fn a_retained_prefix_without_native_recovery_state_never_uses_only_an_sdk_checkpoint() {
    let mut fixture = Fixture::new();
    completed(&fixture).await;
    let head = acknowledged(&fixture).await;
    assert!(
        fixture
            .journal
            .checkpoint(stream())
            .await
            .unwrap()
            .is_some()
    );
    assert!(matches!(
        fixture
            .journal
            .compact(
                head.lease().clone(),
                head.position(),
                JournalSnapshot::new(stream(), head.position(), bytes(b"foreign snapshot format"))
                    .unwrap(),
            )
            .await,
        Err(JournalError::Conflict)
    ));
    fixture.actor.compact(stream()).await.unwrap();
    let db = rusqlite::Connection::open(fixture.root.path().join("cloud-runs.sqlite3")).unwrap();
    db.execute("DELETE FROM snapshots", []).unwrap();
    fixture.restart();
    assert!(matches!(
        fixture.actor.recover().await,
        Err(CoordinatorError::Journal(JournalError::Corrupt))
    ));
    assert_eq!(fixture.engine.calls(), 1);
}

#[tokio::test]
async fn active_native_archive_cannot_be_replaced_by_oversized_records_trailing_bytes_or_false_anchors()
 {
    for mutation in 0..3 {
        let mut fixture = Fixture::new();
        completed(&fixture).await;
        acknowledged(&fixture).await;
        let snapshot = fixture.actor.compact(stream()).await.unwrap();
        let mut malformed = snapshot.payload().as_bytes().to_vec();
        let binding = u32::from_le_bytes(malformed[5..9].try_into().unwrap()) as usize;
        let metadata = 9 + binding;
        let checkpoint_length =
            u32::from_le_bytes(malformed[metadata + 65..metadata + 69].try_into().unwrap())
                as usize;
        let count_offset = metadata + 69 + checkpoint_length;
        match mutation {
            0 => malformed[count_offset..count_offset + 4].copy_from_slice(&4097u32.to_le_bytes()),
            1 => malformed.push(0),
            _ => malformed[metadata + 49] ^= 1,
        }
        let head = fixture.journal.head(stream()).await.unwrap().unwrap();
        assert!(matches!(
            fixture
                .journal
                .compact(
                    head.lease().clone(),
                    snapshot.at(),
                    JournalSnapshot::new(
                        stream(),
                        snapshot.at(),
                        nocterm_cloud_model::payload::OpaqueBytes::new(malformed).unwrap(),
                    )
                    .unwrap(),
                )
                .await,
            Err(JournalError::Conflict)
        ));
        fixture.restart();
        fixture.actor.recover().await.unwrap();
        assert_eq!(fixture.engine.calls(), 1);
    }
}

#[tokio::test]
async fn snapshot_publication_rechecks_original_source_after_a_gated_native_read() {
    let transport = Arc::new(ScriptedPublicationTransport::default());
    let fixture = Fixture::with_transport(
        SupervisorLimits::default(),
        CommandReceiveBudget::new(IntakeLimits::default()),
        transport.clone(),
    );
    fixture
        .actor
        .accept(submit(1, ChatControlRevision::default()))
        .await
        .unwrap();
    let (entered, release) = fixture.cipher.pause_next_open(JournalStorageClass::Outbox);
    let actor = fixture.actor.clone();
    let flushing = tokio::spawn(async move { actor.flush_outbox(stream()).await });
    tokio::task::spawn_blocking(move || entered.recv_timeout(std::time::Duration::from_secs(5)))
        .await
        .unwrap()
        .unwrap();
    fixture.source.revoke();
    release.send(()).unwrap();
    assert!(matches!(
        flushing.await.unwrap(),
        Err(CoordinatorError::StaleSource)
    ));
    assert!(transport.attempts().is_empty());
    assert!(transport.purged().is_empty());
    let publication = snapshot_publication(&fixture.journal);
    assert_eq!(
        publication.delivered(stream()).await.unwrap(),
        EventSequence::default()
    );
    assert_eq!(fixture.engine.calls(), 0);
}
