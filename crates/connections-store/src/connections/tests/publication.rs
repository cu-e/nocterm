//! Committed authority is observed through public carriers and held writer ports.
use super::*;
use nocterm_connections_model::{
    repository::{
        ConnectionDocument, ConnectionRepository, ConnectionStorageError, LoadedConnections,
        StorageOperation,
    },
    test_support::InMemoryConnectionRepository,
};
use nocterm_session::testing::Gate;

fn repository_writer(
    repository: Arc<InMemoryConnectionRepository>,
    gate: Gate,
) -> SnapshotWriter<Profiles> {
    Arc::new(move |profiles| {
        let (repository, gate) = (repository.clone(), gate.clone());
        Box::pin(async move {
            gate.wait().await;
            repository
                .save_profiles(&profiles)
                .map_err(StoreError::backend)
        })
    })
}
fn projection(
    entity: &Entity<Connections>,
    cx: &mut gpui::TestAppContext,
) -> crate::ProfilePublication {
    let carrier = entity.update(cx, |store, cx| store.publications(cx));
    carrier.read_with(cx, |carrier, _| carrier.current().clone())
}
#[gpui::test]
async fn pending_and_failed_profile_writes_do_not_publish_authority(cx: &mut gpui::TestAppContext) {
    let original = profile("Pending");
    let repository = Arc::new(InMemoryConnectionRepository::new(LoadedConnections {
        profiles: documents(std::slice::from_ref(&original)),
        ..Default::default()
    }));
    let gate = Gate::pending();
    let entity = cx.new(|_| {
        Connections::with_backends(
            documents(std::slice::from_ref(&original)),
            Recents::default(),
            repository_writer(repository.clone(), gate.clone()),
            ready_writer(),
        )
    });
    let initial = projection(&entity, cx);
    repository.fail_next(ConnectionStorageError::new(
        ConnectionDocument::Profiles,
        StorageOperation::Write,
        std::io::Error::from(std::io::ErrorKind::PermissionDenied),
    ));
    let mut changed = original.clone();
    changed.target.host = "changed".into();
    let receipt = entity.update(cx, |store, cx| store.save_profile(changed, cx));
    cx.run_until_parked();
    assert_eq!(projection(&entity, cx), initial);
    gate.release();
    assert!(receipt.await.is_err());
    assert_eq!(projection(&entity, cx), initial);
    assert_eq!(
        repository.snapshot().profiles,
        documents(std::slice::from_ref(&original))
    );
}
#[gpui::test]
async fn publication_is_visible_before_successful_receipt_and_contains_no_wire_stamps(
    cx: &mut gpui::TestAppContext,
) {
    let original = profile("Commit");
    let repository = Arc::new(InMemoryConnectionRepository::new(
        LoadedConnections::default(),
    ));
    let gate = Gate::pending();
    let entity = cx.new(|_| {
        Connections::with_backends(
            Profiles::default(),
            Recents::default(),
            repository_writer(repository.clone(), gate.clone()),
            ready_writer(),
        )
    });
    let carrier = entity.update(cx, |store, cx| store.publications(cx));
    let observed = Arc::new(Mutex::new(Vec::new()));
    let notifications = observed.clone();
    let subscription = cx.update(|cx| {
        cx.observe(&carrier, move |carrier, cx| {
            notifications
                .lock()
                .unwrap()
                .push(carrier.read(cx).current().generation())
        })
    });
    let receipt = entity.update(cx, |store, cx| store.save_profile(original.clone(), cx));
    cx.run_until_parked();
    assert_eq!(projection(&entity, cx).generation(), 0);
    gate.release();
    receipt.await.unwrap();
    assert_eq!(projection(&entity, cx).generation(), 1);
    cx.run_until_parked();
    assert_eq!(&*observed.lock().unwrap(), &[1]);
    let encoded = toml::to_string(&repository.snapshot().profiles).unwrap();
    assert!(
        !encoded.contains("incarnation")
            && !encoded.contains("endpoint")
            && !encoded.contains("override")
    );
    assert_eq!(
        toml::from_str::<Profiles>(&encoded).unwrap(),
        documents(std::slice::from_ref(&original))
    );
    drop(subscription);
}
#[gpui::test]
async fn memory_noops_and_unrelated_edits_preserve_authority(cx: &mut gpui::TestAppContext) {
    let original = profile("Memory");
    let entity = cx.new(|_| {
        Connections::with_documents(
            documents(std::slice::from_ref(&original)),
            Recents::default(),
        )
    });
    let initial = projection(&entity, cx);
    entity
        .update(cx, |store, cx| store.save_profile(original.clone(), cx))
        .await
        .unwrap();
    assert_eq!(projection(&entity, cx), initial);
    entity
        .update(cx, |store, cx| {
            store.move_profile(original.id, Some("Work".into()), cx)
        })
        .await
        .unwrap();
    let after = projection(&entity, cx);
    assert_eq!(after.generation(), 1);
    assert_eq!(
        after.get(original.id).unwrap().stamp,
        initial.get(original.id).unwrap().stamp
    );
    let stale = entity.update(cx, |store, cx| {
        store.save_profile_checked(original.clone(), Some(original.clone()), cx)
    });
    assert!(stale.await.is_err());
    assert_eq!(projection(&entity, cx), after);
}
#[gpui::test]
async fn real_endpoint_and_override_cycles_keep_each_transition_stamp(
    cx: &mut gpui::TestAppContext,
) {
    let original = profile("Cycle");
    let entity = cx.new(|_| {
        Connections::with_documents(
            documents(std::slice::from_ref(&original)),
            Recents::default(),
        )
    });
    let initial = projection(&entity, cx);
    let mut changed = original.clone();
    changed.target.host = "other".into();
    changed.icon = Some("debian".into());
    changed.country = Some("de".into());
    entity
        .update(cx, |store, cx| store.save_profile(changed, cx))
        .await
        .unwrap();
    entity
        .update(cx, |store, cx| store.save_profile(original.clone(), cx))
        .await
        .unwrap();
    let stamp = projection(&entity, cx).get(original.id).unwrap().stamp;
    assert_eq!(
        stamp.incarnation,
        initial.get(original.id).unwrap().stamp.incarnation
    );
    assert_eq!(stamp.endpoint, 2);
    assert_eq!(stamp.icon_override, 2);
    assert_eq!(stamp.country_override, 2);
    entity
        .update(cx, |store, cx| store.delete_profile(original.id, cx))
        .await
        .unwrap();
    assert!(projection(&entity, cx).get(original.id).is_none());
    entity
        .update(cx, |store, cx| store.save_profile(original.clone(), cx))
        .await
        .unwrap();
    assert_eq!(
        projection(&entity, cx)
            .get(original.id)
            .unwrap()
            .stamp
            .incarnation,
        4
    );
}

