use super::*;
use crate::ConnectionStore as Connections;
use futures::channel::oneshot;
use nocterm_connections_model::credential_scope::CredentialScope;
use nocterm_core::persist;
use nocterm_session::{Auth, Target};
use nocterm_store::{ShutdownCoordinator, SnapshotWriter, StoreError};
use std::sync::{Arc, Mutex};

mod publication;
mod shutdown;

fn direct(profile: &Profile) -> nocterm_connections_model::credential_scope::CredentialScope {
    nocterm_connections_model::credential_scope::CredentialScope::direct(
        profile.target.clone(),
        profile.auth.clone(),
    )
}

fn profile(name: &str) -> Profile {
    Profile {
        id: nocterm_connections_model::test_support::profile_id_for(name),
        name: name.into(),
        description: String::new(),
        options: Default::default(),
        target: Target::new("ci", "host", 22),
        auth: Auth::Password,
        credential: None,
        launch: None,
        icon: None,
        icon_color: None,
        country: None,
        group: None,
        jump_hosts: Vec::new(),
    }
}

#[gpui::test]
async fn stale_edit_cannot_resurrect_deleted_profile_or_replace_credential(
    cx: &mut gpui::TestAppContext,
) {
    let original = profile("Build");
    let entity = cx.new(|_| Connections::in_memory());
    entity
        .update(cx, |c, cx| c.save_profile(original.clone(), cx))
        .await
        .unwrap();
    let credential = CredentialId::from_bytes([1; 16]);
    entity
        .update(cx, |c, cx| {
            c.associate_credential(direct(&original), credential, cx)
        })
        .await
        .unwrap();
    let rejected = entity
        .update(cx, |c, cx| {
            c.save_profile_checked(original.clone(), Some(original.clone()), cx)
        })
        .await
        .unwrap_err();
    assert!(matches!(
        rejected,
        ConnectionsError::Profile(nocterm_connections_model::profiles::ProfileError::Conflict(id))
            if id == original.id
    ));
    assert_eq!(
        entity.read_with(cx, |c, cx| c
            .profiles(cx)
            .get(original.id)
            .unwrap()
            .credential),
        Some(credential)
    );
    entity
        .update(cx, |c, cx| c.delete_profile(original.id, cx))
        .await
        .unwrap();
    assert!(
        entity
            .update(cx, |c, cx| c.save_profile_checked(
                original.clone(),
                Some(original.clone()),
                cx
            ))
            .await
            .is_err()
    );
    assert!(entity.read_with(cx, |c, cx| c.profiles(cx).is_empty()));
    let mut oversized = original.clone();
    oversized.description = "x".repeat((64 * 1024) + 1);
    assert!(
        entity
            .update(cx, |c, cx| c.save_profile(oversized, cx))
            .await
            .is_err()
    );
}

#[gpui::test]
async fn group_mutations_apply_atomically_and_unlink_recents(cx: &mut gpui::TestAppContext) {
    let grouped = |name: &str, group: Option<&str>| {
        let mut item = profile(name);
        item.target = Target::new("ci", name, 22);
        item.group = group.map(Into::into);
        item
    };
    let (a, b, c) = (
        grouped("a", Some("Work")),
        grouped("b", Some("Work")),
        grouped("c", Some("Other")),
    );
    let entity = cx.new(|_| Connections::in_memory());
    for item in [&a, &b, &c] {
        entity
            .update(cx, |e, cx| e.save_profile(item.clone(), cx))
            .await
            .unwrap();
    }
    entity.update(cx, |e, cx| {
        e.record_use(recent_use(&a, Some(a.id)), cx);
        e.record_use(recent_use(&c, Some(c.id)), cx);
    });
    entity
        .update(cx, |e, cx| e.rename_group("Work".into(), "Team".into(), cx))
        .await
        .unwrap();
    assert_eq!(
        entity.read_with(cx, |e, cx| e.profiles(cx).get(a.id).unwrap().group.clone()),
        Some("Team".into())
    );
    entity
        .update(cx, |e, cx| e.ungroup("Team".into(), cx))
        .await
        .unwrap();
    entity.read_with(cx, |e, cx| {
        assert_eq!(e.profiles(cx).get(b.id).unwrap().group, None);
        assert_eq!(e.profiles(cx).groups(), ["Other"]);
    });
    reject_invalid_group_actions(&entity, cx).await;

    entity
        .update(cx, |e, cx| e.delete_group("Other".into(), vec![c.id], cx))
        .await
        .unwrap();
    entity.read_with(cx, |e, cx| {
        assert!(e.profiles(cx).get(c.id).is_none());
        assert!(e.profiles(cx).get(a.id).is_some() && e.profiles(cx).get(b.id).is_some());
        assert!(e.profiles(cx).groups().is_empty());
        assert!(
            e.recents(cx)
                .iter()
                .all(|recent| recent.profile != Some(c.id))
        );
        assert!(
            e.recents(cx)
                .iter()
                .any(|recent| recent.profile == Some(a.id))
        );
        assert_eq!(e.recents(cx).iter().count(), 2);
    });
}

