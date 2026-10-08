use super::*;
use gpui_kit::px;
use gpui_kit::{TestAppContext, base::actions::Confirm, test::TestWindowExt as _};
use nocterm_session::Target;
use std::path::{Path, PathBuf};

#[gpui_kit::test]
fn dialog_focus_accepts_typing_and_enter_opens_exactly_once(cx: &mut TestAppContext) {
    let (handle, workspace, opened) = crate::test_support::workspace(cx);
    cx.update_window(handle, |_, window, cx| {
        open_editor(None, workspace.downgrade(), window, cx);
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
            Connections::global(cx).read(cx).profiles().iter().count(),
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
    let (handle, workspace, opened) = crate::test_support::workspace(cx);
    cx.update_window(handle, |_, window, cx| {
        open_editor(None, workspace.downgrade(), window, cx);
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
            Connections::global(cx).read(cx).profiles().iter().count(),
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
            Connections::global(cx).read(cx).profiles().iter().count(),
            1
        );
        assert_eq!(opened.borrow().len(), 1);
    })
    .unwrap();
}

#[gpui_kit::test]
#[expect(clippy::too_many_lines, reason = "predates the limit")]
fn sections_keep_drafts_and_multiline_description_enter_does_not_submit(cx: &mut TestAppContext) {
    let (handle, workspace, opened) = crate::test_support::workspace(cx);
    let editor = cx
        .update_window(handle, |_, window, cx| {
            let editor = open_editor_view(None, workspace.downgrade(), window, cx);
            window.render_frame(cx);
            window.input("draft.test", cx);
            let focus = editor.read(cx).description.read(cx).focus_handle(cx);
            window.focus(&focus, cx);
            window.render_frame(cx);
            window.input("first line", cx);
            window.press("enter", cx);
            window.input("second line", cx);
            editor
        })
        .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(
            window.try_find("dialog").is_some(),
            "Textarea Enter must not submit"
        );
        assert!(opened.borrow().is_empty());
        assert_eq!(
            editor.read(cx).description.read(cx).value().as_ref(),
            "first line\nsecond line"
        );
        assert!(window.find("editor-description").bounds().size.height >= px(80.));
        window.click("editor-section-authentication", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("auth-password", cx);
        window.click("editor-section-session", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("session-term").is_some());
        window.click("editor-section-launch", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("profile-launch-toggle", cx);
        editor.update(cx, |editor, cx| {
            editor.launch_fields[0]
                .update(cx, |input, cx| input.set_value("/bin/bash", window, cx));
        });
        window.click("editor-section-connection", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(editor.read(cx).fields(cx).host, "draft.test");
        assert_eq!(
            editor.read(cx).fields(cx).description,
            "first line\nsecond line"
        );
        assert_eq!(editor.read(cx).auth, AuthKind::Password);
        window.click("editor-save", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("dialog").is_none());
        let profile = Connections::global(cx)
            .read(cx)
            .profiles()
            .iter()
            .next()
            .unwrap();
        assert_eq!(profile.description, "first line\nsecond line");
        assert_eq!(profile.auth, Auth::Password);
        assert_eq!(
            profile.launch.as_ref().unwrap().program.as_deref(),
            Some("/bin/bash")
        );
        assert!(opened.borrow().is_empty(), "Save does not connect");
    })
    .unwrap();
}

#[gpui_kit::test]
#[expect(clippy::too_many_lines, reason = "predates the limit")]
fn hidden_invalid_fields_open_their_section_and_footer_stays_visible(cx: &mut TestAppContext) {
    let (handle, workspace, _) = crate::test_support::workspace(cx);
    cx.simulate_window_resize(handle, gpui_kit::size(px(640.), px(560.)));
    let editor = cx
        .update_window(handle, |_, window, cx| {
            let editor = open_editor_view(None, workspace.downgrade(), window, cx);
            window.render_frame(cx);
            window.input("valid.test", cx);
            editor.update(cx, |editor, cx| {
                editor.auth = AuthKind::Key;
                editor
                    .credential
                    .update(cx, |input, cx| input.set_value("not-an-id", window, cx));
            });
            window.click("editor-save", cx);
            editor
        })
        .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(editor.read(cx).section == EditorSection::Authentication);
        assert!(
            editor
                .read(cx)
                .key_path
                .read(cx)
                .focus_handle(cx)
                .is_focused(window)
        );
        let save = window.find("editor-save").bounds();
        assert!(save.bottom() <= window.viewport_size().height);
        assert!(save.left() >= px(0.) && save.right() <= window.viewport_size().width);
        editor.update(cx, |editor, cx| {
            editor
                .key_path
                .update(cx, |input, cx| input.set_value("/tmp/key", window, cx));
        });
        window.click("editor-save", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(
            editor
                .read(cx)
                .credential
                .read(cx)
                .focus_handle(cx)
                .is_focused(window)
        );
        assert!(editor.read(cx).error.is_some());
        assert_eq!(
            Connections::global(cx).read(cx).profiles().iter().count(),
            0
        );
        editor.update(cx, |editor, cx| {
            editor
                .credential
                .update(cx, |input, cx| input.set_value("", window, cx));
            editor.options.update(cx, |options, cx| {
                options.reset(
                    nocterm_session::SessionOptions {
                        term: Some("invalid TERM".into()),
                        ..Default::default()
                    },
                    window,
                    cx,
                )
            });
            editor.select_section(EditorSection::Connection, window, cx);
        });
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
        assert!(editor.read(cx).section == EditorSection::Session);
        assert!(window.try_find("session-term").is_some());
        assert!(
            editor
                .read(cx)
                .options
                .read(cx)
                .focus_handle(cx)
                .contains_focused(window, cx)
        );
        assert!(editor.read(cx).error.is_some());
        assert_eq!(editor.read(cx).fields(cx).host, "valid.test");
        assert!(window.find("editor-save").bounds().bottom() <= window.viewport_size().height);
    })
    .unwrap();
}

#[gpui_kit::test]
fn minimum_window_keeps_footer_in_view_for_every_section_and_wrapped_error(
    cx: &mut TestAppContext,
) {
    let (handle, workspace, _) = crate::test_support::workspace(cx);
    cx.simulate_window_resize(handle, gpui_kit::size(px(640.), px(400.)));
    let editor = cx.update_window(handle, |_, window, cx| {
        let editor = open_editor_view(None, workspace.downgrade(), window, cx);
        editor.update(cx, |editor, cx| {
            editor.error = Some("Connection changed while this editor was open. Review the current saved connection before applying this draft.".into());
            cx.notify();
        });
        editor
    }).unwrap();
    for (_, id, _) in EditorSection::ALL {
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.click(id, cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            for button in ["editor-save", "editor-save-connect", "editor-cancel"] {
                let bounds = window.find(button).bounds();
                assert!(
                    bounds.top() >= px(0.) && bounds.bottom() <= window.viewport_size().height,
                    "{button}: {bounds:?}"
                );
                assert!(bounds.left() >= px(0.) && bounds.right() <= window.viewport_size().width);
            }
        })
        .unwrap();
    }
    cx.update_window(handle, |_, window, cx| {
        window.click("editor-cancel", cx);
        window.render_frame(cx);
        assert!(window.try_find("dialog").is_none());
        assert!(
            editor
                .read(cx)
                .dismissed
                .load(std::sync::atomic::Ordering::Acquire)
        );
    })
    .unwrap();
}