#[gpui::test]
async fn held_profile_queue_preserves_equal_payload_authority_history(
    cx: &mut gpui::TestAppContext,
) {
    let original = profile("Held cycle");
    let repository = Arc::new(InMemoryConnectionRepository::new(LoadedConnections {
        profiles: documents(std::slice::from_ref(&original)),
        ..Default::default()
    }));
    let gate = Gate::pending();
    let entity = cx.new(|_| {
        Connections::with_backends(
            documents(std::slice::from_ref(&original)),
            Recents::default(),
            repository_writer(repository.clone(), gate.clone()),
            ready_writer(),
        )
    });
    let initial = projection(&entity, cx);
    let mut changed = original.clone();
    changed.target.host = "other".into();
    changed.icon = Some("debian".into());
    changed.country = Some("de".into());
    let changed_receipt = entity.update(cx, |store, cx| store.save_profile(changed, cx));
    cx.run_until_parked();
    let restored = entity.update(cx, |store, cx| store.save_profile(original.clone(), cx));
    let deleted = entity.update(cx, |store, cx| store.delete_profile(original.id, cx));
    let readded = entity.update(cx, |store, cx| store.save_profile(original.clone(), cx));
    assert_eq!(projection(&entity, cx), initial);
    gate.release();
    changed_receipt.await.unwrap();
    restored.await.unwrap();
    deleted.await.unwrap();
    readded.await.unwrap();
    let committed = projection(&entity, cx);
    assert_eq!(committed.generation(), 4);
    assert_eq!(
        committed.get(original.id).unwrap().stamp,
        crate::ProfileStamp {
            incarnation: 4,
            endpoint: 4,
            icon_override: 4,
            country_override: 4,
        }
    );
    assert_eq!(repository.snapshot().profiles, documents(&[original]));
}

#[gpui::test]
async fn readonly_noops_and_rejections_publish_no_authority_or_writes(
    cx: &mut gpui::TestAppContext,
) {
    let original = profile("Read only");
    let diagnostic = Arc::new(ConnectionStorageError::new(
        ConnectionDocument::Profiles,
        StorageOperation::Read,
        std::io::Error::from(std::io::ErrorKind::PermissionDenied),
    ));
    let writes = Arc::new(Mutex::new(0));
    let counted = writes.clone();
    let writer: SnapshotWriter<Profiles> = Arc::new(move |_| {
        *counted.lock().unwrap() += 1;
        Box::pin(async { Ok(()) })
    });
    let entity = cx.new(|_| {
        Connections::prepared(
            documents(std::slice::from_ref(&original)),
            Recents::default(),
            writer,
            ready_writer(),
            Some(diagnostic.clone()),
            Arc::new(SuppliedValues::new(1, 0)),
        )
    });
    let initial = projection(&entity, cx);
    entity
        .update(cx, |store, cx| store.save_profile(original.clone(), cx))
        .await
        .unwrap();
    let rejected = entity.update(cx, |store, cx| store.delete_profile(original.id, cx));
    assert!(
        matches!(rejected.await, Err(ConnectionsError::ReadOnly(error)) if Arc::ptr_eq(&error, &diagnostic))
    );
    assert_eq!(projection(&entity, cx), initial);
    assert_eq!(*writes.lock().unwrap(), 0);
    entity.read_with(cx, |store, _| {
        assert!(std::ptr::eq(
            store.load_error().unwrap(),
            diagnostic.as_ref()
        ))
    });
}
