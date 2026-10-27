//! Switching a window asks first only when it would end local tabs.
use crate::test_support::{Fixture, fixture, hub_id, pinned_record, server};
use crate::{OpenEnvironmentMenu, Placement};
use gpui_kit::TestAppContext;
use nocterm_hubs_model::{Environment, testing::FakeHub};

fn window_with_a_tab(environment: Environment, cx: &mut TestAppContext) -> Fixture {
    let window = fixture(
        cx,
        environment,
        FakeHub::new().with_server(server()),
        vec![pinned_record()],
        Some("ada"),
    );
    window.add_tab(cx);
    window
}

/// Chooses the server in the menu of a local window with a tab.
fn choose_the_server(window: &Fixture, cx: &mut TestAppContext) {
    window.dispatch(OpenEnvironmentMenu, cx);
    window.click(("environment-row", 1usize), cx);
    window.settle_dialog(cx);
}

#[gpui_kit::test]
fn a_local_window_with_tabs_asks_before_it_switches(cx: &mut TestAppContext) {
    let window = window_with_a_tab(Environment::Local, cx);
    choose_the_server(&window, cx);
    assert!(window.has_dialog(cx));
    assert!(window.shows("switch-here", cx));
    assert!(window.shows("switch-new-window", cx));
    assert!(window.shows("switch-cancel", cx));
    assert!(
        window.environments.requests().is_empty(),
        "nothing opens before the answer"
    );
}

#[gpui_kit::test]
fn cancelling_the_question_changes_nothing(cx: &mut TestAppContext) {
    let window = window_with_a_tab(Environment::Local, cx);
    choose_the_server(&window, cx);
    window.click("switch-cancel", cx);
    assert!(!window.has_dialog(cx));
    assert!(window.environments.requests().is_empty());
}

#[gpui_kit::test]
fn enter_takes_the_choice_that_keeps_the_tabs(cx: &mut TestAppContext) {
    let window = window_with_a_tab(Environment::Local, cx);
    choose_the_server(&window, cx);
    window.press("enter", cx);
    assert_eq!(
        window.environments.requests(),
        vec![(Environment::Hub(hub_id()), Placement::NewWindow)]
    );
}

#[gpui_kit::test]
fn switch_and_close_tabs_replaces_this_window(cx: &mut TestAppContext) {
    let window = window_with_a_tab(Environment::Local, cx);
    choose_the_server(&window, cx);
    window.click("switch-here", cx);
    assert_eq!(
        window.environments.requests(),
        vec![(Environment::Hub(hub_id()), Placement::ThisWindow)]
    );
}

#[gpui_kit::test]
fn opening_in_a_new_window_asks_nothing(cx: &mut TestAppContext) {
    let window = window_with_a_tab(Environment::Local, cx);
    window.dispatch(OpenEnvironmentMenu, cx);
    window.click(("environment-new-window", 1usize), cx);
    assert!(!window.has_dialog(cx));
    assert_eq!(
        window.environments.requests(),
        vec![(Environment::Hub(hub_id()), Placement::NewWindow)]
    );
}

#[gpui_kit::test]
fn a_hub_window_switches_back_without_asking(cx: &mut TestAppContext) {
    let window = window_with_a_tab(Environment::Hub(hub_id()), cx);
    window.dispatch(OpenEnvironmentMenu, cx);
    window.click(("environment-row", 0usize), cx);
    assert!(!window.has_dialog(cx), "its tabs only detach");
    assert_eq!(
        window.environments.requests(),
        vec![(Environment::Local, Placement::ThisWindow)]
    );
}
