use std::path::PathBuf;

use super::*;

fn profile(name: &str, host: &str, group: Option<&str>) -> Profile {
    Profile {
        description: String::new(),
        options: Default::default(),
        id: crate::test_support::profile_id_for(name),
        name: name.to_owned(),
        target: Target::new("root", host, 22),
        auth: Auth::Auto,
        group: group.map(str::to_owned),
        credential: None,
        launch: None,
        icon: None,
        icon_color: None,
        country: None,
    }
}

fn recent(host: &str, profile: Option<ProfileId>, last_used: u64) -> Recent {
    Recent {
        options: Default::default(),
        target: Target::new("root", host, 22),
        auth: Auth::Auto,
        profile,
        last_used,
        credential: None,
        launch: None,
    }
}

#[test]
fn profiles_read_from_hand_written_toml() {
    let text = r#"
            # Work machines
            [[connection]]
            id = "6f1c1f3e-8f6e-4b8a-9d5e-0a1b2c3d4e5f"
            name = "Build box"
            host = "build.example.com"
            port = 2222
            user = "ci"
            group = "Work"
            auth = { method = "key", path = "/home/me/.ssh/ci" }

            [[connection]]
            id = "0b9f3a51-2d64-4f1e-8c7a-5e6d7c8b9a0f"
            name = "Pi"
            host = "pi.local"
            user = "pi"
        "#;

    let profiles: Profiles = toml::from_str(text).unwrap();
    let all: Vec<&Profile> = profiles.iter().collect();

    assert_eq!(all.len(), 2);
    assert_eq!(all[0].target, Target::new("ci", "build.example.com", 2222));
    assert_eq!(
        all[0].auth,
        Auth::Key {
            path: PathBuf::from("/home/me/.ssh/ci")
        }
    );
    assert_eq!(all[0].group.as_deref(), Some("Work"));
    assert_eq!(all[1].target.port, 22);
    assert_eq!(all[1].auth, Auth::Auto);
}

#[test]
fn profiles_round_trip_and_omit_defaults() {
    let mut profiles = Profiles::default();
    profiles.upsert(profile("Pi", "pi.local", None));

    let text = toml::to_string(&profiles).unwrap();

    assert!(text.contains("[[connection]]"), "{text}");
    assert!(!text.contains("port"), "{text}");
    assert!(!text.contains("auth"), "{text}");
    assert_eq!(toml::from_str::<Profiles>(&text).unwrap(), profiles);
}

#[test]
fn legacy_folders_survive_the_last_profile_moving_out_and_back() {
    let original = profile("Build", "build.local", Some("Work"));
    // Old versions only recorded folder names on profiles.
    let text = format!("[[connection]]\n{}", toml::to_string(&original).unwrap());
    let mut profiles: Profiles = toml::from_str(&text).unwrap();
    assert_eq!(profiles.groups(), ["Work"]);
    let mut moved = original.clone();
    moved.group = Some("Personal".into());
    profiles.upsert(moved);
    let text = toml::to_string(&profiles).unwrap();
    let mut restored: Profiles = toml::from_str(&text).unwrap();
    assert_eq!(restored.groups(), ["Personal", "Work"]);
    assert!(restored.in_group(Some("Work"), "").is_empty());
    restored.upsert(original.clone());
    assert_eq!(restored.get(original.id), Some(&original));
    restored.remove(original.id);
    assert_eq!(restored.groups(), ["Personal", "Work"]);
}

#[test]
fn rename_group_moves_members_and_empty_folders() {
    let mut profiles = Profiles::default();
    let mut build = profile("Build", "build.local", Some("Work"));
    build.description = "ci".into();
    profiles.upsert(build.clone());
    profiles.upsert(profile("Pi", "pi.local", None));
    let gone = profile("Gone", "gone.local", Some("Empty"));
    profiles.upsert(gone.clone());
    profiles.remove(gone.id);

    profiles.rename_group("Work", "Team");
    profiles.rename_group("Empty", "Spare");

    let moved = profiles.get(build.id).unwrap();
    assert_eq!(moved.group.as_deref(), Some("Team"));
    assert_eq!(moved.description, "ci");
    assert_eq!(profiles.groups(), ["Spare", "Team"]);
    let text = toml::to_string(&profiles).unwrap();
    let restored: Profiles = toml::from_str(&text).unwrap();
    assert_eq!(restored.groups(), ["Spare", "Team"]);
    assert!(!restored.has_group("Work"));
}

