//! A destructive button asks first; only the named confirm calls the port.

use super::*;

/// The three removals: the button that asks, what it removes and the call the port gets.
fn removals() -> [(&'static str, &'static str, Call); 3] {
    [
        ("user-plugin-remove", "Remove", Call::Remove(id(), false)),
        (
            "user-plugin-remove-all",
            "Remove with its data",
            Call::Remove(id(), true),
        ),
        (
            "user-plugin-remove-data",
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

#[gpui_kit::test]
fn one_click_asks_and_only_the_confirm_calls_the_port_once(cx: &mut TestAppContext) {
    for (button, label, call) in removals() {
        let fixture = open(vec![row()], Ok(candidate()), cx);
        select(&fixture, cx);
        frame(&fixture, cx, |window, cx| window.click(button, cx));
        cx.run_until_parked();
        assert!(fixture.plugins.calls().is_empty(), "{button} only asks");
        frame(&fixture, cx, |window, cx| {
            assert!(window.try_find("user-removal").is_some());
            assert!(
                window.try_find(button).is_none(),
                "the question replaces it"
            );
            assert_eq!(window.find("user-removal-confirm").label(), Some(label));
            window.click("user-removal-confirm", cx);
        });
        cx.run_until_parked();
        assert_eq!(fixture.plugins.calls(), vec![call], "{button}");
        frame(&fixture, cx, |window, _| {
            assert!(window.try_find("user-removal").is_none());
        });
    }
}

#[gpui_kit::test]
fn the_question_names_the_plugin_and_what_goes(cx: &mut TestAppContext) {
    let fixture = open(vec![row()], Ok(candidate()), cx);
    select(&fixture, cx);
    frame(&fixture, cx, |window, cx| {
        window.click("user-plugin-remove-all", cx)
    });
    frame(&fixture, cx, |window, _| {
        let question = window.find("user-removal").label().unwrap().to_owned();
        assert!(question.contains("Clock") && question.contains("delete its data"));
    });
}

#[gpui_kit::test]
fn enter_on_the_question_removes_nothing_and_escape_withdraws_it(cx: &mut TestAppContext) {
    let fixture = open(vec![row()], Ok(candidate()), cx);
    select(&fixture, cx);
    frame(&fixture, cx, |window, cx| {
        window.click("user-plugin-remove-all", cx)
    });

    frame(&fixture, cx, |window, cx| window.press("enter", cx));
    cx.run_until_parked();
    frame(&fixture, cx, |window, _| {
        assert!(
            window.try_find("user-removal").is_some(),
            "Enter does not answer the question"
        );
    });
    assert!(fixture.plugins.calls().is_empty());

    frame(&fixture, cx, |window, cx| window.press("escape", cx));
    frame(&fixture, cx, |window, _| {
        assert!(window.try_find("user-removal").is_none());
    });
    assert!(fixture.plugins.calls().is_empty());
}

#[gpui_kit::test]
fn a_question_does_not_come_back_after_an_install_was_looked_at(cx: &mut TestAppContext) {
    let fixture = open(vec![row()], Ok(candidate()), cx);
    select(&fixture, cx);
    frame(&fixture, cx, |window, cx| {
        window.click("user-plugin-remove", cx)
    });

    begin(&fixture, cx);
    frame(&fixture, cx, |window, cx| {
        window.click("user-consent-deny", cx)
    });

    frame(&fixture, cx, |window, _| {
        assert!(window.try_find("user-removal").is_none());
    });
    assert!(fixture.plugins.calls().is_empty());
}

#[gpui_kit::test]
fn cancel_and_escape_withdraw_the_question_without_calling_the_port(cx: &mut TestAppContext) {
    let fixture = open(vec![row()], Ok(candidate()), cx);
    select(&fixture, cx);
    frame(&fixture, cx, |window, cx| {
        window.click("user-plugin-remove", cx)
    });
    frame(&fixture, cx, |window, cx| {
        window.click("user-removal-cancel", cx)
    });
    frame(&fixture, cx, |window, _| {
        assert!(window.try_find("user-removal").is_none());
        assert!(window.try_find("user-plugin-remove").is_some());
    });
    frame(&fixture, cx, |window, cx| {
        window.click("user-plugin-remove-data", cx)
    });
    frame(&fixture, cx, |window, cx| window.press("escape", cx));
    frame(&fixture, cx, |window, _| {
        assert!(window.try_find("user-removal").is_none());
    });
    assert!(fixture.plugins.calls().is_empty());
}

#[gpui_kit::test]
fn selecting_another_plugin_clears_the_pending_removal(cx: &mut TestAppContext) {
    let other = UserPluginRow {
        id: PluginId::owned("user-acme-cpu"),
        name: "Cpu".into(),
        ..row()
    };
    let fixture = open(vec![row(), other], Ok(candidate()), cx);
    select(&fixture, cx);
    frame(&fixture, cx, |window, cx| {
        window.click("user-plugin-remove", cx)
    });
    frame(&fixture, cx, |window, cx| {
        window.click("user-plugin-row:user-acme-cpu", cx)
    });
    frame(&fixture, cx, |window, cx| {
        window.click("user-plugin-row:user-acme-clock", cx)
    });
    frame(&fixture, cx, |window, _| {
        assert!(window.try_find("user-removal").is_none());
        assert!(window.try_find("user-plugin-remove").is_some());
    });
    assert!(fixture.plugins.calls().is_empty());
}

#[gpui_kit::test]
fn a_refused_removal_says_why(cx: &mut TestAppContext) {
    let fixture = open(vec![row()], Ok(candidate()), cx);
    select(&fixture, cx);
    frame(&fixture, cx, |window, cx| {
        window.click("user-plugin-remove", cx)
    });
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
