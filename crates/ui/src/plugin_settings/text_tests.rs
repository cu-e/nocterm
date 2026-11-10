//! Typed text is data: it is sent once, cleaned by the schema, and never overwritten while
//! the user is still typing.

use std::time::Duration;

use gpui_kit::{AppContext as _, TestAppContext, test::TestWindowExt as _};
use nocterm_plugin_model::{FieldEdit, SettingValue};

use super::{
    FormFrame, FormScope, PluginSettingsSource as _,
    tests::{Fixture, id, label_of, open},
};

fn type_name(fixture: &Fixture, text: &str, cx: &mut TestAppContext) {
    cx.update_window(fixture.window, |_, window, cx| {
        window.render_frame(cx);
        window.click(gpui_kit::SharedString::from(id("name", "input")), cx);
        window.press("ctrl-a", cx);
        window.press("backspace", cx);
        window.input(text, cx);
    })
    .unwrap();
    cx.run_until_parked();
}

fn shown(fixture: &Fixture, expected: &str, cx: &mut TestAppContext) -> bool {
    cx.update_window(fixture.window, |_, window, cx| {
        window.render_frame(cx);
        let scope: gpui_kit::ElementId = gpui_kit::SharedString::from(id("name", "input")).into();
        gpui_kit::base::test_support::snapshots(window)
            .iter()
            .any(|e| e.path().contains(&scope) && e.value() == Some(expected))
    })
    .unwrap()
}

fn stored(fixture: &Fixture, cx: &mut TestAppContext) -> String {
    cx.update(|cx| match fixture.source.values(cx).get("name") {
        Some(SettingValue::Text(text)) => text.clone(),
        other => panic!("unexpected {other:?}"),
    })
}

#[gpui_kit::test]
fn typing_is_sent_once_after_a_pause(cx: &mut TestAppContext) {
    let fixture = open(FormScope::Plugin, FormFrame::Host, cx);
    type_name(&fixture, "Oslo", cx);
    assert!(fixture.source.edits().is_empty());
    cx.executor().advance_clock(Duration::from_millis(600));
    cx.run_until_parked();
    assert_eq!(stored(&fixture, cx), "Oslo");
    assert_eq!(fixture.source.edits().len(), 1);
    cx.executor().advance_clock(Duration::from_millis(1200));
    cx.run_until_parked();
    assert_eq!(fixture.source.edits().len(), 1);
}

#[gpui_kit::test]
fn enter_sends_at_once_and_the_pause_does_not_send_again(cx: &mut TestAppContext) {
    let fixture = open(FormScope::Plugin, FormFrame::Host, cx);
    type_name(&fixture, "Oslo", cx);
    cx.update_window(fixture.window, |_, window, cx| window.press("enter", cx))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(stored(&fixture, cx), "Oslo");
    cx.executor().advance_clock(Duration::from_millis(600));
    cx.run_until_parked();
    assert_eq!(fixture.source.edits().len(), 1);
}

#[gpui_kit::test]
fn flush_sends_what_is_still_waiting(cx: &mut TestAppContext) {
    let fixture = open(FormScope::Plugin, FormFrame::Host, cx);
    type_name(&fixture, "Bergen", cx);
    fixture.form.update(cx, |form, cx| form.flush(cx));
    cx.run_until_parked();
    assert_eq!(stored(&fixture, cx), "Bergen");
}

#[gpui_kit::test]
fn text_is_shown_as_typed_and_too_long_text_is_refused_under_the_field(cx: &mut TestAppContext) {
    let fixture = open(FormScope::Plugin, FormFrame::Host, cx);
    type_name(&fixture, "<b>*x*</b>", cx);
    cx.update_window(fixture.window, |_, window, cx| window.press("enter", cx))
        .unwrap();
    cx.run_until_parked();
    assert!(
        shown(&fixture, "<b>*x*</b>", cx),
        "the input keeps what was typed"
    );
    assert_eq!(
        label_of(&fixture, id("name", "error"), cx).as_deref(),
        Some("Use at most 8 characters.")
    );
    assert_eq!(stored(&fixture, cx), "");
}

#[gpui_kit::test]
fn refused_text_stays_beside_its_error_when_something_else_changes(cx: &mut TestAppContext) {
    let fixture = open(FormScope::Plugin, FormFrame::Host, cx);
    type_name(&fixture, "far too long", cx);
    cx.update_window(fixture.window, |_, window, cx| window.press("enter", cx))
        .unwrap();
    cx.run_until_parked();
    cx.update(|cx| fixture.source.set("flag", SettingValue::Bool(true), cx));
    cx.run_until_parked();
    assert!(shown(&fixture, "far too long", cx));
    assert_eq!(
        label_of(&fixture, id("name", "error"), cx).as_deref(),
        Some("Use at most 8 characters.")
    );
}

#[gpui_kit::test]
fn stored_markup_and_links_are_shown_as_literal_text(cx: &mut TestAppContext) {
    let fixture = open(FormScope::Plugin, FormFrame::Host, cx);
    cx.update(|cx| {
        fixture
            .source
            .set("name", SettingValue::Text("<b>x</b> a://b".into()), cx)
    });
    cx.run_until_parked();
    assert!(shown(&fixture, "<b>x</b> a://b", cx));
}

#[gpui_kit::test]
fn a_change_made_elsewhere_does_not_overwrite_text_being_typed(cx: &mut TestAppContext) {
    let fixture = open(FormScope::Plugin, FormFrame::Host, cx);
    type_name(&fixture, "Oslo", cx);
    cx.update(|cx| fixture.source.set("flag", SettingValue::Bool(true), cx));
    cx.run_until_parked();
    assert!(shown(&fixture, "Oslo", cx));
    cx.update(|cx| {
        fixture
            .source
            .set("name", SettingValue::Text("Rome".into()), cx)
    });
    cx.executor().advance_clock(Duration::from_millis(600));
    cx.run_until_parked();
    assert_eq!(
        fixture.source.edits().last().map(|(_, edit)| edit.clone()),
        Some(FieldEdit::Set(SettingValue::Text("Oslo".into())))
    );
}

#[gpui_kit::test]
fn an_untouched_field_follows_changes_made_elsewhere(cx: &mut TestAppContext) {
    let fixture = open(FormScope::Plugin, FormFrame::Host, cx);
    cx.update(|cx| {
        fixture
            .source
            .set("name", SettingValue::Text("Rome".into()), cx)
    });
    cx.run_until_parked();
    assert!(shown(&fixture, "Rome", cx));
}