#[test]
fn rename_group_merges_and_is_case_sensitive() {
    let mut profiles = Profiles::default();
    profiles.upsert(profile("A", "a.local", Some("Work")));
    profiles.upsert(profile("B", "b.local", Some("Personal")));
    profiles.upsert(profile("C", "c.local", Some("work")));

    profiles.rename_group("Work", "Personal");
    assert_eq!(profiles.groups(), ["Personal", "work"]);
    assert_eq!(profiles.in_group(Some("Personal"), "").len(), 2);
    assert_eq!(profiles.in_group(Some("work"), "").len(), 1);
}

#[test]
fn ungroup_files_members_at_top_level_and_forgets_folder() {
    let mut profiles = Profiles::default();
    profiles.upsert(profile("A", "a.local", Some("Work")));
    profiles.upsert(profile("B", "b.local", Some("Other")));
    let mut remembered = profile("C", "c.local", Some("Old"));
    profiles.upsert(remembered.clone());
    remembered.group = None;
    profiles.upsert(remembered);

    profiles.ungroup("Work");
    profiles.ungroup("Old");

    assert_eq!(profiles.in_group(None, "").len(), 2);
    assert_eq!(profiles.groups(), ["Other"]);
}

#[test]
fn remove_group_drops_only_members_and_does_not_resurrect_folder() {
    let mut profiles = Profiles::default();
    profiles.upsert(profile("A", "a.local", Some("Work")));
    profiles.upsert(profile("B", "b.local", Some("Other")));
    profiles.upsert(profile("C", "c.local", None));

    let removed = profiles.remove_group("Work");

    assert_eq!(removed.len(), 1);
    assert_eq!(removed[0].name, "A");
    assert_eq!(profiles.iter().count(), 2);
    assert_eq!(profiles.groups(), ["Other"]);
    assert!(!profiles.has_group("Work"));
}

#[test]
fn group_actions_on_unknown_or_identical_names_change_nothing() {
    let mut profiles = Profiles::default();
    profiles.upsert(profile("A", "a.local", Some("Work")));
    profiles.upsert(profile("B", "b.local", None));
    let before = toml::to_string(&profiles).unwrap();

    profiles.rename_group("Work", "Work");
    profiles.rename_group("Missing", "Missing");
    profiles.ungroup("Missing");
    let removed = profiles.remove_group("Missing");

    assert!(removed.is_empty());
    assert_eq!(toml::to_string(&profiles).unwrap(), before);
}

#[test]
fn credential_references_and_launch_round_trip_without_secret_fields() {
    let id = CredentialId::from_bytes([42; 16]);
    let mut item = profile("Build", "build.local", None);
    item.credential = Some(id);
    item.launch = Some(ShellLaunch {
        program: Some("/bin/bash".into()),
        args: vec!["-l".into()],
        ..ShellLaunch::default()
    });
    let mut profiles = Profiles::default();
    profiles.upsert(item);
    let text = toml::to_string(&profiles).unwrap();
    assert!(text.contains(&id.to_string()));
    for forbidden in ["password", "passphrase", "master", "secret"] {
        assert!(!text.contains(forbidden));
    }
    assert_eq!(toml::from_str::<Profiles>(&text).unwrap(), profiles);
    let mut item = recent("build.local", None, 1);
    item.credential = Some(id);
    item.launch = Some(ShellLaunch::default());
    let mut recents = Recents::default();
    recents.record(item);
    let text = toml::to_string(&recents).unwrap();
    assert!(text.contains(&id.to_string()));
    assert_eq!(toml::from_str::<Recents>(&text).unwrap(), recents);
}