#[gpui::test]
async fn delete_group_rejects_members_that_changed_since_they_were_shown(
    cx: &mut gpui::TestAppContext,
) {
    let mut a = profile("a");
    a.group = Some("Work".into());
    let b = profile("b");
    let entity = cx.new(|_| Connections::in_memory());
    for item in [&a, &b] {
        entity
            .update(cx, |e, cx| e.save_profile(item.clone(), cx))
            .await
            .unwrap();
    }
    // `b` is moved into the group after the dialog listed only `a`.
    entity
        .update(cx, |e, cx| e.move_profile(b.id, Some("Work".into()), cx))
        .await
        .unwrap();
    let before = entity.read_with(cx, |e, cx| e.profiles(cx).clone());
    let error = entity
        .update(cx, |e, cx| e.delete_group("Work".into(), vec![a.id], cx))
        .await
        .unwrap_err();
    assert!(error.to_string().contains("changed"), "{error}");
    assert_eq!(entity.read_with(cx, |e, cx| e.profiles(cx).clone()), before);
    entity
        .update(cx, |e, cx| {
            e.delete_group("Work".into(), vec![b.id, a.id], cx)
        })
        .await
        .unwrap();
    assert!(entity.read_with(cx, |e, cx| e.profiles(cx).is_empty()));
}

#[gpui::test]
async fn rename_group_trims_the_new_name(cx: &mut gpui::TestAppContext) {
    let mut a = profile("a");
    a.group = Some("Work".into());
    let mut b = profile("b");
    b.group = Some("Team".into());
    let entity = cx.new(|_| Connections::in_memory());
    for item in [&a, &b] {
        entity
            .update(cx, |e, cx| e.save_profile(item.clone(), cx))
            .await
            .unwrap();
    }
    entity
        .update(cx, |e, cx| {
            e.rename_group("Work".into(), "  Team\t".into(), cx)
        })
        .await
        .unwrap();
    entity.read_with(cx, |e, cx| {
        assert_eq!(e.profiles(cx).groups(), ["Team"]);
        assert_eq!(
            e.profiles(cx).get(a.id).unwrap().group.as_deref(),
            Some("Team")
        );
    });
}

fn documents(items: &[Profile]) -> Profiles {
    let mut profiles = Profiles::default();
    for item in items {
        profiles.upsert(item.clone());
    }
    profiles
}
fn ready_writer<T: Send + 'static>() -> SnapshotWriter<T> {
    Arc::new(|_| Box::pin(async { Ok(()) }))
}

