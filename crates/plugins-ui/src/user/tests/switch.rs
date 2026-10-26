//! The switch that allows user plugins at the next start, and the held run.

use super::*;

#[gpui_kit::test]
fn a_run_that_is_not_allowed_shows_the_switch_and_why_and_no_list_or_install(
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
                .starts_with("User plugins are off")
        );
        assert!(window.try_find("user-plugins-restart").is_none());
        assert!(window.try_find("user-install").is_none());
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
        assert!(!label.contains("User plugins are off"), "{label}");
        assert_eq!(window.find("user-plugins-allowed").checked(), Some(false));
        assert!(window.try_find("user-plugins-restart").is_none());
        assert!(window.try_find("user-install").is_none());
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
        assert!(window.try_find("user-install").is_none());
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
            Some("Restart Nocterm to load user plugins.")
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
            Some("Restart Nocterm to stop loading user plugins.")
        );
        assert!(window.try_find("user-plugin-row:user-acme-clock").is_some());
        assert!(window.try_find("user-install").is_some());
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