#[test]
fn icon_choices_round_trip_and_old_files_have_none() {
    let mut item = profile("Web", "web.local", None);
    item.icon = Some("ubuntu".into());
    item.icon_color = Some("#E95420".into());
    let mut profiles = Profiles::default();
    profiles.upsert(item);
    let text = toml::to_string(&profiles).unwrap();
    assert!(text.contains("icon = \"ubuntu\""), "{text}");
    assert_eq!(toml::from_str::<Profiles>(&text).unwrap(), profiles);

    let plain = toml::to_string(&profile("Pi", "pi.local", None)).unwrap();
    assert!(!plain.contains("icon"), "{plain}");
}

#[test]
fn upsert_replaces_by_id() {
    let mut profiles = Profiles::default();
    let mut pi = profile("Pi", "pi.local", None);
    profiles.upsert(pi.clone());

    pi.name = "Raspberry".to_owned();
    profiles.upsert(pi.clone());

    assert_eq!(profiles.iter().count(), 1);
    assert_eq!(profiles.get(pi.id).unwrap().name, "Raspberry");
    assert_eq!(profiles.remove(pi.id), Some(pi));
    assert!(profiles.is_empty());
}

#[test]
fn groups_and_filtering() {
    let mut profiles = Profiles::default();
    profiles.upsert(profile("web", "web.example.com", Some("work")));
    profiles.upsert(profile("db", "db.example.com", Some("Work")));
    profiles.upsert(profile("Pi", "pi.local", None));
    profiles.upsert(profile("api", "api.example.com", Some("work")));

    assert_eq!(profiles.groups(), ["Work", "work"]);

    let names = |group, filter| {
        profiles
            .in_group(group, filter)
            .iter()
            .map(|profile| profile.name.clone())
            .collect::<Vec<_>>()
    };
    assert_eq!(
        names(Some("work"), ""),
        ["web", "api"],
        "in the user's order"
    );
    assert_eq!(names(Some("work"), "WEB"), ["web"]);
    assert_eq!(names(None, ""), ["Pi"]);
    assert_eq!(names(None, "example"), Vec::<String>::new());
}

#[test]
fn recents_keep_the_latest_use_of_each_destination() {
    let mut recents = Recents::default();
    let saved =
        crate::test_support::profile_id_for("crates/connections-model/src/profiles/tests.rs:2");

    recents.record(recent("a", None, 1));
    recents.record(recent("b", Some(saved), 2));
    recents.record(recent("a", None, 3));
    // The same profile, even if its destination changed since.
    recents.record(recent("b2", Some(saved), 4));

    let hosts: Vec<&str> = recents.iter().map(|r| r.target.host.as_str()).collect();
    assert_eq!(hosts, ["b2", "a"]);
}

#[test]
fn recents_are_bounded() {
    let mut recents = Recents::default();
    for ix in 0..(MAX_RECENTS as u64 + 3) {
        recents.record(recent(&format!("host{ix}"), None, ix));
    }

    assert_eq!(recents.iter().count(), MAX_RECENTS);
    assert_eq!(recents.iter().next().unwrap().target.host, "host10");
}

#[test]
fn unlinking_a_deleted_profile_keeps_its_destination() {
    let mut recents = Recents::default();
    let gone =
        crate::test_support::profile_id_for("crates/connections-model/src/profiles/tests.rs:3");
    recents.record(recent("a", None, 1));
    recents.record(recent("a", Some(gone), 2));

    recents.unlink(gone);

    let all: Vec<&Recent> = recents.iter().collect();
    assert_eq!(all.len(), 1, "the two entries are now the same destination");
    assert_eq!(all[0].profile, None);
    assert_eq!(all[0].last_used, 2);
}

