use std::{collections::VecDeque, fs};

use super::*;

#[gpui_kit::test]
async fn moves_preserve_latest_attributes_and_roll_back_failed_storage(
    cx: &mut gpui_kit::TestAppContext,
) {
    let dir = tempfile::tempdir().unwrap();
    let profile = Profile {
        id: ProfileId::generate(),
        name: "Build".into(),
        description: "Retained".into(),
        options: Default::default(),
        target: Target::new("ci", "build", 2222),
        auth: Auth::Password,
        credential: Some(CredentialId::generate().unwrap()),
        launch: Some(Default::default()),
        icon: None,
        icon_color: None,
        country: None,
        group: Some("Work".into()),
    };
    let entity = cx.new(|_| Connections::in_memory());
    entity
        .update(cx, |c, cx| c.save_profile(profile.clone(), cx))
        .await
        .unwrap();
    entity
        .update(cx, |c, cx| {
            c.move_profile(profile.id, Some("Personal".into()), cx)
        })
        .await
        .unwrap();
    let mut expected = profile.clone();
    expected.group = Some("Personal".into());
    assert_eq!(
        entity.read_with(cx, |c, _| c.profiles.get(profile.id).cloned()),
        Some(expected)
    );
    let before = entity.read_with(cx, |c, _| c.profiles.clone());
    entity.update(cx, |c, _| c.profiles_file = Some(dir.path().into()));
    assert!(
        entity
            .update(cx, |c, cx| c.move_profile(profile.id, None, cx))
            .await
            .is_err()
    );
    assert_eq!(entity.read_with(cx, |c, _| c.profiles.clone()), before);
    entity.update(cx, |c, _| {
        c.profiles_file = None;
        c.load_error = Some("Invalid TOML".into());
    });
    assert!(
        entity
            .update(cx, |c, cx| c.move_profile(profile.id, None, cx))
            .await
            .is_err()
    );
    assert_eq!(entity.read_with(cx, |c, _| c.profiles.clone()), before);
    entity
        .update(cx, |c, cx| {
            c.move_profile(profile.id, Some("Personal".into()), cx)
        })
        .await
        .unwrap();
}

fn profile(name: &str) -> Profile {
    Profile {
        id: ProfileId::generate(),
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
    }
}

#[gpui_kit::test]
async fn ordered_writes_publish_after_success_and_reject_queued_stale_editor(
    cx: &mut gpui_kit::TestAppContext,
) {
    use futures::FutureExt as _;
    use std::sync::{Arc, Mutex};
    let original = profile("Build");
    let entity = cx.new(|_| Connections::in_memory());
    entity
        .update(cx, |c, cx| c.save_profile(original.clone(), cx))
        .await
        .unwrap();
    let (first_send, first_receive) = oneshot::channel::<Result<(), String>>();
    let (second_send, second_receive) = oneshot::channel::<Result<(), String>>();
    let gates = Arc::new(Mutex::new(VecDeque::from([first_receive, second_receive])));
    let writes = Arc::new(Mutex::new(Vec::<Profiles>::new()));
    let observed = writes.clone();
    entity.update(cx, |c, _| {
        c.profiles_file = Some("/not-used-by-injected-writer".into());
        c.test_writer = Some(Arc::new(move |profiles| {
            observed.lock().unwrap().push(profiles);
            let gate = gates.lock().unwrap().pop_front().unwrap();
            async { gate.await.unwrap() }.boxed()
        }));
    });
    let first = entity.update(cx, |c, cx| {
        c.move_profile(original.id, Some("A".into()), cx)
    });
    let second = entity.update(cx, |c, cx| {
        c.move_profile(original.id, Some("B".into()), cx)
    });
    let mut edited = original.clone();
    edited.description = "Old draft".into();
    let stale = entity.update(cx, |c, cx| {
        c.save_profile_checked(edited, Some(original.clone()), cx)
    });
    cx.run_until_parked();
    assert_eq!(writes.lock().unwrap().len(), 1);
    assert_eq!(
        entity.read_with(cx, |c, _| c.profiles.get(original.id).cloned()),
        Some(original.clone())
    );
    // A normal UI mutation remains runnable while disk completion is held.
    entity.update(cx, |c, _| {
        c.profile_persistence_error = Some("UI remains responsive".into())
    });
    first_send.send(Ok(())).unwrap();
    cx.run_until_parked();
    assert_eq!(writes.lock().unwrap().len(), 2);
    assert_eq!(
        entity.read_with(cx, |c, _| c
            .profiles
            .get(original.id)
            .unwrap()
            .group
            .clone()),
        Some("A".into())
    );
    second_send.send(Ok(())).unwrap();
    first.await.unwrap();
    second.await.unwrap();
    assert!(stale.await.unwrap_err().contains("changed or was deleted"));
    assert_eq!(
        writes.lock().unwrap().len(),
        2,
        "stale draft never reaches persistence"
    );
    assert_eq!(
        entity.read_with(cx, |c, _| c
            .profiles
            .get(original.id)
            .unwrap()
            .group
            .clone()),
        Some("B".into())
    );
}

