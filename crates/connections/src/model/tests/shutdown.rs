use super::*;

pub(super) fn recent_profiles(profiles: &[Profile]) -> Recents {
    let mut recents = Recents::default();
    for profile in profiles {
        recents.record(Recent {
            options: profile.options.clone(),
            target: profile.target.clone(),
            auth: profile.auth.clone(),
            profile: Some(profile.id),
            last_used: 1,
            credential: profile.credential,
            launch: profile.launch.clone(),
        });
    }
    recents
}
fn disk_recents(path: std::path::PathBuf) -> SnapshotWriter<Recents> {
    Arc::new(move |recents| {
        let path = path.clone();
        Box::pin(async move { persist::save(&path, &recents).map_err(StoreError::backend) })
    })
}

#[gpui_kit::test]
async fn active_prefix_and_independent_recents_are_durable_while_final_profile_hangs(
    cx: &mut gpui_kit::TestAppContext,
) {
    let (mut a, mut b) = (profile("A"), profile("B"));
    a.target.host = "a".into();
    b.target.host = "b".into();
    let independent = profile("Independent");
    let directory = tempfile::tempdir().unwrap();
    let profile_path = directory.path().join("profiles.toml");
    let recents_path = directory.path().join("recents.toml");
    let (release, blocked) = oneshot::channel();
    let blocked = Arc::new(Mutex::new(Some(blocked)));
    let saved_path = profile_path.clone();
    let writer: SnapshotWriter<Profiles> = Arc::new(move |profiles| {
        let blocked = blocked.lock().unwrap().take();
        let path = saved_path.clone();
        Box::pin(async move {
            if let Some(blocked) = blocked {
                blocked.await.unwrap();
            } else {
                return futures::future::pending().await;
            }
            persist::save(&path, &profiles).map_err(StoreError::backend)
        })
    });
    let shutdown = ShutdownCoordinator::default();
    let entity = cx.update(|cx| {
        Connections::with_backends(
            documents(&[a.clone(), b.clone()]),
            recent_profiles(&[a.clone(), b.clone()]),
            writer,
            disk_recents(recents_path.clone()),
        )
        .into_entity(&shutdown, cx)
        .unwrap()
    });
    let active = entity.update(cx, |c, cx| c.delete_profile(a.id, cx));
    cx.run_until_parked();
    let final_save = entity.update(cx, |c, cx| c.delete_profile(b.id, cx));
    entity.update(cx, |c, cx| {
        c.record_use(&spec_for_profile(&independent), None, cx)
    });
    let work = cx.update(|cx| shutdown.capture_all(cx));
    drop(entity);
    let task = cx.background_executor.spawn(work);
    cx.run_until_parked();
    release.send(()).unwrap();
    cx.run_until_parked();
    let saved: Recents = persist::load(&recents_path).unwrap().unwrap();
    assert!(
        saved
            .iter()
            .any(|recent| recent.target == independent.target)
    );
    assert_eq!(
        saved
            .iter()
            .find(|recent| recent.target == a.target)
            .unwrap()
            .profile,
        None
    );
    assert_eq!(
        saved
            .iter()
            .find(|recent| recent.target == b.target)
            .unwrap()
            .profile,
        Some(b.id)
    );
    let profiles: Profiles = persist::load(&profile_path).unwrap().unwrap();
    assert!(profiles.get(a.id).is_none() && profiles.get(b.id).is_some());
    active.await.unwrap();
    let report = task.await;
    assert!(report.timed_out());
    assert!(matches!(report.failures(), [StoreError::DeadlineExceeded]));
    drop(final_save);
}