#[gpui::test]
async fn ordered_writes_rebase_failed_predecessors_and_reject_stale_editors(
    cx: &mut gpui::TestAppContext,
) {
    let original = profile("Build");
    let (release, blocked) = oneshot::channel();
    let blocked = Arc::new(Mutex::new(Some(blocked)));
    let writes = Arc::new(Mutex::new(Vec::new()));
    let saved = writes.clone();
    let writer = Arc::new(move |profiles: Profiles| {
        saved.lock().unwrap().push(profiles);
        let blocked = blocked.lock().unwrap().take();
        Box::pin(async move {
            if let Some(blocked) = blocked {
                blocked.await.unwrap();
                return Err(StoreError::PendingWrites);
            }
            Ok(())
        }) as futures::future::BoxFuture<'static, Result<(), StoreError>>
    });
    let entity = cx.new(|_| {
        Connections::with_backends(
            documents(std::slice::from_ref(&original)),
            Recents::default(),
            writer,
            ready_writer(),
        )
    });
    let first = entity.update(cx, |c, cx| {
        c.move_profile(original.id, Some("A".into()), cx)
    });
    cx.run_until_parked();
    assert_eq!(
        entity.read_with(cx, |c, cx| c
            .profiles(cx)
            .get(original.id)
            .unwrap()
            .group
            .clone()),
        None
    );
    let second = entity.update(cx, |c, cx| {
        c.move_profile(original.id, Some("B".into()), cx)
    });
    let stale = entity.update(cx, |c, cx| {
        c.save_profile_checked(original.clone(), Some(original.clone()), cx)
    });
    release.send(()).unwrap();
    assert!(first.await.is_err());
    second.await.unwrap();
    assert!(matches!(
        stale.await,
        Err(ConnectionsError::Profile(
            nocterm_connections_model::profiles::ProfileError::Conflict(_)
        ))
    ));
    assert_eq!(writes.lock().unwrap().len(), 2);
    assert_eq!(
        entity.read_with(cx, |c, cx| c
            .profiles(cx)
            .get(original.id)
            .unwrap()
            .group
            .clone()),
        Some("B".into())
    );
}

#[gpui::test]
async fn save_success_applies_recent_effects_before_receipt_without_a_waiting_caller(
    cx: &mut gpui::TestAppContext,
) {
    let original = profile("Build");
    let entity = cx.new(|_| {
        Connections::with_backends(
            documents(std::slice::from_ref(&original)),
            Recents::default(),
            ready_writer(),
            ready_writer(),
        )
    });
    entity.update(cx, |c, cx| {
        c.record_use(recent_use(&original, Some(original.id)), cx)
    });
    let save = entity.update(cx, |c, cx| c.delete_profile(original.id, cx));
    drop(save);
    cx.run_until_parked();
    assert!(entity.read_with(cx, |c, cx| c.profiles(cx).is_empty()));
    assert!(entity.read_with(cx, |c, cx| {
        c.recents(cx).iter().all(|recent| recent.profile.is_none())
    }));
}

#[gpui::test]
async fn consumed_credential_does_not_overwrite_later_independent_use(
    cx: &mut gpui::TestAppContext,
) {
    let original = profile("Build");
    let entity = cx.new(|_| {
        Connections::with_documents(
            documents(std::slice::from_ref(&original)),
            Recents::default(),
        )
    });
    let assigned = CredentialId::from_bytes([2; 16]);
    entity
        .update(cx, |c, cx| {
            c.associate_credential(direct(&original), assigned, cx)
        })
        .await
        .unwrap();
    let newer = CredentialId::from_bytes([3; 16]);
    let mut spec = recent_use(&original, Some(original.id));
    spec.credential = Some(newer);
    entity.update(cx, |c, cx| c.record_use(spec, cx));
    entity
        .update(cx, |c, cx| {
            c.move_profile(original.id, Some("Work".into()), cx)
        })
        .await
        .unwrap();
    assert_eq!(
        entity.read_with(cx, |c, cx| c.recents(cx).iter().next().unwrap().credential),
        Some(newer)
    );
}

