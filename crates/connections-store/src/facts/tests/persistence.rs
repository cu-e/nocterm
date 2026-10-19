use super::*;
use nocterm_connections_model::{
    os::OsId,
    repository::{
        ConnectionDocument, ConnectionRepository, ConnectionStorageError, LoadedConnections,
        StorageOperation,
    },
    test_support::InMemoryConnectionRepository,
};
#[gpui::test]
async fn visible_observations_retry_at_quit_after_a_normal_write_failure(cx: &mut TestAppContext) {
    let repository = Arc::new(InMemoryConnectionRepository::new(
        LoadedConnections::default(),
    ));
    repository.fail_next(ConnectionStorageError::new(
        ConnectionDocument::Facts,
        StorageOperation::Write,
        std::io::Error::from(std::io::ErrorKind::StorageFull),
    ));
    let backend = repository.clone();
    let writer: SnapshotWriter<Known> = Arc::new(move |known| {
        let backend = backend.clone();
        Box::pin(async move { backend.save_facts(&known).map_err(StoreError::backend) })
    });
    let fixture = Fixture::new(
        profile(),
        Default::default(),
        false,
        cx.background_executor.clone(),
        writer,
        cx,
    );
    let session = fs();
    fixture.os.answer_next(Ok(OsId::new("debian")));
    fixture.request(session.clone(), cx).unwrap();
    cx.run_until_parked();
    assert_eq!(fixture.raw_os(cx).as_deref(), Some("debian"));
    let work = cx.update(|cx| fixture.shutdown.capture_all(cx));
    let report = cx.background_executor.spawn(work).await;
    assert!(report.failures().is_empty());
    assert_eq!(
        repository.snapshot().known.os(fixture.profile.id),
        Some("debian")
    );
}
#[gpui::test]
async fn captured_known_survives_owner_drop_and_late_jobs_cannot_publish(cx: &mut TestAppContext) {
    let repository = Arc::new(InMemoryConnectionRepository::new(
        LoadedConnections::default(),
    ));
    let backend = repository.clone();
    let writer: SnapshotWriter<Known> = Arc::new(move |known| {
        let backend = backend.clone();
        Box::pin(async move { backend.save_facts(&known).map_err(StoreError::backend) })
    });
    let fixture = Fixture::new(
        profile(),
        Default::default(),
        false,
        cx.background_executor.clone(),
        writer,
        cx,
    );
    let session = fs();
    let gate = nocterm_session::testing::Gate::pending();
    fixture.os.held_next(Ok(OsId::new("debian")), gate.clone());
    fixture.request(session.clone(), cx).unwrap();
    cx.run_until_parked();
    let work = cx.update(|cx| fixture.shutdown.capture_all(cx));
    assert!(matches!(
        fixture.request(fs(), cx),
        Err(crate::FactsAdmissionError::Closing)
    ));
    gate.release();
    cx.run_until_parked();
    assert_eq!(fixture.raw_os(cx), None);
    drop(fixture);
    let report = cx.background_executor.spawn(work).await;
    assert!(report.failures().is_empty());
    assert!(repository.snapshot().known.countries().next().is_none());
}

#[gpui::test]
fn imported_authority_and_orphan_pruning_do_not_eagerly_rewrite_untouched_observations(
    cx: &mut TestAppContext,
) {
    let orphan = nocterm_connections_model::profiles::ProfileId::from_bytes([2; 16]);
    let known: Known =
        toml::from_str(&format!("[server.\"{orphan}\"]\ncountry = \"de\"\n")).unwrap();
    let repository = Arc::new(InMemoryConnectionRepository::new(LoadedConnections {
        known: known.clone(),
        ..Default::default()
    }));
    let backend = repository.clone();
    let writer: SnapshotWriter<Known> = Arc::new(move |known| {
        let backend = backend.clone();
        Box::pin(async move { backend.save_facts(&known).map_err(StoreError::backend) })
    });
    let fixture = Fixture::new(
        profile(),
        known.clone(),
        false,
        cx.background_executor.clone(),
        writer,
        cx,
    );
    cx.run_until_parked();
    assert!(repository.calls().is_empty());
    assert_eq!(repository.snapshot().known, known);
    assert!(
        fixture
            .facts
            .read_with(cx, |facts, cx| facts.country(orphan, cx).is_none())
    );
    assert!(fixture.facts.read_with(cx, |facts, _| matches!(
        facts.flag(CountryCode::new("de").unwrap()),
        Some(crate::FlagState::Unavailable)
    )));
}