#[gpui_kit::test]
async fn stale_edit_cannot_resurrect_deleted_profile_or_replace_credential(
    cx: &mut gpui_kit::TestAppContext,
) {
    let original = profile("Build");
    let entity = cx.new(|_| Connections::in_memory());
    entity
        .update(cx, |c, cx| c.save_profile(original.clone(), cx))
        .await
        .unwrap();
    let credential = CredentialId::generate().unwrap();
    entity
        .update(cx, |c, cx| {
            c.associate_credential(&original.target, &original.auth, credential, cx)
        })
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
    assert_eq!(
        entity.read_with(cx, |c, _| c.profiles.get(original.id).unwrap().credential),
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
    assert!(entity.read_with(cx, |c, _| c.profiles.is_empty()));
    let mut oversized = original.clone();
    oversized.description = "x".repeat(MAX_PROFILE_BYTES + 1);
    assert!(
        entity
            .update(cx, |c, cx| c.save_profile(oversized, cx))
            .await
            .is_err()
    );
}

#[gpui_kit::test]
async fn group_mutations_apply_atomically_and_unlink_recents(cx: &mut gpui_kit::TestAppContext) {
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
        e.record_use(&spec_for_profile(&a), Some(a.id), cx);
        e.record_use(&spec_for_profile(&c), Some(c.id), cx);
    });
    entity
        .update(cx, |e, cx| e.rename_group("Work".into(), "Team".into(), cx))
        .await
        .unwrap();
    assert_eq!(
        entity.read_with(cx, |e, _| e.profiles.get(a.id).unwrap().group.clone()),
        Some("Team".into())
    );
    entity
        .update(cx, |e, cx| e.ungroup("Team".into(), cx))
        .await
        .unwrap();
    entity.read_with(cx, |e, _| {
        assert_eq!(e.profiles.get(b.id).unwrap().group, None);
        assert_eq!(e.profiles.groups(), ["Other"]);
    });
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
            e.rename_group("Other".into(), "x".repeat(MAX_PROFILE_BYTES + 1), cx)
        })
        .await
        .unwrap_err();

    entity
        .update(cx, |e, cx| e.delete_group("Other".into(), vec![c.id], cx))
        .await
        .unwrap();
    entity.read_with(cx, |e, _| {
        assert!(e.profiles.get(c.id).is_none());
        assert!(e.profiles.get(a.id).is_some() && e.profiles.get(b.id).is_some());
        assert!(e.profiles.groups().is_empty());
        assert!(e.recents.iter().all(|recent| recent.profile != Some(c.id)));
        assert!(e.recents.iter().any(|recent| recent.profile == Some(a.id)));
        assert_eq!(e.recents.iter().count(), 2);
    });
}

#[gpui_kit::test]
async fn delete_group_rejects_members_that_changed_since_they_were_shown(
    cx: &mut gpui_kit::TestAppContext,
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
    let before = entity.read_with(cx, |e, _| e.profiles.clone());
    let error = entity
        .update(cx, |e, cx| e.delete_group("Work".into(), vec![a.id], cx))
        .await
        .unwrap_err();
    assert!(error.contains("changed"), "{error}");
    assert_eq!(entity.read_with(cx, |e, _| e.profiles.clone()), before);
    entity
        .update(cx, |e, cx| {
            e.delete_group("Work".into(), vec![b.id, a.id], cx)
        })
        .await
        .unwrap();
    assert!(entity.read_with(cx, |e, _| e.profiles.is_empty()));
}

#[gpui_kit::test]
async fn rename_group_trims_the_new_name(cx: &mut gpui_kit::TestAppContext) {
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
    entity.read_with(cx, |e, _| {
        assert_eq!(e.profiles.groups(), ["Team"]);
        assert_eq!(e.profiles.get(a.id).unwrap().group.as_deref(), Some("Team"));
    });
}