#[gpui::test]
async fn queue_full_and_captured_closing_are_semantic_errors(cx: &mut gpui::TestAppContext) {
    let shutdown = ShutdownCoordinator::default();
    let entity = cx.update(|cx| {
        Connections::with_backends(
            Profiles::default(),
            Recents::default(),
            ready_writer(),
            ready_writer(),
        )
        .into_entity(&shutdown, cx)
        .unwrap()
    });
    let (_, overflow) = entity.update(cx, |c, cx| {
        let accepted = (0..MAX_PENDING_WRITES)
            .map(|_| c.save_profile(profile("Queued"), cx))
            .collect::<Vec<_>>();
        (accepted, c.save_profile(profile("Overflow"), cx))
    });
    assert!(matches!(overflow.await, Err(ConnectionsError::Full)));
    let work = cx.update(|cx| shutdown.capture_all(cx));
    assert!(matches!(
        entity
            .update(cx, |c, cx| c.save_profile(profile("After capture"), cx))
            .await,
        Err(ConnectionsError::Closing)
    ));
    assert!(
        cx.background_executor
            .spawn(work)
            .await
            .failures()
            .is_empty()
    );
}

#[gpui::test]
async fn successful_profile_save_preserves_recents_failure_until_explicit_retry(
    cx: &mut gpui::TestAppContext,
) {
    use std::sync::atomic::{AtomicBool, Ordering};
    let original = profile("Build");
    let fail = Arc::new(AtomicBool::new(true));
    let failed = fail.clone();
    let recents_writer: SnapshotWriter<Recents> = Arc::new(move |_| {
        let fail = failed.load(Ordering::SeqCst);
        Box::pin(async move {
            if fail {
                Err(StoreError::PendingWrites)
            } else {
                Ok(())
            }
        })
    });
    let entity = cx.new(|_| {
        Connections::with_backends(
            documents(std::slice::from_ref(&original)),
            Recents::default(),
            ready_writer(),
            recents_writer,
        )
    });
    entity.update(cx, |c, cx| {
        c.record_use(recent_use(&original, Some(original.id)), cx)
    });
    cx.run_until_parked();
    assert!(entity.read_with(cx, |c, cx| c.persistence_error(cx).is_some()));
    entity
        .update(cx, |c, cx| {
            c.move_profile(original.id, Some("Work".into()), cx)
        })
        .await
        .unwrap();
    assert!(entity.read_with(cx, |c, cx| c.persistence_error(cx).is_some()));
    fail.store(false, Ordering::SeqCst);
    entity.update(cx, |c, cx| {
        c.record_use(recent_use(&original, Some(original.id)), cx)
    });
    cx.run_until_parked();
    assert!(entity.read_with(cx, |c, cx| c.persistence_error(cx).is_none()));
}

async fn reject_invalid_group_actions(entity: &Entity<Connections>, cx: &mut gpui::TestAppContext) {
    entity
        .update(cx, |e, cx| e.rename_group("Work".into(), "X".into(), cx))
        .await
        .unwrap_err();
    entity
        .update(cx, |e, cx| e.ungroup("Work".into(), cx))
        .await
        .unwrap_err();
    entity
        .update(cx, |e, cx| e.delete_group("Work".into(), vec![], cx))
        .await
        .unwrap_err();
    entity
        .update(cx, |e, cx| {
            e.rename_group("Other".into(), String::new(), cx)
        })
        .await
        .unwrap_err();
    entity
        .update(cx, |e, cx| e.rename_group("Other".into(), " \t".into(), cx))
        .await
        .unwrap_err();
    entity
        .update(cx, |e, cx| {
            e.rename_group("Other".into(), "x".repeat((64 * 1024) + 1), cx)
        })
        .await
        .unwrap_err();
}