#[test]
fn recents_round_trip() {
    let mut recents = Recents::default();
    recents.record(recent(
        "a",
        Some(crate::test_support::profile_id_for(
            "crates/connections-model/src/profiles/tests.rs:4",
        )),
        7,
    ));

    let text = toml::to_string(&recents).unwrap();

    assert_eq!(toml::from_str::<Recents>(&text).unwrap(), recents);
}
#[test]
fn placing_reorders_within_and_across_folders() {
    let mut profiles = Profiles::default();
    let a = profile("a", "a.host", None);
    let b = profile("b", "b.host", None);
    let c = profile("c", "c.host", Some("Work"));
    for p in [&a, &b, &c] {
        profiles.upsert(p.clone());
    }
    let names = |profiles: &Profiles, group: Option<&str>| {
        profiles
            .in_group(group, "")
            .iter()
            .map(|p| p.name.clone())
            .collect::<Vec<_>>()
    };
    profiles.place(b.id, None, Some(a.id)).unwrap();
    assert_eq!(names(&profiles, None), ["b", "a"]);
    profiles
        .place(a.id, Some("Work".into()), Some(c.id))
        .unwrap();
    assert_eq!(names(&profiles, None), ["b"]);
    assert_eq!(names(&profiles, Some("Work")), ["a", "c"]);
    profiles.place(a.id, Some("Work".into()), None).unwrap();
    assert_eq!(names(&profiles, Some("Work")), ["c", "a"]);
    profiles.place(b.id, None, Some(b.id)).unwrap();
    assert_eq!(names(&profiles, Some("Work")), ["c", "a"]);
    profiles
        .place(a.id, Some("Work".into()), Some(a.id))
        .unwrap();
    assert_eq!(
        names(&profiles, Some("Work")),
        ["c", "a"],
        "dropped on itself"
    );
    profiles.place(a.id, None, Some(a.id)).unwrap();
    assert_eq!(names(&profiles, None), ["b", "a"]);
    assert!(
        profiles.has_group("Work"),
        "an emptied folder is remembered"
    );
    assert!(
        profiles
            .place(
                crate::test_support::profile_id_for(
                    "crates/connections-model/src/profiles/tests.rs:5"
                ),
                None,
                None
            )
            .is_err()
    );
}
#[test]
fn one_pass_grouped_view_preserves_folders_filters_and_user_order() {
    let mut profiles = Profiles::default();
    let saved = profile("Archived", "old.host", Some("Empty"));
    profiles.upsert(saved.clone());
    profiles.remove(saved.id);
    profiles.upsert(profile("beta", "build.host", Some("Work")));
    profiles.upsert(profile("Alpha", "build.host", Some("Work")));
    profiles.upsert(profile("Standalone", "other.host", None));
    let (top, groups) = profiles.grouped("");
    assert_eq!(
        top.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(),
        ["Standalone"]
    );
    assert_eq!(
        groups
            .iter()
            .map(|(name, _)| name.as_str())
            .collect::<Vec<_>>(),
        ["Empty", "Work"]
    );
    assert!(groups[0].1.is_empty());
    assert_eq!(
        groups[1]
            .1
            .iter()
            .map(|p| p.name.as_str())
            .collect::<Vec<_>>(),
        ["beta", "Alpha"]
    );
    let (top, groups) = profiles.grouped(" BUILD ");
    assert!(top.is_empty());
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].1, profiles.in_group(Some("Work"), "BUILD"));
}

#[test]
fn supplied_ids_round_trip_all_legacy_uuid_spellings_with_typed_parse_causes() {
    use std::error::Error as _;
    let id = ProfileId::from_bytes([42; 16]);
    let canonical = id.to_string();
    let compact = canonical.replace('-', "");
    for text in [
        canonical.clone(),
        canonical.to_uppercase(),
        compact,
        format!("{{{canonical}}}"),
        format!("urn:uuid:{canonical}"),
    ] {
        assert_eq!(text.parse::<ProfileId>().unwrap(), id);
    }
    let error = "invalid id".parse::<ProfileId>().unwrap_err();
    assert!(
        error
            .source()
            .unwrap()
            .downcast_ref::<uuid::Error>()
            .is_some()
    );
}