#[gpui_kit::test]
async fn group_mutations_fail_after_load_error_or_write_failure(cx: &mut gpui_kit::TestAppContext) {
    let mut item = profile("a");
    item.group = Some("Work".into());
    let entity = cx.new(|_| Connections::in_memory());
    entity
        .update(cx, |e, cx| e.save_profile(item.clone(), cx))
        .await
        .unwrap();
    let before = entity.read_with(cx, |e, _| e.profiles.clone());
    let dir = tempfile::tempdir().unwrap();
    entity.update(cx, |e, _| e.profiles_file = Some(dir.path().into()));
    entity
        .update(cx, |e, cx| e.delete_group("Work".into(), vec![item.id], cx))
        .await
        .unwrap_err();
    assert_eq!(entity.read_with(cx, |e, _| e.profiles.clone()), before);
    entity.update(cx, |e, _| {
        e.profiles_file = None;
        e.load_error = Some("Invalid TOML".into());
    });
    entity
        .update(cx, |e, cx| e.rename_group("Work".into(), "T".into(), cx))
        .await
        .unwrap_err();
    entity
        .update(cx, |e, cx| e.ungroup("Work".into(), cx))
        .await
        .unwrap_err();
    entity
        .update(cx, |e, cx| e.delete_group("Work".into(), vec![item.id], cx))
        .await
        .unwrap_err();
    assert_eq!(entity.read_with(cx, |e, _| e.profiles.clone()), before);
}

#[gpui_kit::test]
async fn recents_are_scheduled_only_when_a_group_mutation_removes_profiles(
    cx: &mut gpui_kit::TestAppContext,
) {
    let mut a = profile("a");
    a.target = Target::new("ci", "a", 22);
    a.group = Some("Work".into());
    let entity = cx.new(|_| Connections::in_memory());
    entity
        .update(cx, |e, cx| e.save_profile(a.clone(), cx))
        .await
        .unwrap();
    for (name, folder) in [("e1", "Empty1"), ("e2", "Empty2")] {
        let mut empty = profile(name);
        empty.target = Target::new("ci", name, 22);
        empty.group = Some(folder.into());
        entity
            .update(cx, |e, cx| e.save_profile(empty.clone(), cx))
            .await
            .unwrap();
        entity
            .update(cx, |e, cx| e.move_profile(empty.id, None, cx))
            .await
            .unwrap();
    }
    let revision =
        |cx: &mut gpui_kit::TestAppContext| entity.read_with(cx, |e, _| e.recents_revision);
    let base = revision(cx);
    entity
        .update(cx, |e, cx| e.move_profile(a.id, Some("Other".into()), cx))
        .await
        .unwrap();
    entity
        .update(cx, |e, cx| {
            e.rename_group("Other".into(), "Work".into(), cx)
        })
        .await
        .unwrap();
    entity
        .update(cx, |e, cx| e.ungroup("Empty1".into(), cx))
        .await
        .unwrap();
    entity
        .update(cx, |e, cx| e.delete_group("Empty2".into(), vec![], cx))
        .await
        .unwrap();
    assert_eq!(revision(cx), base, "nothing was removed");
    entity
        .update(cx, |e, cx| e.delete_group("Work".into(), vec![a.id], cx))
        .await
        .unwrap();
    assert_eq!(revision(cx), base.wrapping_add(1));
}

#[gpui_kit::test]
async fn queue_admission_and_shutdown_refusal_are_explicit(cx: &mut gpui_kit::TestAppContext) {
    let entity = cx.new(|_| Connections::in_memory());
    let directory = tempfile::tempdir().unwrap();
    let (_pending, rejected) = entity.update(cx, |c, cx| {
        c.profiles_file = Some(directory.path().into());
        let pending = (0..MAX_PENDING_WRITES)
            .map(|_| c.save_profile(profile("Queued"), cx))
            .collect::<Vec<_>>();
        let rejected = c.save_profile(profile("Overflow"), cx);
        (pending, rejected)
    });
    assert!(rejected.await.unwrap_err().contains("queue is full"));
    entity.update(cx, |c, _| c.queue.close());
    assert!(
        entity
            .update(cx, |c, cx| c.save_profile(profile("Shutdown"), cx))
            .await
            .unwrap_err()
            .contains("shutting down")
    );
}