#[gpui_kit::test]
async fn equal_profile_payload_still_requires_real_publication_before_unlinking(
    cx: &mut gpui_kit::TestAppContext,
) {
    for fail in [true, false] {
        let original = profile("Original");
        let writes = Arc::new(Mutex::new(Vec::new()));
        let observed = writes.clone();
        let writer: SnapshotWriter<Profiles> = Arc::new(move |profiles| {
            observed.lock().unwrap().push(profiles);
            Box::pin(async move {
                if fail {
                    Err(StoreError::PendingWrites)
                } else {
                    Ok(())
                }
            })
        });
        let directory = tempfile::tempdir().unwrap();
        let recents_path = directory.path().join("recents.toml");
        let shutdown = ShutdownCoordinator::default();
        let initial = documents(std::slice::from_ref(&original));
        let entity = cx.update(|cx| {
            Connections::with_backends(
                initial.clone(),
                recent_profiles(std::slice::from_ref(&original)),
                writer,
                disk_recents(recents_path.clone()),
            )
            .into_entity(&shutdown, cx)
            .unwrap()
        });
        let (deleted, readded) = entity.update(cx, |c, cx| {
            (
                c.delete_profile(original.id, cx),
                c.save_profile(original.clone(), cx),
            )
        });
        let work = cx.update(|cx| shutdown.capture_all(cx));
        let report = cx.background_executor.spawn(work).await;
        assert_eq!(*writes.lock().unwrap(), [initial]);
        assert_eq!(report.failures().is_empty(), !fail);
        assert_eq!(deleted.await.is_ok(), !fail);
        assert_eq!(readded.await.is_ok(), !fail);
        let saved = persist::load::<Recents>(&recents_path).unwrap();
        if fail {
            assert!(saved.is_none());
            assert_eq!(
                entity.read_with(cx, |c, cx| c.recents(cx).iter().next().unwrap().profile),
                Some(original.id)
            );
        } else {
            assert_eq!(saved.unwrap().iter().next().unwrap().profile, None);
        }
    }
}

#[gpui_kit::test]
async fn failed_recents_never_revoke_a_successful_profile_receipt(
    cx: &mut gpui_kit::TestAppContext,
) {
    let original = profile("Original");
    let directory = tempfile::tempdir().unwrap();
    let profile_path = directory.path().join("profiles.toml");
    let saved_path = profile_path.clone();
    let writer: SnapshotWriter<Profiles> = Arc::new(move |profiles| {
        let path = saved_path.clone();
        Box::pin(async move { persist::save(&path, &profiles).map_err(StoreError::backend) })
    });
    let recents_writer = Arc::new(|_| {
        Box::pin(async { Err(StoreError::PendingWrites) })
            as futures::future::BoxFuture<'static, Result<(), StoreError>>
    });
    let shutdown = ShutdownCoordinator::default();
    let entity = cx.update(|cx| {
        Connections::with_backends(
            documents(std::slice::from_ref(&original)),
            recent_profiles(std::slice::from_ref(&original)),
            writer,
            recents_writer,
        )
        .into_entity(&shutdown, cx)
        .unwrap()
    });
    let deleted = entity.update(cx, |c, cx| c.delete_profile(original.id, cx));
    let work = cx.update(|cx| shutdown.capture_all(cx));
    let report = cx.background_executor.spawn(work).await;
    assert_eq!(report.failures().len(), 1);
    deleted.await.unwrap();
    assert!(
        persist::load::<Profiles>(&profile_path)
            .unwrap()
            .unwrap()
            .is_empty()
    );
}

#[gpui_kit::test]
async fn independent_recents_save_even_when_profiles_fail(cx: &mut gpui_kit::TestAppContext) {
    let original = profile("Original");
    let directory = tempfile::tempdir().unwrap();
    let recents_path = directory.path().join("recents.toml");
    let writer: SnapshotWriter<Profiles> =
        Arc::new(|_| Box::pin(async { Err(StoreError::PendingWrites) }));
    let shutdown = ShutdownCoordinator::default();
    let entity = cx.update(|cx| {
        Connections::with_backends(
            documents(std::slice::from_ref(&original)),
            Recents::default(),
            writer,
            disk_recents(recents_path.clone()),
        )
        .into_entity(&shutdown, cx)
        .unwrap()
    });
    let deleted = entity.update(cx, |c, cx| c.delete_profile(original.id, cx));
    entity.update(cx, |c, cx| {
        c.record_use(&spec_for_profile(&original), Some(original.id), cx)
    });
    let work = cx.update(|cx| shutdown.capture_all(cx));
    let report = cx.background_executor.spawn(work).await;
    assert_eq!(report.failures().len(), 1);
    assert!(deleted.await.is_err());
    assert_eq!(
        persist::load::<Recents>(&recents_path)
            .unwrap()
            .unwrap()
            .iter()
            .next()
            .unwrap()
            .profile,
        Some(original.id)
    );
}