#[test]
fn profile_encoding_causes_keep_real_codec_error_and_clone_identity() {
    use std::error::Error;
    for native in [
        toml::to_string(&Option::<u8>::None).unwrap_err(),
        toml::to_string(&7_u8).unwrap_err(),
    ] {
        let message = native.to_string();
        let error = ProfileError::encoding(native);
        let clone = error.clone();
        let original = error
            .source()
            .unwrap()
            .source()
            .unwrap()
            .downcast_ref::<toml::ser::Error>()
            .unwrap();
        let shared = clone
            .source()
            .unwrap()
            .source()
            .unwrap()
            .downcast_ref::<toml::ser::Error>()
            .unwrap();
        assert!(std::ptr::eq(original, shared));
        assert_eq!(original.to_string(), message);
        assert_eq!(
            error.to_string(),
            format!("Could not encode connection fields: {message}")
        );
        assert_eq!(error.to_string(), clone.to_string());
        assert!(original.source().is_none());
    }
}

#[cfg(unix)]
#[test]
fn profile_mutation_validation_retains_native_path_encoding_cause() {
    use std::{error::Error, os::unix::ffi::OsStrExt};
    let mut candidate = profile("native path", "host.example", None);
    candidate.options.logging = Some(nocterm_session::LoggingOptions {
        directory: Some(PathBuf::from(std::ffi::OsStr::from_bytes(b"native-\xff"))),
        ..Default::default()
    });
    candidate.options.validate().unwrap();
    let expected = toml::to_string(&candidate).unwrap_err().to_string();
    let mutation = ProfileMutation::Save {
        profile: Box::new(candidate),
        expected: ProfileExpectation::Absent,
    };
    let error = mutation.validate().unwrap_err();
    assert!(matches!(error, ProfileError::Encoding(_)));
    let clone = error.clone();
    let original = error
        .source()
        .unwrap()
        .source()
        .unwrap()
        .downcast_ref::<toml::ser::Error>()
        .unwrap();
    assert_eq!(original.to_string(), expected);
    assert!(std::ptr::eq(
        original,
        clone
            .source()
            .unwrap()
            .source()
            .unwrap()
            .downcast_ref::<toml::ser::Error>()
            .unwrap()
    ));
}

#[test]
fn profile_mutations_keep_exact_size_limits_and_checked_conflicts() {
    const LIMIT: usize = 64 * 1024;
    let mut candidate = profile("size", "host.example", None);
    candidate.description = "x".into();
    let overhead = toml::to_string(&candidate).unwrap().len() - 1;
    candidate.description = "x".repeat(LIMIT - overhead);
    assert_eq!(toml::to_string(&candidate).unwrap().len(), LIMIT);
    let mutation = |profile| ProfileMutation::Save {
        profile: Box::new(profile),
        expected: ProfileExpectation::Absent,
    };
    mutation(candidate.clone()).validate().unwrap();
    candidate.description.push('x');
    assert!(matches!(
        mutation(candidate).validate(),
        Err(ProfileError::ProfileSize)
    ));
    ProfileMutation::RenameGroup {
        from: "a".repeat(LIMIT / 2),
        to: "b".repeat(LIMIT / 2),
    }
    .validate()
    .unwrap();
    assert!(matches!(
        ProfileMutation::RenameGroup {
            from: "a".repeat(LIMIT / 2 + 1),
            to: "b".repeat(LIMIT / 2),
        }
        .validate(),
        Err(ProfileError::MutationSize)
    ));

    let original = profile("checked", "host.example", None);
    let mut current = original.clone();
    current.name = "new name".into();
    let mut profiles = Profiles::default();
    profiles.upsert(current.clone());
    let before = toml::to_string(&profiles).unwrap();
    let stale = ProfileMutation::Save {
        profile: Box::new(original.clone()),
        expected: ProfileExpectation::Snapshot(Box::new(original)),
    };
    stale.validate().unwrap();
    assert!(
        matches!(stale.apply(&mut profiles), Err(ProfileError::Conflict(id)) if id == current.id)
    );
    assert_eq!(profiles.get(current.id), Some(&current));
    assert_eq!(toml::to_string(&profiles).unwrap(), before);
}