#[gpui::test]
async fn noop_profiles_do_not_write_and_backend_sources_remain_typed(
    cx: &mut gpui::TestAppContext,
) {
    let original = profile("Original");
    let writes = Arc::new(Mutex::new(0));
    let observed = writes.clone();
    let writer: SnapshotWriter<Profiles> = Arc::new(move |_| {
        *observed.lock().unwrap() += 1;
        Box::pin(async {
            Err(StoreError::backend(persist::PersistError::Io {
                path: "/profiles.toml".into(),
                source: std::io::Error::from(std::io::ErrorKind::StorageFull),
            }))
        })
    });
    let entity = cx.new(|_| {
        Connections::with_backends(
            documents(std::slice::from_ref(&original)),
            shutdown::recent_profiles(std::slice::from_ref(&original)),
            writer,
            ready_writer(),
        )
    });
    entity
        .update(cx, |c, cx| c.save_profile(original.clone(), cx))
        .await
        .unwrap();
    assert_eq!(*writes.lock().unwrap(), 0);
    let result = entity
        .update(cx, |c, cx| c.delete_profile(original.id, cx))
        .await;
    let Err(ConnectionsError::Persistence(StoreError::Backend(source))) = result else {
        panic!("expected typed backend failure");
    };
    assert!(
        matches!(source.downcast_ref::<persist::PersistError>(), Some(persist::PersistError::Io { source, .. }) if source.kind() == std::io::ErrorKind::StorageFull)
    );
    assert_eq!(*writes.lock().unwrap(), 1);
    assert_eq!(
        entity.read_with(cx, |c, cx| c.recents(cx).iter().next().unwrap().profile),
        Some(original.id)
    );
    assert_eq!(
        entity.read_with(cx, |c, cx| c.profiles(cx).get(original.id).cloned()),
        Some(original)
    );
}

#[gpui::test]
async fn deleting_and_readding_the_same_profile_keeps_the_old_recent_unlinked(
    cx: &mut gpui::TestAppContext,
) {
    let original = profile("Original");
    let entity = cx.new(|_| {
        Connections::with_documents(
            documents(std::slice::from_ref(&original)),
            Recents::default(),
        )
    });
    entity.update(cx, |c, cx| {
        c.record_use(recent_use(&original, Some(original.id)), cx)
    });
    entity
        .update(cx, |c, cx| c.delete_profile(original.id, cx))
        .await
        .unwrap();
    entity
        .update(cx, |c, cx| c.save_profile(original.clone(), cx))
        .await
        .unwrap();
    assert_eq!(
        entity.read_with(cx, |c, cx| c.profiles(cx).get(original.id).cloned()),
        Some(original)
    );
    assert_eq!(
        entity.read_with(cx, |c, cx| c.recents(cx).iter().next().unwrap().profile),
        None
    );
}

#[gpui::test]
async fn deleting_a_profile_with_jump_hosts_removes_its_recents(cx: &mut gpui::TestAppContext) {
    let mut routed = profile("Routed");
    routed.target = Target::new("ci", "private", 22);
    routed.jump_hosts = vec![
        nocterm_connections_model::jump_route::JumpHostEntry::Address(Target::new(
            "ops", "bastion", 22,
        )),
    ];
    let direct = profile("Direct");
    let entity = cx.new(|_| {
        Connections::with_documents(
            documents(&[routed.clone(), direct.clone()]),
            Recents::default(),
        )
    });
    entity.update(cx, |c, cx| {
        c.record_use(recent_use(&routed, Some(routed.id)), cx);
        c.record_use(recent_use(&direct, Some(direct.id)), cx);
    });
    for id in [routed.id, direct.id] {
        entity
            .update(cx, |c, cx| c.delete_profile(id, cx))
            .await
            .unwrap();
    }
    let remaining = entity.read_with(cx, |c, cx| {
        c.recents(cx)
            .iter()
            .map(|recent| recent.target.clone())
            .collect::<Vec<_>>()
    });
    assert_eq!(
        remaining,
        [direct.target],
        "only the connection that never used a route stays reachable as a plain destination"
    );
}