#[gpui_kit::test]
async fn final_capture_retries_the_failed_active_intent_as_part_of_its_complete_document(
    cx: &mut gpui_kit::TestAppContext,
) {
    let (mut a, mut b) = (profile("A"), profile("B"));
    a.target.host = "a".into();
    b.target.host = "b".into();
    let (release, blocked) = oneshot::channel();
    let blocked = Arc::new(Mutex::new(Some(blocked)));
    let writes = Arc::new(Mutex::new(Vec::new()));
    let saved = writes.clone();
    let writer: SnapshotWriter<Profiles> = Arc::new(move |profiles| {
        let blocked = blocked.lock().unwrap().take();
        let saved = saved.clone();
        Box::pin(async move {
            if let Some(blocked) = blocked {
                blocked.await.unwrap();
                return Err(StoreError::PendingWrites);
            }
            saved.lock().unwrap().push(profiles);
            Ok(())
        })
    });
    let directory = tempfile::tempdir().unwrap();
    let recents_path = directory.path().join("recents.toml");
    let shutdown = ShutdownCoordinator::default();
    let entity = cx.update(|cx| {
        Connections::with_backends(
            documents(&[a.clone(), b.clone()]),
            recent_profiles(&[a.clone(), b.clone()]),
            writer,
            disk_recents(recents_path.clone()),
        )
        .into_entity(&shutdown, cx)
        .unwrap()
    });
    let active = entity.update(cx, |c, cx| c.delete_profile(a.id, cx));
    cx.run_until_parked();
    let final_receipt = entity.update(cx, |c, cx| c.delete_profile(b.id, cx));
    let work = cx.update(|cx| shutdown.capture_all(cx));
    let task = cx.background_executor.spawn(work);
    release.send(()).unwrap();
    assert!(task.await.failures().is_empty());
    active.await.unwrap();
    final_receipt.await.unwrap();
    assert_eq!(writes.lock().unwrap().len(), 1);
    assert!(writes.lock().unwrap()[0].is_empty());
    assert!(
        persist::load::<Recents>(&recents_path)
            .unwrap()
            .unwrap()
            .iter()
            .all(|recent| recent.profile.is_none())
    );
}

#[gpui_kit::test]
async fn final_failure_preserves_physical_prefix_effects_and_final_success_applies_all(
    cx: &mut gpui_kit::TestAppContext,
) {
    for fail_final in [true, false] {
        let (mut a, mut b) = (profile("A"), profile("B"));
        a.target.host = "a".into();
        b.target.host = "b".into();
        let (release, blocked) = oneshot::channel();
        let blocked = Arc::new(Mutex::new(Some(blocked)));
        let directory = tempfile::tempdir().unwrap();
        let profile_path = directory.path().join("profiles.toml");
        let saved_path = profile_path.clone();
        let recents_path = directory.path().join("recents.toml");
        let writer: SnapshotWriter<Profiles> = Arc::new(move |profiles| {
            let blocked = blocked.lock().unwrap().take();
            let path = saved_path.clone();
            Box::pin(async move {
                if let Some(blocked) = blocked {
                    blocked.await.unwrap();
                } else if fail_final {
                    return Err(StoreError::PendingWrites);
                }
                persist::save(&path, &profiles).map_err(StoreError::backend)
            })
        });
        let shutdown = ShutdownCoordinator::default();
        let entity = cx.update(|cx| {
            Connections::with_backends(
                documents(&[a.clone(), b.clone()]),
                recent_profiles(&[a.clone(), b.clone()]),
                writer,
                disk_recents(recents_path.clone()),
            )
            .into_entity(&shutdown, cx)
            .unwrap()
        });
        let active = entity.update(cx, |c, cx| c.delete_profile(a.id, cx));
        cx.run_until_parked();
        let final_receipt = entity.update(cx, |c, cx| c.delete_profile(b.id, cx));
        let work = cx.update(|cx| shutdown.capture_all(cx));
        let task = cx.background_executor.spawn(work);
        release.send(()).unwrap();
        let report = task.await;
        assert_eq!(report.failures().is_empty(), !fail_final);
        active.await.unwrap();
        assert_eq!(final_receipt.await.is_ok(), !fail_final);
        let profiles: Profiles = persist::load(&profile_path).unwrap().unwrap();
        assert!(profiles.get(a.id).is_none());
        assert_eq!(profiles.get(b.id).is_some(), fail_final);
        let saved: Recents = persist::load(&recents_path).unwrap().unwrap();
        assert_eq!(
            saved
                .iter()
                .find(|recent| recent.target == a.target)
                .unwrap()
                .profile,
            None
        );
        assert_eq!(
            saved
                .iter()
                .find(|recent| recent.target == b.target)
                .unwrap()
                .profile
                .is_some(),
            fail_final
        );
    }
}