#[expect(clippy::too_many_lines, reason = "predates the limit")]
fn reload_conflict_then_finish(cx: &mut TestAppContext, save: bool) {
    use futures::FutureExt as _;
    let (handle, workspace, opened) = crate::test_support::workspace(cx);
    let original = build_profile(
        ProfileId::generate(),
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
    let old_editor = cx
        .update_window(handle, |_, window, cx| {
            Connections::global(cx).update(cx, |connections, cx| {
                connections
                    .save_profile(original.clone(), cx)
                    .now_or_never()
                    .unwrap()
                    .unwrap();
            });
            let editor =
                open_editor_view(Some(original.clone()), workspace.downgrade(), window, cx);
            window.render_frame(cx);
            window.press("ctrl-a", cx);
            window.input("Unsaved stale draft", cx);
            Connections::global(cx).update(cx, |connections, cx| {
                connections
                    .save_profile(latest.clone(), cx)
                    .now_or_never()
                    .unwrap()
                    .unwrap();
            });
            window.click("editor-save", cx);
            editor
        })
        .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(
            old_editor.read(cx).error.is_some(),
            "stale snapshot must conflict"
        );
        assert_eq!(
            old_editor.read(cx).name.read(cx).value().as_ref(),
            "Unsaved stale draft"
        );
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
        assert!(
            window.try_find("dialog").is_none(),
            "finishing the replacement must not reveal the stale editor"
        );
        assert!(
            old_editor
                .read(cx)
                .dismissed
                .load(std::sync::atomic::Ordering::Acquire)
        );
        let connections = Connections::global(cx);
        let stored = connections.read(cx).profiles().get(original.id).unwrap();
        if save {
            latest.name = "Saved after reload".into();
        }
        assert_eq!(
            stored, &latest,
            "reload must validate against the latest saved snapshot and preserve all of its fields"
        );
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

fn fields(host: &str) -> Fields {
    Fields {
        host: host.to_owned(),
        ..Fields::default()
    }
}

fn build(fields: &Fields, auth: AuthKind) -> Result<Profile, String> {
    build_profile(
        ProfileId::generate(),
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

    let chosen = build(
        &Fields {
            icon: "debian".into(),
            icon_color: " e95420 ".into(),
            ..fields("host")
        },
        AuthKind::Auto,
    )
    .unwrap();
    assert_eq!(chosen.icon.as_deref(), Some("debian"));
    assert_eq!(chosen.icon_color.as_deref(), Some("#E95420"));

    let error = build(
        &Fields {
            icon_color: "orange".into(),
            ..fields("host")
        },
        AuthKind::Auto,
    )
    .unwrap_err();
    assert!(error.contains("orange"), "{error}");
}

#[test]
fn countries_are_two_letter_codes_stored_in_lower_case() {
    let plain = build(&fields("host"), AuthKind::Auto).unwrap();
    assert_eq!(plain.country, None);
    let chosen = build(
        &Fields {
            country: " De ".into(),
            ..fields("host")
        },
        AuthKind::Auto,
    )
    .unwrap();
    assert_eq!(chosen.country.as_deref(), Some("de"));
    for invalid in ["DEU", "1x", "../"] {
        let error = build(
            &Fields {
                country: invalid.into(),
                ..fields("host")
            },
            AuthKind::Auto,
        )
        .unwrap_err();
        assert!(error.contains("country code"), "{error}");
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
    let profile = build(
        &Fields {
            name: "Build".into(),
            host: "build.local".into(),
            port: "2222".into(),
            user: "ci".into(),
            group: " Work ".into(),
            key_path: "~/.ssh/ci".into(),
            ..Fields::default()
        },
        AuthKind::Key,
    )
    .unwrap();

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
    let id = nocterm_session::CredentialId::generate().unwrap();
    draft.credential = id.to_string();
    draft.override_launch = true;
    draft.program = "/bin/bash".into();
    draft.args = r#"["-c","printf '%s' \"a b\""]"#.into();
    draft.env = r#"{"LANG":"C"}"#.into();
    draft.cwd = "/tmp/a b".into();
    draft.launch_integration = false;
    let profile = build(&draft, AuthKind::Auto).unwrap();
    assert_eq!(profile.credential, Some(id));
    let launch = profile.launch.unwrap();
    assert_eq!(launch.args, vec!["-c", "printf '%s' \"a b\""]);
    assert!(!launch.integration);
    draft.env = r#"{"INVALID;KEY":"value"}"#.into();
    assert!(build(&draft, AuthKind::Auto).is_err());
    draft.override_launch = false;
    draft.credential = "plain password".into();
    assert!(build(&draft, AuthKind::Auto).is_err());
}

#[test]
fn starting_directory_sets_launch_cwd_without_override() {
    let mut draft = fields("example.com");
    draft.cwd = "/var/www".into();
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
    assert!(
        build(
            &Fields {
                port: "ssh".into(),
                ..fields("host")
            },
            AuthKind::Auto
        )
        .unwrap_err()
        .contains("port")
    );
    assert!(
        build(&fields("host"), AuthKind::Key)
            .unwrap_err()
            .contains("key")
    );
    assert!(
        build_profile(
            ProfileId::generate(),
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
