use gpui_kit::{AppContext as _, test::TestWindowExt as _};
use nocterm_session::{Auth, Target};

use super::*;
use nocterm_connections_model::profiles::{ProfileId, Recent};

fn profile(name: &str) -> Profile {
    Profile {
        description: String::new(),
        options: Default::default(),
        id: nocterm_connections_model::test_support::profile_id_for(name),
        name: name.into(),
        target: Target::new("root", "192.0.2.1", 22),
        auth: Auth::Auto,
        group: None,
        credential: None,
        launch: None,
        icon: None,
        icon_color: None,
        country: None,
        jump_hosts: Vec::new(),
    }
}

fn recent(profile: Option<ProfileId>) -> Recent {
    Recent {
        options: Default::default(),
        target: Target::new("root", "old-address.example", 22),
        auth: Auth::Auto,
        profile,
        last_used: 1,
        credential: None,
        launch: None,
    }
}

#[test]
fn recent_profiles_use_current_saved_metadata_in_mru_order() {
    let mut first = profile("Original name");
    let second = profile("Second");
    let removed = profile("Removed");
    let mut profiles = Profiles::default();
    profiles.upsert(first.clone());
    profiles.upsert(second.clone());
    let mut recents = Recents::default();
    recents.record(recent(Some(first.id)));
    recents.record(recent(Some(second.id)));
    recents.record(recent(None));
    recents.record(recent(Some(removed.id)));

    first.name = "Renamed server".into();
    first.target.host = "current-address.example".into();
    first.icon = Some("ubuntu".into());
    first.country = Some("us".into());
    profiles.upsert(first.clone());
    let result: Vec<_> = recent_profiles(&profiles, &recents).collect();
    assert_eq!(result, vec![&second, &first]);
}

#[test]
fn recent_profiles_deduplicate_ids_from_hand_edited_recents() {
    let first = profile("First");
    let second = profile("Second");
    let mut profiles = Profiles::default();
    profiles.upsert(first.clone());
    profiles.upsert(second.clone());
    let duplicate = recent(Some(second.id));
    let row = toml::to_string(&duplicate).unwrap();
    let recents: Recents =
        toml::from_str(&format!("[[recent]]\n{row}\n[[recent]]\n{row}")).unwrap();
    assert_eq!(
        recent_profiles(&profiles, &recents).collect::<Vec<_>>(),
        vec![&second]
    );
}

