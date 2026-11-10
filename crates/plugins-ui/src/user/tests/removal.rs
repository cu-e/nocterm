//! Removal asks in a dialog what goes, the least first; only its confirm calls the port.

use super::*;
use gpui_kit::component::WindowExt as _;

/// The three removals: the choice in the dialog, the confirm's label and the call the port gets.
fn removals() -> [(&'static str, &'static str, Call); 3] {
    [
        ("user-removal-keep", "Remove", Call::Remove(id(), false)),
        (
            "user-removal-with-data",
            "Remove with its data",
            Call::Remove(id(), true),
        ),
        (
            "user-removal-data",
            "Remove its data",
            Call::RemoveData(id()),
        ),
    ]
}

fn select(fixture: &Fixture, cx: &mut TestAppContext) {
    frame(fixture, cx, |window, cx| {
        window.click("user-plugin-row:user-acme-clock", cx)
    });
}

fn ask(fixture: &Fixture, cx: &mut TestAppContext) {
    select(fixture, cx);
    frame(fixture, cx, |window, cx| {
        window.click("user-plugin-remove", cx)
    });
}

#[gpui_kit::test]
fn the_small_remove_button_asks_in_a_dialog_and_only_the_confirm_calls_the_port(
    cx: &mut TestAppContext,
) {
    for (choice, label, call) in removals() {
        let fixture = open(vec![row()], Ok(candidate()), cx);
        ask(&fixture, cx);
        assert!(fixture.plugins.calls().is_empty(), "the button only asks");
        frame(&fixture, cx, |window, cx| {
            assert!(window.has_active_dialog(cx));
            assert_eq!(
                window.find("user-removal-confirm").label(),
                Some("Remove"),
                "the least is chosen first"
            );
            window.click(choice, cx);
        });
        frame(&fixture, cx, |window, cx| {
            assert_eq!(window.find("user-removal-confirm").label(), Some(label));
            window.click("user-removal-confirm", cx);
        });
        assert_eq!(fixture.plugins.calls(), vec![call], "{choice}");
        frame(&fixture, cx, |window, cx| {
            assert!(!window.has_active_dialog(cx));
        });
    }
}

#[gpui_kit::test]
fn the_question_names_the_plugin_and_what_goes(cx: &mut TestAppContext) {
    let fixture = open(vec![row()], Ok(candidate()), cx);
    ask(&fixture, cx);
    frame(&fixture, cx, |window, cx| {
        window.click("user-removal-with-data", cx)
    });
    frame(&fixture, cx, |window, _| {
        let question = window.find("user-removal").label().unwrap().to_owned();
        assert!(question.contains("Clock") && question.contains("delete its data"));
    });
}

#[gpui_kit::test]
fn enter_removes_nothing_and_escape_and_cancel_close_the_dialog(cx: &mut TestAppContext) {
    let fixture = open(vec![row()], Ok(candidate()), cx);
    ask(&fixture, cx);
    frame(&fixture, cx, |window, cx| window.press("enter", cx));
    frame(&fixture, cx, |window, cx| {
        assert!(window.has_active_dialog(cx), "Enter does not answer");
        window.press("escape", cx);
    });
    frame(&fixture, cx, |window, cx| {
        assert!(!window.has_active_dialog(cx));
        window.click("user-plugin-remove", cx);
    });
    frame(&fixture, cx, |window, cx| {
        window.click("user-removal-cancel", cx)
    });
    frame(&fixture, cx, |window, cx| {
        assert!(!window.has_active_dialog(cx));
    });
    assert!(fixture.plugins.calls().is_empty());
}

#[gpui_kit::test]
fn a_refused_removal_says_why(cx: &mut TestAppContext) {
    let fixture = open(vec![row()], Ok(candidate()), cx);
    ask(&fixture, cx);
    frame(&fixture, cx, |window, cx| {
        window.click("user-removal-confirm", cx)
    });
    frame(&fixture, cx, |window, _| {
        assert_eq!(
            window.find("user-notice").label(),
            Some("The change was refused: disk full")
        );
    });
}