#[gpui_kit::test]
async fn successful_profile_save_preserves_recents_failure_until_recents_retry_succeeds(
    cx: &mut gpui_kit::TestAppContext,
) {
    let directory = tempfile::tempdir().unwrap();
    let blocked_recents = directory.path().join("recents.toml");
    fs::create_dir(&blocked_recents).unwrap();
    let original = profile("Build");
    let entity = cx.new(|_| Connections::in_memory());
    entity
        .update(cx, |c, cx| c.save_profile(original.clone(), cx))
        .await
        .unwrap();
    entity.update(cx, |c, cx| {
        c.profiles_file = Some(directory.path().join("connections.toml"));
        c.recents_file = Some(blocked_recents.clone());
        c.record_use(&spec_for_profile(&original), Some(original.id), cx);
    });
    cx.run_until_parked();
    let failure = entity.read_with(cx, |c, _| {
        assert!(c.recents_written_revision < c.recents_revision);
        c.persistence_error().unwrap().to_owned()
    });
    assert!(failure.contains("Could not save recent connections"));
    entity
        .update(cx, |c, cx| {
            c.move_profile(original.id, Some("Work".into()), cx)
        })
        .await
        .unwrap();
    assert_eq!(
        entity.read_with(cx, |c, _| c.persistence_error().map(str::to_owned)),
        Some(failure)
    );
    entity.read_with(cx, |c, _| {
        assert!(c.recents_written_revision < c.recents_revision)
    });
    fs::remove_dir(&blocked_recents).unwrap();
    entity.update(cx, |c, cx| {
        c.record_use(&spec_for_profile(&original), Some(original.id), cx)
    });
    cx.run_until_parked();
    entity.read_with(cx, |c, _| {
        assert!(c.persistence_error().is_none());
        assert_eq!(c.recents_written_revision, c.recents_revision);
    });
    assert_eq!(
        persist::load::<Recents>(&blocked_recents).unwrap().unwrap(),
        entity.read_with(cx, |c, _| c.recents.clone())
    );
    // The opposite writer must preserve ownership of a profile failure too.
    entity.update(cx, |c, _| c.profiles_file = Some(directory.path().into()));
    assert!(
        entity
            .update(cx, |c, cx| c.move_profile(original.id, None, cx))
            .await
            .is_err()
    );
    let profile_failure = entity.read_with(cx, |c, _| c.persistence_error().unwrap().to_owned());
    entity.update(cx, |c, cx| {
        c.record_use(&spec_for_profile(&original), Some(original.id), cx)
    });
    cx.run_until_parked();
    assert_eq!(
        entity.read_with(cx, |c, _| c.persistence_error().map(str::to_owned)),
        Some(profile_failure)
    );
}

#[test]
fn saved_profile_keeps_description_and_session_overrides_when_opened() {
    let profile = Profile {
        id: ProfileId::generate(),
        name: "Legacy build".into(),
        description: "Windows-1251 machine\nUses a private proxy".into(),
        target: nocterm_session::Target::new("ci", "build.example", 22),
        auth: nocterm_session::Auth::Auto,
        group: None,
        credential: None,
        launch: None,
        icon: None,
        icon_color: None,
        country: None,
        options: nocterm_session::SessionOptions {
            term: Some("screen-256color".into()),
            charset: Some(nocterm_session::Charset::Windows1251),
            proxy: Some(nocterm_session::ProxyConfig::HttpConnect {
                host: "127.0.0.1".into(),
                port: 8080,
            }),
            logging: Some(nocterm_session::LoggingOptions::default()),
        },
    };
    let serialized = toml::to_string(&profile).unwrap();
    let restored: Profile = toml::from_str(&serialized).unwrap();
    assert_eq!(restored.description, profile.description);
    assert_eq!(spec_for_profile(&restored).options, profile.options);
}
#[test]
fn a_missing_file_is_an_empty_list() {
    let dir = tempfile::tempdir().unwrap();

    let (profiles, error) = load_profiles(&dir.path().join("connections.toml"));

    assert!(profiles.is_empty());
    assert_eq!(error, None);
    assert_eq!(
        load_recents(&dir.path().join("recents.toml")),
        Recents::default()
    );
}

#[test]
fn a_broken_file_is_reported_not_discarded() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("connections.toml");
    fs::write(&path, "[[connection]]\nname = 3\n").unwrap();

    let (profiles, error) = load_profiles(&path);

    assert!(profiles.is_empty());
    assert!(error.unwrap().contains("connections.toml"));
}

#[test]
fn broken_recents_start_afresh() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("recents.toml");
    fs::write(&path, "not toml at all [").unwrap();

    assert_eq!(load_recents(&path), Recents::default());
}

#[gpui_kit::test]
async fn quitting_with_queued_writes_does_not_panic(cx: &mut gpui_kit::TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let entity = cx.update(|cx| {
        let mut connections = Connections::in_memory();
        connections.profiles_file = Some(directory.path().join("connections.toml"));
        connections.install(cx);
        Connections::global(cx)
    });
    let _save = entity.update(cx, |c, cx| c.save_profile(profile("Queued"), cx));
    cx.quit();
}