#[gpui_kit::test]
async fn empty_center_reconnect_uses_normal_directory_and_current_profile(
    cx: &mut gpui_kit::TestAppContext,
) {
    let (handle, workspace, opened, installed) = crate::test_support::workspace(cx);
    cx.update_window(handle, |_, window, cx| {
        workspace.update(cx, |workspace, cx| {
            crate::register(&installed, workspace, window, cx)
        });
    })
    .unwrap();
    let mut server = profile("Saved name");
    let connections = installed.connections().clone();
    connections
        .update(cx, |connections, cx| {
            connections.save_profile(server.clone(), cx)
        })
        .await
        .unwrap();
    connections.update(cx, |connections, cx| {
        crate::session_intents::record_use(
            connections,
            &spec_for_profile(&server),
            Some(server.id),
            cx,
        )
    });
    server.name = "Current name".into();
    server.target.host = "203.0.113.9".into();
    server.icon = Some("ubuntu".into());
    connections
        .update(cx, |connections, cx| {
            connections.save_profile(server.clone(), cx)
        })
        .await
        .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        let expected = Directory::new(installed.clone()).connections(cx);
        let recent = Directory::new(installed.clone()).recent_connections(cx);
        assert_eq!(recent.len(), 1);
        assert_eq!(recent[0].id, expected[0].id);
        assert_eq!(recent[0].name, "Current name");
        assert_eq!(recent[0].target.host, "203.0.113.9");
        assert!(recent[0].icon.is_some());
        window.render_frame(cx);
        let row = gpui_kit::SharedString::from(format!("empty-recent-{}", server.id));
        assert_eq!(window.find(row.clone()).label(), Some("Current name"));
        window.click(row, cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(&*opened.borrow(), &[spec_for_profile(&server)]);
}

#[gpui_kit::test]
async fn recent_flags_follow_detection_setting_and_keep_manually_chosen_country(
    cx: &mut gpui_kit::TestAppContext,
) {
    let (_, _, _, installed) = crate::test_support::workspace(cx);
    let detected = profile("Detected country");
    let mut chosen = profile("Chosen country");
    chosen.country = Some("de".into());
    let connections = installed.connections().clone();
    for server in [&detected, &chosen] {
        connections
            .update(cx, |connections, cx| {
                connections.save_profile(server.clone(), cx)
            })
            .await
            .unwrap();
        connections.update(cx, |connections, cx| {
            crate::session_intents::record_use(
                connections,
                &spec_for_profile(server),
                Some(server.id),
                cx,
            )
        });
    }
    // Supply persisted observations and an injected flag, without native flag I/O.
    let known: nocterm_connections_model::facts::Known =
        toml::from_str(&format!("[server.\"{}\"]\ncountry = \"de\"\n", detected.id)).unwrap();
    let observations = std::sync::Arc::new(
        nocterm_connections_model::test_support::ScriptedObservations::default(),
    );
    observations.flag_next(Ok(Some(
        b"<svg xmlns=\"http://www.w3.org/2000/svg\"/>".to_vec(),
    )));
    let observations = crate::ObservationDeps::new(
        observations.clone(),
        observations.clone(),
        observations,
        false,
        std::sync::Arc::new(nocterm_connections_model::os_observation::OfflineOsDetector),
    );
    let installed = cx.update(|cx| {
        let profiles = connections.read(cx).profiles(cx).clone();
        let recents = connections.read(cx).recents(cx).clone();
        crate::test_support::install_owners(
            crate::Connections::with_documents(profiles, recents),
            crate::FactsPrepared::new(
                known,
                std::sync::Arc::new(|_| Box::pin(async { Ok(()) })),
                observations,
            ),
            installed.settings().clone(),
            cx,
        )
    });
    cx.run_until_parked();
    for enabled in [true, false, true] {
        cx.update(|cx| {
            installed
                .settings()
                .update::<nocterm_ui::AppearanceSettings>(cx, move |appearance| {
                    appearance.detect_server_country = enabled
                })
        })
        .await
        .unwrap();
        cx.update(|cx| {
            let normal = Directory::new(installed.clone()).connections(cx);
            let recent = Directory::new(installed.clone()).recent_connections(cx);
            assert_eq!(recent.len(), 2);
            for server in &recent {
                let expected = normal.iter().find(|entry| entry.id == server.id).unwrap();
                assert_eq!(server.flag.is_some(), expected.flag.is_some());
                assert_eq!(
                    server.flag.is_some(),
                    server.id == chosen.id.to_string() || enabled
                );
                if let (Some(actual), Some(expected)) = (&server.flag, &expected.flag) {
                    assert!(std::sync::Arc::ptr_eq(actual, expected));
                }
            }
        });
    }
}

#[gpui_kit::test]
async fn distinct_installations_keep_profiles_facts_and_credential_association_isolated(
    cx: &mut gpui_kit::TestAppContext,
) {
    let (_, _, _, first) = crate::test_support::workspace(cx);
    let (_, _, _, second) = crate::test_support::workspace(cx);
    let first_profile = profile("First installation");
    let second_profile = profile("Second installation");
    for (installed, server) in [(&first, &first_profile), (&second, &second_profile)] {
        installed
            .connections()
            .update(cx, |connections, cx| {
                connections.save_profile(server.clone(), cx)
            })
            .await
            .unwrap();
    }
    let credential_owner = first.connections().clone();
    let credential = nocterm_session::CredentialId::from_bytes([7; 16]);
    credential_owner
        .update(cx, |connections, cx| {
            connections.associate_credential(
                nocterm_connections_model::credential_scope::CredentialScope::direct(
                    first_profile.target.clone(),
                    first_profile.auth.clone(),
                ),
                credential,
                cx,
            )
        })
        .await
        .unwrap();
    cx.read(|cx| {
        let left = Directory::new(first.clone()).connections(cx);
        let right = Directory::new(second.clone()).connections(cx);
        assert_eq!(
            left.iter().map(|row| row.name.as_ref()).collect::<Vec<_>>(),
            ["First installation"]
        );
        assert_eq!(
            right
                .iter()
                .map(|row| row.name.as_ref())
                .collect::<Vec<_>>(),
            ["Second installation"]
        );
        assert_eq!(
            credential_owner
                .read(cx)
                .profiles(cx)
                .get(first_profile.id)
                .unwrap()
                .credential,
            Some(credential)
        );
        assert_eq!(
            second
                .connections()
                .read(cx)
                .profiles(cx)
                .get(second_profile.id)
                .unwrap()
                .credential,
            None
        );
    });
    cx.update(|cx| {
        first.facts().update(cx, |facts, cx| {
            facts
                .request_flag(
                    nocterm_connections_model::observation::CountryCode::new("de").unwrap(),
                    cx,
                )
                .unwrap()
        })
    });
    cx.run_until_parked();
    assert!(cx.read(|cx| first.flag("de", cx).is_some()));
    assert!(cx.read(|cx| second.flag("de", cx).is_none()));
}

#[gpui_kit::test]
async fn a_second_workspace_keeps_shared_installation_after_first_window_closes(
    cx: &mut gpui_kit::TestAppContext,
) {
    let (first_window, first_workspace, _, installed) = crate::test_support::workspace(cx);
    let (second_window, second_workspace, _, _other_installation) =
        crate::test_support::workspace(cx);
    for (handle, workspace) in [
        (first_window, &first_workspace),
        (second_window, &second_workspace),
    ] {
        cx.update_window(handle, |_, window, cx| {
            workspace.update(cx, |workspace, cx| {
                crate::register(&installed, workspace, window, cx)
            })
        })
        .unwrap();
    }
    let first_weak = first_workspace.downgrade();
    drop(first_workspace);
    cx.update_window(first_window, |_, window, _| window.remove_window())
        .unwrap();
    cx.run_until_parked();
    assert!(first_weak.upgrade().is_none());
    let server = profile("Surviving installation");
    installed
        .connections()
        .update(cx, |connections, cx| {
            connections.save_profile(server.clone(), cx)
        })
        .await
        .unwrap();
    let directory =
        second_workspace.read_with(cx, |workspace, _| workspace.connection_directory().unwrap());
    cx.read(|cx| {
        assert_eq!(directory.connections(cx).len(), 1);
        assert_eq!(directory.connections(cx)[0].name, "Surviving installation");
    });
    cx.update_window(second_window, |_, window, cx| window.render_frame(cx))
        .unwrap();
}

#[gpui_kit::test]
async fn a_directory_uses_its_retained_settings_after_another_installation_is_created(
    cx: &mut gpui_kit::TestAppContext,
) {
    let (_, _, _, first) = crate::test_support::workspace(cx);
    let (_, _, _, second) = crate::test_support::workspace(cx);
    let changed = cx.update(|cx| {
        first
            .settings()
            .update::<nocterm_ui::AppearanceSettings>(cx, |appearance| {
                appearance.detect_server_country = false
            })
    });
    changed.await.unwrap();
    cx.read(|cx| {
        assert!(!first.detecting_country(cx));
        assert!(second.detecting_country(cx));
    });
}

#[gpui_kit::test]
async fn directory_background_attempt_does_not_record_foreground_recent_use(
    cx: &mut gpui_kit::TestAppContext,
) {
    let (handle, workspace, opened, installed) = crate::test_support::workspace(cx);
    let server = profile("Background route");
    installed
        .connections()
        .update(cx, |connections, cx| {
            connections.save_profile(server.clone(), cx)
        })
        .await
        .unwrap();
    let directory = Directory::new(installed.clone());
    cx.update_window(handle, |_, window, cx| {
        // This fixture overrides foreground opening only. The background factory
        // returns None, but the directory must still never record this attempt.
        assert!(
            directory
                .open_background(&server.id.to_string(), &workspace.downgrade(), window, cx)
                .is_none()
        );
        assert_eq!(
            installed.connections().read(cx).recents(cx).iter().count(),
            0
        );
        assert!(directory.open(&server.id.to_string(), &workspace.downgrade(), window, cx));
        assert_eq!(
            installed.connections().read(cx).recents(cx).iter().count(),
            1
        );
    })
    .unwrap();
    assert_eq!(opened.borrow().len(), 1);
}
