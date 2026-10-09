use gpui_kit::{AppContext as _, test::TestWindowExt as _};
use nocterm_session::{Auth, Target};

use super::*;
use crate::store::{ProfileId, Recent};

fn profile(name: &str) -> Profile {
    Profile {
        description: String::new(),
        options: Default::default(),
        id: ProfileId::generate(),
        name: name.into(),
        target: Target::new("root", "192.0.2.1", 22),
        auth: Auth::Auto,
        group: None,
        credential: None,
        launch: None,
        icon: None,
        icon_color: None,
        country: None,
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
    let mut recents = Recents::default();
    recents.record(recent(Some(first.id)));
    recents.record(recent(Some(second.id)));
    recents.iter_mut().nth(1).unwrap().profile = Some(second.id);
    assert_eq!(
        recent_profiles(&profiles, &recents).collect::<Vec<_>>(),
        vec![&second]
    );
}

#[gpui_kit::test]
async fn empty_center_reconnect_uses_normal_directory_and_current_profile(
    cx: &mut gpui_kit::TestAppContext,
) {
    let (handle, workspace, opened) = crate::test_support::workspace(cx);
    cx.update_window(handle, |_, window, cx| {
        workspace.update(cx, |workspace, cx| crate::register(workspace, window, cx));
    })
    .unwrap();
    let mut server = profile("Saved name");
    let connections = cx.read(Connections::global);
    connections
        .update(cx, |connections, cx| {
            connections.save_profile(server.clone(), cx)
        })
        .await
        .unwrap();
    connections.update(cx, |connections, cx| {
        connections.record_use(&spec_for_profile(&server), Some(server.id), cx)
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
        let expected = Directory.connections(cx);
        let recent = Directory.recent_connections(cx);
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
    crate::test_support::workspace(cx);
    cx.update(|cx| {
        nocterm_ui::init(
            nocterm_ui::DesignTokens::builtin(),
            nocterm_ui::SettingsStore::in_memory(Default::default()),
            cx,
        );
    });
    let detected = profile("Detected country");
    let mut chosen = profile("Chosen country");
    chosen.country = Some("de".into());
    let connections = cx.read(Connections::global);
    for server in [&detected, &chosen] {
        connections
            .update(cx, |connections, cx| {
                connections.save_profile(server.clone(), cx)
            })
            .await
            .unwrap();
        connections.update(cx, |connections, cx| {
            connections.record_use(&spec_for_profile(server), Some(server.id), cx)
        });
    }
    // Supply persisted observations and a cached flag, so no network lookup is needed.
    let directory = tempfile::tempdir().unwrap();
    let paths = nocterm_core::Paths::rooted_at(directory.path());
    std::fs::create_dir_all(paths.state_dir().join("flags")).unwrap();
    std::fs::write(
        paths.state_dir().join("servers.toml"),
        format!("[server.\"{}\"]\ncountry = \"de\"\n", detected.id),
    )
    .unwrap();
    std::fs::write(
        paths.state_dir().join("flags/de.svg"),
        b"<svg xmlns=\"http://www.w3.org/2000/svg\"/>",
    )
    .unwrap();
    cx.update(|cx| ServerFacts::load(&paths).install(cx));
    cx.run_until_parked();
    for enabled in [true, false, true] {
        cx.update(|cx| {
            nocterm_ui::SettingsExt::update_setting::<nocterm_ui::AppearanceSettings>(
                cx,
                move |appearance| appearance.detect_server_country = enabled,
            )
        })
        .await
        .unwrap();
        cx.update(|cx| {
            let normal = Directory.connections(cx);
            let recent = Directory.recent_connections(cx);
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
