use super::*;
use crate::Connections;
use gpui_kit::px;
use gpui_kit::{TestAppContext, base::actions::Confirm, test::TestWindowExt as _};
use nocterm_session::Auth;
use nocterm_session::Target;
use std::path::{Path, PathBuf};

#[gpui_kit::test]
fn dialog_focus_accepts_typing_and_enter_opens_exactly_once(cx: &mut TestAppContext) {
    let (handle, workspace, opened, installed) = crate::test_support::workspace(cx);
    cx.update_window(handle, |_, window, cx| {
        open_editor(installed.clone(), None, workspace.downgrade(), window, cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.input("example.test", cx);
        window.press("enter", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("dialog").is_none());
        assert_eq!(
            installed.connections().read(cx).profiles(cx).iter().count(),
            1
        );
        assert_eq!(
            opened.borrow().len(),
            1,
            "one Enter must open exactly one session"
        );
        assert_eq!(opened.borrow()[0].target.host, "example.test");
    })
    .unwrap();
}

#[gpui_kit::test]
fn dialog_confirm_keeps_invalid_form_open_then_saves_valid_form(cx: &mut TestAppContext) {
    let (handle, workspace, opened, installed) = crate::test_support::workspace(cx);
    cx.update_window(handle, |_, window, cx| {
        open_editor(installed.clone(), None, workspace.downgrade(), window, cx);
        window.render_frame(cx);
        window.dispatch_action(Box::new(Confirm { secondary: false }), cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("dialog").is_some());
        assert!(opened.borrow().is_empty());
        assert_eq!(
            installed.connections().read(cx).profiles(cx).iter().count(),
            0
        );
        window.input("valid.test", cx);
        window.dispatch_action(Box::new(Confirm { secondary: false }), cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("dialog").is_none());
        assert_eq!(
            installed.connections().read(cx).profiles(cx).iter().count(),
            1
        );
        assert_eq!(opened.borrow().len(), 1);
    })
    .unwrap();
}

// Drive the native controls rather than reading or mutating editor fields.
#[gpui_kit::test]
fn sections_keep_drafts_and_multiline_description_enter_does_not_submit(cx: &mut TestAppContext) {
    let (handle, workspace, opened, installed) = crate::test_support::workspace(cx);
    cx.update_window(handle, |_, window, cx| {
        open_editor(installed.clone(), None, workspace.downgrade(), window, cx);
        window.render_frame(cx);
        window.input("draft.test", cx);
        window.scroll(
            "editor-section-content",
            gpui_kit::ScrollDelta::Pixels(gpui_kit::point(px(0.), px(-800.))),
            cx,
        );
        window.render_frame(cx);
        window.click("editor-description", cx);
        window.input("first line", cx);
        window.press("enter", cx);
        window.input("second line", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("dialog").is_some());
        assert!(opened.borrow().is_empty());
        assert!(window.find("editor-description").bounds().size.height >= px(80.));
        window.click("editor-section-authentication", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("auth-password", cx);
        window.click("editor-section-launch", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("profile-launch-toggle", cx);
        window.render_frame(cx);
        window.click("editor-field-Remote executable", cx);
        window.input("/bin/bash", cx);
        window.click("editor-section-connection", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("editor-save", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("dialog").is_none());
        let connections = installed.connections();
        let profile = connections.read(cx).profiles(cx).iter().next().unwrap();
        assert_eq!(profile.target.host, "draft.test");
        assert_eq!(profile.description, "first line\nsecond line");
        assert_eq!(profile.auth, Auth::Password);
        assert_eq!(
            profile.launch.as_ref().unwrap().program.as_deref(),
            Some("/bin/bash")
        );
        assert!(opened.borrow().is_empty());
    })
    .unwrap();
}

#[gpui_kit::test]
fn hidden_missing_key_opens_authentication_and_focuses_native_input(cx: &mut TestAppContext) {
    let (handle, workspace, _, installed) = crate::test_support::workspace(cx);
    cx.simulate_window_resize(handle, gpui_kit::size(px(640.), px(560.)));
    cx.update_window(handle, |_, window, cx| {
        open_editor(installed.clone(), None, workspace.downgrade(), window, cx);
        window.render_frame(cx);
        window.input("valid.test", cx);
        window.click("editor-section-authentication", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("auth-key", cx);
        window.click("editor-section-connection", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("editor-save", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("editor-field-Private key").is_some());
        assert_eq!(window.focused_input(cx).unwrap().value(cx).as_ref(), "");
        window.input("/tmp/key", cx);
        window.click("editor-save", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("dialog").is_none());
        let connections = installed.connections();
        assert_eq!(
            connections
                .read(cx)
                .profiles(cx)
                .iter()
                .next()
                .unwrap()
                .auth,
            Auth::Key {
                path: PathBuf::from("/tmp/key")
            }
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn minimum_window_keeps_footer_in_view_for_every_section(cx: &mut TestAppContext) {
    let (handle, workspace, _, installed) = crate::test_support::workspace(cx);
    cx.simulate_window_resize(handle, gpui_kit::size(px(640.), px(400.)));
    cx.update_window(handle, |_, window, cx| {
        open_editor(installed.clone(), None, workspace.downgrade(), window, cx)
    })
    .unwrap();
    for (_, id, _) in EditorSection::ALL {
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.click(id, cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert_footer_in_view(window);
        })
        .unwrap();
    }
    cx.update_window(handle, |_, window, cx| {
        window.click("editor-cancel", cx);
        window.render_frame(cx);
        assert!(window.try_find("dialog").is_none());
    })
    .unwrap();
}

fn assert_footer_in_view(window: &gpui_kit::Window) {
    for button in ["editor-save", "editor-save-connect", "editor-cancel"] {
        let bounds = window.find(button).bounds();
        assert!(bounds.top() >= px(0.) && bounds.bottom() <= window.viewport_size().height);
        assert!(bounds.left() >= px(0.) && bounds.right() <= window.viewport_size().width);
    }
}

fn conflicting_profiles() -> (Profile, Profile) {
    let original = build_profile(
        nocterm_connections_model::test_support::profile_id_for(
            "crates/connections/src/editor/tests.rs:1",
        ),
        &fields("old.test"),
        AuthKind::Auto,
        Some("tester"),
        None,
    )
    .unwrap();
    let mut latest = original.clone();
    latest.name = "Current saved connection".into();
    latest.target.host = "latest.test".into();
    latest.description = "Notes changed in another view".into();
    latest.auth = Auth::Password;
    (original, latest)
}

fn reload_conflict_then_finish(cx: &mut TestAppContext, save: bool) {
    use futures::FutureExt as _;
    let (handle, workspace, opened, installed) = crate::test_support::workspace(cx);
    cx.simulate_window_resize(handle, gpui_kit::size(px(640.), px(400.)));
    let (original, mut latest) = conflicting_profiles();
    cx.update_window(handle, |_, window, cx| {
        installed.connections().update(cx, |connections, cx| {
            connections
                .save_profile(original.clone(), cx)
                .now_or_never()
                .unwrap()
                .unwrap();
        });
        open_editor(
            installed.clone(),
            Some(original.clone()),
            workspace.downgrade(),
            window,
            cx,
        );
        window.render_frame(cx);
        window.press("ctrl-a", cx);
        window.input("Unsaved stale draft", cx);
        installed.connections().update(cx, |connections, cx| {
            connections
                .save_profile(latest.clone(), cx)
                .now_or_never()
                .unwrap()
                .unwrap();
        });
        window.click("editor-save", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(
            window.focused_input(cx).unwrap().value(cx).as_ref(),
            "Unsaved stale draft"
        );
        assert_footer_in_view(window);
        window.click("editor-reload-current", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(
            window.focused_input(cx).unwrap().value(cx).as_ref(),
            latest.name
        );
        if save {
            window.press("ctrl-a", cx);
            window.input("Saved after reload", cx);
            window.click("editor-save", cx);
        } else {
            window.click("editor-cancel", cx);
        }
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("dialog").is_none());
        let connections = installed.connections();
        let stored = connections.read(cx).profiles(cx).get(original.id).unwrap();
        if save {
            latest.name = "Saved after reload".into();
        }
        assert_eq!(stored, &latest);
        assert!(opened.borrow().is_empty());
    })
    .unwrap();
}
#[gpui_kit::test]
fn conflict_reload_then_cancel_closes_the_old_editor(cx: &mut TestAppContext) {
    reload_conflict_then_finish(cx, false);
}
#[gpui_kit::test]
fn conflict_reload_then_save_uses_latest_revision_and_leaves_no_old_editor(
    cx: &mut TestAppContext,
) {
    reload_conflict_then_finish(cx, true);
}

#[gpui_kit::test]
fn hidden_invalid_session_options_open_the_session_section(cx: &mut TestAppContext) {
    let (handle, workspace, _, installed) = crate::test_support::workspace(cx);
    let mut profile = build(&fields("valid.test"), AuthKind::Auto).unwrap();
    profile.options.term = Some("invalid TERM".into());
    cx.update_window(handle, |_, window, cx| {
        open_editor(
            installed.clone(),
            Some(profile),
            workspace.downgrade(),
            window,
            cx,
        );
        window.render_frame(cx);
        window.click("editor-save", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("session-term").is_some());
        assert!(window.try_find("dialog").is_some());
        assert_eq!(
            installed.connections().read(cx).profiles(cx).iter().count(),
            0
        );
    })
    .unwrap();
}
#[gpui_kit::test]
fn hidden_invalid_credential_focuses_credential_and_keeps_draft(cx: &mut TestAppContext) {
    let (handle, workspace, _, installed) = crate::test_support::workspace(cx);
    cx.update_window(handle, |_, window, cx| {
        open_editor(installed.clone(), None, workspace.downgrade(), window, cx);
        window.render_frame(cx);
        window.input("valid.test", cx);
        window.click("editor-section-authentication", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.input("not-an-id", cx);
        window.click("editor-section-connection", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("editor-save", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("editor-field-Credential ID").is_some());
        assert_eq!(
            window.focused_input(cx).unwrap().value(cx).as_ref(),
            "not-an-id"
        );
        assert_eq!(
            installed.connections().read(cx).profiles(cx).iter().count(),
            0
        );
        window.press("ctrl-a", cx);
        window.press("backspace", cx);
        window.click("editor-save", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("dialog").is_none());
        let connections = installed.connections();
        assert_eq!(
            connections
                .read(cx)
                .profiles(cx)
                .iter()
                .next()
                .unwrap()
                .target
                .host,
            "valid.test"
        );
    })
    .unwrap();
}

fn fields(host: &str) -> ProfileDraft {
    ProfileDraft {
        endpoint: nocterm_connections_model::draft::EndpointDraft {
            host: host.to_owned(),
            ..Default::default()
        },
        ..Default::default()
    }
}
fn build(fields: &ProfileDraft, auth: AuthKind) -> Result<Profile, String> {
    build_profile(
        nocterm_connections_model::test_support::profile_id_for(
            "crates/connections/src/editor/tests.rs:2",
        ),
        fields,
        auth,
        Some("me"),
        Some(Path::new("/home/me")),
    )
    .map_err(|error| error.message)
}
#[test]
fn icon_and_colour_are_optional_and_colours_are_normalised() {
    let plain = build(&fields("host"), AuthKind::Auto).unwrap();
    assert_eq!((plain.icon, plain.icon_color), (None, None));
    let mut draft = fields("host");
    draft.appearance.icon = "debian".into();
    draft.appearance.icon_color = " e95420 ".into();
    let chosen = build(&draft, AuthKind::Auto).unwrap();
    assert_eq!(chosen.icon.as_deref(), Some("debian"));
    assert_eq!(chosen.icon_color.as_deref(), Some("#E95420"));
    draft.appearance.icon_color = "orange".into();
    assert!(
        build(&draft, AuthKind::Auto)
            .unwrap_err()
            .contains("orange")
    );
}
#[test]
fn countries_are_two_letter_codes_stored_in_lower_case() {
    let mut draft = fields("host");
    assert_eq!(build(&draft, AuthKind::Auto).unwrap().country, None);
    draft.appearance.country = " De ".into();
    assert_eq!(
        build(&draft, AuthKind::Auto).unwrap().country.as_deref(),
        Some("de")
    );
    for invalid in ["DEU", "1x", "../"] {
        draft.appearance.country = invalid.into();
        assert!(
            build(&draft, AuthKind::Auto)
                .unwrap_err()
                .contains("country code")
        );
    }
}
#[test]
fn a_host_alone_is_enough() {
    let profile = build(&fields(" example.com "), AuthKind::Auto).unwrap();
    assert_eq!(profile.name, "example.com");
    assert_eq!(profile.target, Target::new("me", "example.com", 22));
    assert_eq!(profile.auth, Auth::Auto);
    assert_eq!(profile.group, None);
}
#[test]
fn every_field_is_used() {
    let mut draft = fields("build.local");
    draft.metadata.name = "Build".into();
    draft.metadata.group = " Work ".into();
    draft.endpoint.port = "2222".into();
    draft.endpoint.user = "ci".into();
    draft.authentication.key_path = "~/.ssh/ci".into();
    let profile = build(&draft, AuthKind::Key).unwrap();
    assert_eq!(profile.name, "Build");
    assert_eq!(profile.target, Target::new("ci", "build.local", 2222));
    assert_eq!(
        profile.auth,
        Auth::Key {
            path: PathBuf::from("/home/me/.ssh/ci")
        }
    );
    assert_eq!(profile.group.as_deref(), Some("Work"));
}
#[test]
fn launch_override_and_credential_id_validate_and_preserve_arguments() {
    let mut draft = fields("example.com");
    assert!(build(&draft, AuthKind::Auto).unwrap().launch.is_none());
    let id = nocterm_session::CredentialId::from_bytes([42; 16]);
    draft.authentication.credential = id.to_string();
    draft.launch.override_launch = true;
    draft.launch.program = "/bin/bash".into();
    draft.launch.args = r#"["-c","printf '%s' \"a b\""]"#.into();
    draft.launch.env = r#"{"LANG":"C"}"#.into();
    draft.launch.cwd = "/tmp/a b".into();
    draft.launch.integration = false;
    let profile = build(&draft, AuthKind::Auto).unwrap();
    assert_eq!(profile.credential, Some(id));
    let launch = profile.launch.unwrap();
    assert_eq!(launch.args, vec!["-c", "printf '%s' \"a b\""]);
    assert!(!launch.integration);
    draft.launch.env = r#"{"INVALID;KEY":"value"}"#.into();
    assert!(build(&draft, AuthKind::Auto).is_err());
    draft.launch.override_launch = false;
    draft.authentication.credential = "plain password".into();
    assert!(build(&draft, AuthKind::Auto).is_err());
}
#[test]
fn starting_directory_sets_launch_cwd_without_override() {
    let mut draft = fields("example.com");
    draft.launch.cwd = "/var/www".into();
    let profile = build(&draft, AuthKind::Auto).unwrap();
    assert_eq!(
        profile.launch.as_ref().and_then(|l| l.cwd.as_deref()),
        Some("/var/www")
    );
    assert_eq!(profile.launch.as_ref().unwrap().program, None);
}
#[test]
fn mistakes_are_explained() {
    assert!(
        build(&fields(""), AuthKind::Auto)
            .unwrap_err()
            .contains("host")
    );
    assert!(build(&fields("me@host"), AuthKind::Auto).is_err());
    let mut draft = fields("host");
    draft.endpoint.port = "ssh".into();
    assert!(build(&draft, AuthKind::Auto).unwrap_err().contains("port"));
    assert!(
        build(&fields("host"), AuthKind::Key)
            .unwrap_err()
            .contains("key")
    );
    assert!(
        build_profile(
            nocterm_connections_model::test_support::profile_id_for(
                "crates/connections/src/editor/tests.rs:3"
            ),
            &fields("host"),
            AuthKind::Auto,
            None,
            None
        )
        .unwrap_err()
        .message
        .contains("user")
    );
}

#[gpui_kit::test]
fn new_connection_uses_supplied_user_and_identifier_source(cx: &mut TestAppContext) {
    use nocterm_connections_model::draft::EditorDefaults;
    let (handle, workspace, opened, installed) = crate::test_support::workspace(cx);
    let inputs = crate::PreparedConnections::memory(EditorDefaults {
        user: Some("supplied-user".into()),
        home: None,
    });
    let installed = cx.update(|cx| {
        let (connections, inputs, facts) = inputs.into_owners();
        crate::Installed::from_owners(
            connections,
            facts,
            inputs,
            installed.settings().clone(),
            &nocterm_store::ShutdownCoordinator::default(),
            cx,
        )
        .unwrap()
    });
    let connections = installed.connections().clone();
    cx.update_window(handle, |_, window, cx| {
        open_editor(installed.clone(), None, workspace.downgrade(), window, cx);
        window.render_frame(cx);
        window.input("supplied.test", cx);
        window.press("enter", cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(opened.borrow()[0].target.user, "supplied-user");
    let mut bytes = [0; 16];
    bytes[15] = 1;
    assert_eq!(
        connections.read_with(cx, |connections, cx| connections
            .profiles(cx)
            .iter()
            .next()
            .unwrap()
            .id),
        nocterm_connections_model::profiles::ProfileId::from_bytes(bytes)
    );
}

#[gpui_kit::test]
fn creating_a_profile_preserves_seeded_profiles_with_memory_or_writer_backends(
    cx: &mut TestAppContext,
) {
    for writer_backed in [false, true] {
        let (handle, workspace, _, installed) = crate::test_support::workspace(cx);
        let mut bytes = [0; 16];
        bytes[15] = 1;
        let saved = build_profile(
            nocterm_connections_model::profiles::ProfileId::from_bytes(bytes),
            &fields("saved.test"),
            AuthKind::Auto,
            Some("tester"),
            None,
        )
        .unwrap();
        let mut profiles = nocterm_connections_model::profiles::Profiles::default();
        profiles.upsert(saved.clone());
        let installed = cx.update(|cx| {
            let owner = if writer_backed {
                Connections::with_backends(
                    profiles,
                    Default::default(),
                    std::sync::Arc::new(|_| Box::pin(async { Ok(()) })),
                    std::sync::Arc::new(|_| Box::pin(async { Ok(()) })),
                )
            } else {
                Connections::with_documents(profiles, Default::default())
            };
            crate::test_support::install_model(owner, &installed, cx)
        });
        let connections = installed.connections().clone();
        cx.update_window(handle, |_, window, cx| {
            open_editor(installed.clone(), None, workspace.downgrade(), window, cx);
            window.render_frame(cx);
            window.input("new.test", cx);
            window.click("editor-field-User", cx);
            window.input("tester", cx);
            window.click("editor-save", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert!(
                window.try_find("dialog").is_none(),
                "a new connection must not conflict with an existing identifier"
            );
        })
        .unwrap();
        connections.read_with(cx, |connections, cx| {
            let profiles = connections.profiles(cx);
            assert_eq!(profiles.iter().count(), 2);
            assert_eq!(profiles.get(saved.id), Some(&saved));
            assert!(
                profiles
                    .iter()
                    .any(|profile| profile.target.host == "new.test" && profile.id != saved.id)
            );
        });
    }
}

#[gpui_kit::test]
fn unsaved_country_input_loads_flag_when_detection_is_off_and_no_session_exists(
    cx: &mut TestAppContext,
) {
    cx.executor().allow_parking();
    use nocterm_connections_model::{
        observation::CountryCode,
        test_support::{ObservationCall, ScriptedObservations},
    };
    use std::{
        sync::Arc,
        time::{Duration, Instant},
    };
    let (window, workspace, _, original) = crate::test_support::workspace(cx);
    let script = Arc::new(ScriptedObservations::default());
    script.flag_next(Ok(Some(b"<svg/>".to_vec())));
    let receipt = cx.update(|cx| {
        original
            .settings()
            .update::<nocterm_ui::AppearanceSettings>(cx, |settings| {
                settings.detect_server_country = false
            })
    });
    cx.run_until_parked();
    futures::executor::block_on(receipt).unwrap();
    let installed = cx.update(|cx| {
        crate::test_support::install_owners_with_executor(
            Connections::with_documents(Default::default(), Default::default()),
            crate::FactsPrepared::in_memory(crate::ObservationDeps::new(
                script.clone(),
                script.clone(),
                script.clone(),
                true,
                Arc::new(nocterm_connections_model::os_observation::OfflineOsDetector),
            )),
            original.settings().clone(),
            gpui_kit::BackgroundExecutor::new(Arc::new(gpui_kit::ThreadedDispatcher::new())),
            cx,
        )
    });
    cx.update_window(window, |_, window, cx| {
        open_editor(installed.clone(), None, workspace.downgrade(), window, cx);
        window.render_frame(cx);
        window.scroll(
            "editor-section-content",
            gpui_kit::ScrollDelta::Pixels(gpui_kit::point(px(0.), px(-1200.))),
            cx,
        );
        window.render_frame(cx);
        window.click("editor-country", cx);
        window.input("fr", cx);
    })
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        cx.run_until_parked();
        if cx.read(|cx| installed.flag("fr", cx).is_some()) {
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::yield_now();
    }
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("dialog").is_some());
        assert_eq!(window.focused_input(cx).unwrap().value(cx).as_ref(), "fr");
        assert_eq!(
            installed.connections().read(cx).profiles(cx).iter().count(),
            0
        );
        assert_eq!(workspace.read(cx).items().count(), 0);
    })
    .unwrap();
    assert_eq!(
        script.calls(),
        vec![ObservationCall::Flag(CountryCode::new("fr").unwrap())]
    );
}
