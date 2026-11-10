//! The one switch of an external plugin: on turns on what is off, off keeps the place.

use super::*;

#[gpui_kit::test]
fn on_allows_in_windows_only_what_is_not_allowed_yet(cx: &mut TestAppContext) {
    let mut placed = row();
    placed.enabled = false;
    let fixture = open(vec![placed], Ok(candidate()), cx);
    frame(&fixture, cx, |window, cx| {
        window.click("user-plugin-row:user-acme-clock", cx)
    });
    frame(&fixture, cx, |window, cx| {
        window.click("user-plugin-enabled", cx)
    });
    cx.run_until_parked();
    assert_eq!(fixture.plugins.calls(), vec![Call::Enabled(id(), true)]);
}

#[gpui_kit::test]
fn an_updated_plugin_that_is_not_placed_shows_off_and_on_only_places_it(cx: &mut TestAppContext) {
    let mut replaced = row();
    replaced.placeable = false;
    let fixture = open(vec![replaced], Ok(candidate()), cx);
    frame(&fixture, cx, |window, cx| {
        window.click("user-plugin-row:user-acme-clock", cx)
    });
    frame(&fixture, cx, |window, cx| {
        window.click("user-plugin-enabled", cx)
    });
    cx.run_until_parked();
    assert_eq!(fixture.plugins.calls(), vec![Call::Placeable(id(), true)]);
}

#[gpui_kit::test]
fn off_disables_and_keeps_the_place(cx: &mut TestAppContext) {
    let fixture = open(vec![row()], Ok(candidate()), cx);
    frame(&fixture, cx, |window, cx| {
        window.click("user-plugin-row:user-acme-clock", cx)
    });
    frame(&fixture, cx, |window, cx| {
        window.click("user-plugin-enabled", cx)
    });
    cx.run_until_parked();
    assert_eq!(fixture.plugins.calls(), vec![Call::Enabled(id(), false)]);
}
