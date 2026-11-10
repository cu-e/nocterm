//! The switch that allows user plugins at the next start, and the held run.

use super::*;

#[gpui_kit::test]
fn a_run_that_is_not_allowed_shows_the_switch_why_and_the_folder_but_no_list_or_install(
    cx: &mut TestAppContext,
) {
    let fixture = open_held(HeldBy::NotAllowed, false, cx);
    frame(&fixture, cx, |window, _| {
        assert_eq!(
            window.find("user-plugins-allowed").checked(),
            Some(false),
            "the stored choice"
        );
        assert!(
            window
                .find("user-plugins-held")
                .label()
                .unwrap()
                .starts_with("External plugins are off")
        );
        assert!(window.try_find("user-plugins-restart").is_none());
        assert!(
            window.try_find("user-install-held").is_some(),
            "says what to do first"
        );
        assert!(window.try_find("user-open-folder").is_some());
        assert!(
            window.try_find("user-folder").is_none(),
            "nothing to install from"
        );
        assert!(window.try_find("user-plugin-list").is_none());
        assert!(window.try_find("user-plugin-row:user-acme-clock").is_none());
    });
}

#[gpui_kit::test]
fn a_run_whose_table_does_not_read_says_why_before_any_click(cx: &mut TestAppContext) {
    let by = HeldBy::Unreadable {
        reason: "invalid type: integer `3`".into(),
    };
    let fixture = open_held(by, false, cx);
    frame(&fixture, cx, |window, _| {
        let held = window.find("user-plugins-held");
        let label = held.label().unwrap();
        assert!(
            label.starts_with("The [plugins] table of settings.toml does not read"),
            "{label}"
        );
        assert!(label.contains("invalid type: integer `3`"), "{label}");
        assert!(!label.contains("External plugins are off"), "{label}");
        assert_eq!(window.find("user-plugins-allowed").checked(), Some(false));
        assert!(window.try_find("user-plugins-restart").is_none());
        assert!(
            window.try_find("user-install-held").is_some(),
            "says what to do first"
        );
        assert!(window.try_find("user-plugin-list").is_none());
    });
}

#[gpui_kit::test]
fn a_safe_mode_run_says_so_and_the_switch_only_stores_the_next_start(cx: &mut TestAppContext) {
    let fixture = open_held(HeldBy::SafeMode, false, cx);
    frame(&fixture, cx, |window, cx| {
        assert!(
            window
                .find("user-plugins-held")
                .label()
                .unwrap()
                .starts_with("Safe mode")
        );
        window.click("user-plugins-allowed", cx);
    });
    cx.run_until_parked();
    assert_eq!(*fixture.switch.calls.borrow(), [true]);
    assert!(fixture.plugins.calls().is_empty(), "no plugin was touched");
    frame(&fixture, cx, |window, _| {
        assert_eq!(window.find("user-plugins-allowed").checked(), Some(true));
        assert!(window.try_find("user-plugin-list").is_none());
        assert!(
            window.try_find("user-install-held").is_some(),
            "says what to do first"
        );
    });
}

#[gpui_kit::test]
fn a_changed_switch_asks_for_a_restart_until_it_is_flipped_back(cx: &mut TestAppContext) {
    let fixture = open_held(HeldBy::NotAllowed, false, cx);
    frame(&fixture, cx, |window, cx| {
        window.click("user-plugins-allowed", cx);
    });
    cx.run_until_parked();
    frame(&fixture, cx, |window, cx| {
        assert_eq!(
            window.find("user-plugins-restart").label(),
            Some("Restart Nocterm to load external plugins.")
        );
        window.click("user-plugins-allowed", cx);
    });
    cx.run_until_parked();
    frame(&fixture, cx, |window, _| {
        assert!(window.try_find("user-plugins-restart").is_none());
    });
}