#[gpui::test]
async fn remembering_a_password_stays_with_the_route_it_was_typed_for(
    cx: &mut gpui::TestAppContext,
) {
    let via = |name: &str, hop: &str| {
        let mut item = profile(name);
        item.target = Target::new("root", "192.168.122.10", 22);
        item.jump_hosts = vec![
            nocterm_connections_model::jump_route::JumpHostEntry::Address(Target::new(
                "ops", hop, 22,
            )),
        ];
        item
    };
    let (vm_a, vm_b) = (via("vm-a", "hyp-a"), via("vm-b", "hyp-b"));
    let mut plain = profile("plain");
    plain.target = vm_a.target.clone();
    let entity = cx.new(|_| {
        Connections::with_documents(
            documents(&[vm_a.clone(), vm_b.clone(), plain.clone()]),
            Recents::default(),
        )
    });
    entity.update(cx, |c, cx| {
        c.record_use(recent_use(&vm_a, Some(vm_a.id)), cx);
        c.record_use(recent_use(&vm_b, Some(vm_b.id)), cx);
    });
    let credential = CredentialId::from_bytes([9; 16]);
    let scope = CredentialScope::routed(vm_a.id, vm_a.target.clone(), vm_a.auth.clone());
    entity
        .update(cx, |c, cx| c.associate_credential(scope, credential, cx))
        .await
        .unwrap();

    let held = entity.read_with(cx, |c, cx| {
        let profiles = c.profiles(cx);
        let profile_of = |id| profiles.get(id).and_then(|item| item.credential);
        let recent_of = |id| {
            c.recents(cx)
                .iter()
                .find(|recent| recent.profile == Some(id))
                .and_then(|recent| recent.credential)
        };
        (
            profile_of(vm_a.id),
            profile_of(vm_b.id),
            profile_of(plain.id),
            recent_of(vm_a.id),
            recent_of(vm_b.id),
        )
    });
    assert_eq!(
        held,
        (Some(credential), None, None, Some(credential), None),
        "the password for vm-a is not offered to the same address through another route or directly"
    );
}

#[gpui::test]
fn normal_publication_survives_dropping_model_and_receipt_tasks(cx: &mut gpui::TestAppContext) {
    let original = profile("Original");
    let directory = tempfile::tempdir().unwrap();
    let recents_path = directory.path().join("recents.toml");
    let saved_path = recents_path.clone();
    let writer: SnapshotWriter<Recents> = Arc::new(move |recents| {
        let path = saved_path.clone();
        Box::pin(async move { persist::save(&path, &recents).map_err(StoreError::backend) })
    });
    let recents = shutdown::recent_profiles(std::slice::from_ref(&original));
    let coordinator = ShutdownCoordinator::default();
    let entity = cx.update(|cx| {
        Connections::with_backends(
            documents(std::slice::from_ref(&original)),
            recents,
            ready_writer(),
            writer,
        )
        .into_entity(&coordinator, cx)
        .unwrap()
    });
    let deleted = entity.update(cx, |c, cx| c.delete_profile(original.id, cx));
    drop(deleted);
    drop(entity);
    cx.run_until_parked();
    assert_eq!(
        persist::load::<Recents>(&recents_path)
            .unwrap()
            .unwrap()
            .iter()
            .next()
            .unwrap()
            .profile,
        None
    );
}

#[gpui::test]
fn record_use_preserves_the_supplied_timestamp_without_clock_discovery(
    cx: &mut gpui::TestAppContext,
) {
    use nocterm_connections_model::test_support::CountingClock;
    let values = Arc::new(CountingClock::new(42));
    let owner =
        Connections::memory_with_clock(Profiles::default(), Recents::default(), values.clone());
    let entity = cx.new(|_| owner);
    let profile = profile("Timestamp");
    entity.update(cx, |connections, cx| {
        connections.record_use(recent_use(&profile, Some(profile.id)), cx)
    });
    assert_eq!(
        entity.read_with(cx, |connections, cx| connections
            .recents(cx)
            .iter()
            .next()
            .unwrap()
            .last_used),
        42
    );
    assert_eq!(values.calls(), 1);
}

fn recent_use(profile: &Profile, id: Option<ProfileId>) -> RecentUse {
    RecentUse {
        options: profile.options.clone(),
        target: profile.target.clone(),
        auth: profile.auth.clone(),
        profile: id,
        credential: profile.credential,
        launch: profile.launch.clone(),
    }
}