#[gpui_kit::test]
fn switching_off_a_loaded_run_keeps_the_list_and_asks_for_a_restart(cx: &mut TestAppContext) {
    let fixture = open(vec![row()], Ok(candidate()), cx);
    frame(&fixture, cx, |window, cx| {
        assert_eq!(window.find("user-plugins-allowed").checked(), Some(true));
        assert!(window.try_find("user-plugins-restart").is_none());
        assert!(window.try_find("user-plugins-held").is_none());
        window.click("user-plugins-allowed", cx);
    });
    cx.run_until_parked();
    frame(&fixture, cx, |window, _| {
        assert_eq!(
            window.find("user-plugins-restart").label(),
            Some("Restart Nocterm to stop loading external plugins.")
        );
        assert!(window.try_find("user-plugin-row:user-acme-clock").is_some());
        assert!(window.try_find("user-install").is_some());
        assert!(window.try_find("user-install-held").is_none());
    });
}

#[gpui_kit::test]
fn a_refused_write_shows_why_and_the_switch_keeps_its_value(cx: &mut TestAppContext) {
    for error in [
        EnablementError::Malformed {
            reason: "invalid type".into(),
        },
        EnablementError::Write(SharedCause::new(std::io::Error::other("disk full"))),
    ] {
        let fixture = open_held(HeldBy::NotAllowed, false, cx);
        let expected = describe(&error);
        fixture.switch.refuse.replace(Some(error));
        frame(&fixture, cx, |window, cx| {
            window.click("user-plugins-allowed", cx);
        });
        cx.run_until_parked();
        frame(&fixture, cx, |window, _| {
            assert_eq!(window.find("user-notice").label(), Some(expected.as_str()));
            assert_eq!(window.find("user-plugins-allowed").checked(), Some(false));
        });
    }
}

#[gpui_kit::test]
fn a_developer_session_has_no_switch(cx: &mut TestAppContext) {
    let fixture = open(vec![], Ok(candidate()), cx);
    fixture.plugins.dev.replace(Some("/work/clock.wasm".into()));
    fixture.switch.allowed.set(false);
    fixture.section.update(cx, |_, cx| cx.notify());
    frame(&fixture, cx, |window, _| {
        assert!(window.try_find("user-plugins-allowed").is_none());
        assert!(window.try_find("user-plugins-restart").is_none());
        assert!(window.try_find("user-dev-load").is_some());
    });
}

#[gpui_kit::test]
fn the_plugins_folder_is_made_in_a_held_run_too_and_a_failure_says_why(cx: &mut TestAppContext) {
    let fixture = open_held(HeldBy::NotAllowed, false, cx);
    frame(&fixture, cx, |window, cx| {
        window.click("user-open-folder", cx)
    });
    cx.run_until_parked();
    assert_eq!(
        fixture
            .folder
            .prepared
            .load(std::sync::atomic::Ordering::SeqCst),
        1
    );
    let text = fixture
        .section
        .read_with(cx, |section, _| section.folder().error().cloned());
    assert_eq!(
        text.as_deref(),
        Some("The plugins folder could not be opened: read-only file system")
    );
}

#[gpui_kit::test]
fn a_file_in_the_plugins_folder_installs_through_the_consent(cx: &mut TestAppContext) {
    let fixture = open(vec![], Ok(candidate()), cx);
    cx.run_until_parked();
    frame(&fixture, cx, |window, cx| {
        assert!(window.try_find("user-install").is_some());
        assert!(window.try_find("user-install-held").is_none());
        assert!(window.try_find("user-folder").is_some());
        assert_eq!(
            window.find(("user-folder-file", 0usize)).label(),
            Some("weather.wasm")
        );
        window.click(("user-folder-install", 0usize), cx);
    });
    cx.run_until_parked();
    assert!(
        fixture.plugins.calls().is_empty(),
        "nothing installs unasked"
    );
    frame(&fixture, cx, |window, _| {
        assert!(
            window.try_find("user-consent").is_some(),
            "nothing installs unasked"
        );
        assert!(window.try_find("user-folder").is_none());
    });
}
