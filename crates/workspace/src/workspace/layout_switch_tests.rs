//! Choosing the window layout: the selection is an app-wide setting that every window follows,
//! while an edit of the selected layout only rearranges the window.
use std::{cell::RefCell, rc::Rc};

use gpui_kit::{
    AnyWindowHandle, App, AppContext as _, Entity, Focusable as _, TestAppContext, Window,
    WindowOptions, test::TestWindowExt as _,
};
use nocterm_plugin_model::ContributionId;
use nocterm_settings::SettingsDocument;
use nocterm_store::{SettingsConfig, SettingsHandle};
use nocterm_window_layout::{LayoutSettings, PresetId};

use super::{LayoutSwitchError, Workspace};
use crate::{
    PanelRegistration,
    test_support::{EXPANDABLE, PANEL_A, PANEL_B, TEST_PLUGIN, TestPanel, deps_over},
    workspace::tests::fixture_with_deps,
};

fn pid(id: &str) -> PresetId {
    PresetId::from_owned(id.to_owned())
}

fn panel(id: &'static ContributionId) -> ContributionId {
    id.clone()
}

/// A saved layout over the test panels: `open` shows on a switch, the others are the members
/// of the navigation group on the left and of the assistant group on the right. An empty
/// `assistant` leaves that group out of the bar.
fn preset(
    id: &str,
    name: &str,
    open: &[&str],
    navigation: &[&str],
    assistant: &[&str],
) -> serde_json::Value {
    serde_json::json!({
        "id": id,
        "name": name,
        "open": open,
        "bar": {
            "left": [{ "group": "navigation", "primary": true, "members": navigation }],
            "right": if assistant.is_empty() {
                serde_json::json!([])
            } else {
                serde_json::json!([{ "group": "assistant", "members": assistant }])
            },
        },
    })
}

fn layouts_of(presets: Vec<serde_json::Value>) -> LayoutSettings {
    serde_json::from_value(serde_json::json!({ "presets": presets }))
        .expect("the stored layouts are valid")
}

/// The layouts stored by the test: two presets that place the same panels and differ in the
/// panels they open.
fn stored() -> LayoutSettings {
    let place = |id, name, open| {
        preset(
            id,
            name,
            &[open],
            &["test.panel-a", "test.panel-b"],
            &["test.expandable"],
        )
    };
    layouts_of(vec![
        place("p1", "Reading", "test.panel-a"),
        place("p2", "Writing", "test.panel-b"),
    ])
}

pub(super) struct Windows {
    pub(super) handle: AnyWindowHandle,
    pub(super) workspace: Entity<Workspace>,
    pub(super) settings: SettingsHandle,
}

/// A window over fresh shared settings that already hold [`stored`] layouts.
pub(super) fn windows(cx: &mut TestAppContext) -> Windows {
    windows_over(cx, SettingsDocument::default(), Some(stored()), false)
}

/// A window over fresh shared settings holding `document`, with `layouts` stored in it, and
/// the legacy swapped flag as given.
pub(super) fn windows_over(
    cx: &mut TestAppContext,
    mut document: SettingsDocument,
    layouts: Option<LayoutSettings>,
    legacy_swapped: bool,
) -> Windows {
    let shared: Rc<RefCell<Option<SettingsHandle>>> = Rc::default();
    let slot = shared.clone();
    let (handle, workspace, _) = fixture_with_deps(cx, move |layout, cx| {
        document.register::<LayoutSettings>();
        if let Some(layouts) = layouts {
            document.set(layouts);
        }
        layout
            .set_flag("workspace.swapped", legacy_swapped, cx)
            .expect("the flag is kept in memory");
        let settings = SettingsConfig::in_memory(document).install(cx);
        *slot.borrow_mut() = Some(settings.clone());
        deps_over(layout, &[], &[&TEST_PLUGIN]).with_settings(settings)
    });
    let settings = shared.take().expect("the fixture built its dependencies");
    Windows {
        handle,
        workspace,
        settings,
    }
}

/// Another window over the same settings.
pub(super) fn another_window(
    cx: &mut TestAppContext,
    settings: &SettingsHandle,
) -> (AnyWindowHandle, Entity<Workspace>) {
    cx.update(|cx| {
        let deps = deps_over(
            nocterm_ui::LayoutHandle::in_memory(cx),
            &[],
            &[&TEST_PLUGIN],
        )
        .with_settings(settings.clone());
        let (handle, workspace) =
            gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                cx.new(|cx| Workspace::new(deps, window, cx))
            })
            .unwrap();
        handle
            .update(cx, |_, window, _| window.activate_window())
            .unwrap();
        (handle, workspace)
    })
}

pub(super) fn register_all(
    cx: &mut TestAppContext,
    handle: AnyWindowHandle,
    workspace: &Entity<Workspace>,
) {
    cx.update_window(handle, |_, window, cx| {
        workspace.update(cx, |workspace, cx| {
            for contribution in [PANEL_A, PANEL_B] {
                let panel = TestPanel::builder(contribution.title).build(cx);
                workspace
                    .register_panel(
                        PanelRegistration::new(contribution, move |_, _, _| panel.clone()),
                        window,
                        cx,
                    )
                    .expect("the panel is in the test catalog");
            }
        });
    })
    .unwrap();
}

/// Registers the expandable panel and returns it, so a test can ask which side it is on.
fn register_expandable(
    cx: &mut TestAppContext,
    handle: AnyWindowHandle,
    workspace: &Entity<Workspace>,
) -> Entity<TestPanel> {
    let kept: Rc<RefCell<Option<Entity<TestPanel>>>> = Rc::default();
    let slot = kept.clone();
    cx.update_window(handle, |_, window, cx| {
        workspace.update(cx, |workspace, cx| {
            let registration = PanelRegistration::new(EXPANDABLE, move |_, _, cx| {
                let panel = TestPanel::builder("Expandable").build(cx);
                *slot.borrow_mut() = Some(panel.clone());
                panel
            })
            .expandable();
            workspace
                .register_panel(registration, window, cx)
                .expect("the panel is in the test catalog");
        });
    })
    .unwrap();
    kept.take().expect("the factory ran")
}

fn switch(
    cx: &mut TestAppContext,
    handle: AnyWindowHandle,
    workspace: &Entity<Workspace>,
    id: &str,
) -> Result<(), LayoutSwitchError> {
    let task = cx
        .update_window(handle, |_, _, cx| {
            workspace.update(cx, |workspace, cx| workspace.switch_layout(pid(id), cx))
        })
        .unwrap();
    cx.run_until_parked();
    cx.foreground_executor().block_test(task)
}

/// Puts the focus inside the workspace, where its action handlers listen.
pub(super) fn focus_workspace(workspace: &Entity<Workspace>, window: &mut Window, cx: &mut App) {
    window.render_frame(cx);
    let focus = workspace.read(cx).focus_handle(cx);
    window.focus(&focus, cx);
}

fn selected(cx: &mut TestAppContext, workspace: &Entity<Workspace>) -> String {
    cx.update(|cx| {
        workspace
            .read(cx)
            .layouts()
            .into_iter()
            .find(|choice| choice.is_selected())
            .map(|choice| choice.id().as_str().to_owned())
            .expect("one layout is selected")
    })
}

fn open_panels(cx: &mut TestAppContext, workspace: &Entity<Workspace>) -> Vec<&'static str> {
    cx.update(|cx| {
        [PANEL_A, PANEL_B]
            .into_iter()
            .filter(|contribution| workspace.read(cx).panel_is_open(&contribution.id))
            .map(|contribution| contribution.title)
            .collect()
    })
}

#[gpui_kit::test]
fn the_menu_lists_the_built_ins_then_the_saved_layouts(cx: &mut TestAppContext) {
    let Windows { workspace, .. } = windows(cx);
    let names: Vec<_> = cx.update(|cx| {
        workspace
            .read(cx)
            .layouts()
            .into_iter()
            .map(|choice| choice.name().to_owned())
            .collect()
    });
    assert_eq!(names, ["Default", "Swapped sides", "Reading", "Writing"]);
    assert_eq!(selected(cx, &workspace), "default");
}

#[gpui_kit::test]
fn switching_to_swapped_sides_moves_the_assistant_to_the_left(cx: &mut TestAppContext) {
    let Windows {
        handle, workspace, ..
    } = windows(cx);
    let assistant = register_expandable(cx, handle, &workspace);
    cx.update(|cx| assert!(!assistant.read(cx).docked_left()));
    switch(cx, handle, &workspace, "swapped").unwrap();
    assert_eq!(selected(cx, &workspace), "swapped");
    cx.update(|cx| assert!(assistant.read(cx).docked_left()));
    switch(cx, handle, &workspace, "default").unwrap();
    cx.update(|cx| assert!(!assistant.read(cx).docked_left()));
}

#[gpui_kit::test]
fn switching_to_a_saved_layout_opens_exactly_the_panels_it_names(cx: &mut TestAppContext) {
    let Windows {
        handle, workspace, ..
    } = windows(cx);
    register_all(cx, handle, &workspace);
    switch(cx, handle, &workspace, "p2").unwrap();
    assert_eq!(open_panels(cx, &workspace), [PANEL_B.title]);
    switch(cx, handle, &workspace, "p1").unwrap();
    assert_eq!(open_panels(cx, &workspace), [PANEL_A.title]);
}

#[gpui_kit::test]
fn a_built_in_layout_leaves_the_open_panels_alone(cx: &mut TestAppContext) {
    let Windows {
        handle, workspace, ..
    } = windows(cx);
    register_all(cx, handle, &workspace);
    switch(cx, handle, &workspace, "p1").unwrap();
    switch(cx, handle, &workspace, "swapped").unwrap();
    assert_eq!(open_panels(cx, &workspace), [PANEL_A.title]);
}

#[gpui_kit::test]
fn editing_the_selected_layout_does_not_reopen_its_panels(cx: &mut TestAppContext) {
    let Windows {
        handle,
        workspace,
        settings,
    } = windows(cx);
    register_all(cx, handle, &workspace);
    switch(cx, handle, &workspace, "p1").unwrap();
    cx.update_window(handle, |_, window, cx| {
        workspace.update(cx, |workspace, cx| {
            workspace.toggle_panel(&panel(&PANEL_A.id), window, cx);
        });
    })
    .unwrap();
    assert_eq!(open_panels(cx, &workspace), Vec::<&str>::new());
    let task = cx.update(|cx| {
        settings.update::<LayoutSettings>(cx, |layouts| layouts.presets[0].name = "Skimming".into())
    });
    cx.run_until_parked();
    cx.foreground_executor().block_test(task).unwrap();
    assert_eq!(selected(cx, &workspace), "p1");
    assert_eq!(
        open_panels(cx, &workspace),
        Vec::<&str>::new(),
        "an edit is not a switch"
    );
}

#[gpui_kit::test]
fn a_switch_in_one_window_reaches_the_other(cx: &mut TestAppContext) {
    let Windows {
        handle,
        workspace,
        settings,
    } = windows(cx);
    let (other_handle, other) = another_window(cx, &settings);
    register_all(cx, other_handle, &other);
    switch(cx, handle, &workspace, "p2").unwrap();
    assert_eq!(selected(cx, &other), "p2");
    assert_eq!(open_panels(cx, &other), [PANEL_B.title]);
}

#[gpui_kit::test]
fn a_window_composed_under_a_saved_layout_opens_its_panels(cx: &mut TestAppContext) {
    let Windows {
        handle,
        workspace,
        settings,
    } = windows(cx);
    switch(cx, handle, &workspace, "p1").unwrap();
    let (late_handle, late) = another_window(cx, &settings);
    assert_eq!(selected(cx, &late), "p1");
    register_all(cx, late_handle, &late);
    assert_eq!(open_panels(cx, &late), [PANEL_A.title]);
}

#[gpui_kit::test]
fn an_unknown_layout_is_refused_and_changes_nothing(cx: &mut TestAppContext) {
    let Windows {
        handle, workspace, ..
    } = windows(cx);
    let result = switch(cx, handle, &workspace, "p9");
    assert!(matches!(result, Err(LayoutSwitchError::Unknown(id)) if id.as_str() == "p9"));
    assert_eq!(selected(cx, &workspace), "default");
}

#[gpui_kit::test]
fn next_and_previous_layout_cycle_through_the_menu_and_wrap(cx: &mut TestAppContext) {
    let Windows {
        handle, workspace, ..
    } = windows(cx);
    let press = |cx: &mut TestAppContext, action: Box<dyn gpui_kit::Action>| {
        cx.update_window(handle, |_, window, cx| {
            focus_workspace(&workspace, window, cx);
            window.dispatch_action(action, cx)
        })
        .unwrap();
        cx.run_until_parked();
    };
    press(cx, Box::new(crate::NextLayout));
    assert_eq!(selected(cx, &workspace), "swapped");
    press(cx, Box::new(crate::PreviousLayout));
    press(cx, Box::new(crate::PreviousLayout));
    assert_eq!(selected(cx, &workspace), "p2", "the menu wraps backwards");
    press(cx, Box::new(crate::NextLayout));
    assert_eq!(selected(cx, &workspace), "default", "and forwards");
}

#[gpui_kit::test]
fn the_switch_layout_action_selects_the_layout_it_names(cx: &mut TestAppContext) {
    let Windows {
        handle, workspace, ..
    } = windows(cx);
    cx.update_window(handle, |_, window, cx| {
        focus_workspace(&workspace, window, cx);
        window.dispatch_action(Box::new(crate::SwitchLayout { id: "p1".into() }), cx)
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(selected(cx, &workspace), "p1");
}

fn is_open(
    cx: &mut TestAppContext,
    workspace: &Entity<Workspace>,
    contribution: &'static nocterm_plugin_model::SystemContribution,
) -> bool {
    cx.update(|cx| workspace.read(cx).panel_is_open(&contribution.id))
}

fn press(
    cx: &mut TestAppContext,
    handle: AnyWindowHandle,
    workspace: &Entity<Workspace>,
    action: Box<dyn gpui_kit::Action>,
) {
    cx.update_window(handle, |_, window, cx| {
        focus_workspace(workspace, window, cx);
        window.dispatch_action(action, cx)
    })
    .unwrap();
    cx.run_until_parked();
}

#[gpui_kit::test]
fn a_layouts_table_that_does_not_read_refuses_a_switch_and_stays_as_written(
    cx: &mut TestAppContext,
) {
    let table = "[layouts]\nactive = \"p1\"\n[[layouts.presets]]\nid = \"p1\"\n\
                 [layouts.presets.bar]\nleft = [{ separator = false }]\n";
    let mut document = SettingsDocument::from_table(table.parse().expect("the text is toml"));
    document.register::<LayoutSettings>();
    let written = document.to_table();
    let Windows {
        handle,
        workspace,
        settings,
    } = windows_over(cx, document, None, false);
    let result = switch(cx, handle, &workspace, "swapped");
    assert!(
        matches!(result, Err(LayoutSwitchError::Malformed(_))),
        "{result:?}"
    );
    cx.update(|cx| {
        assert_eq!(settings.document(cx).to_table(), written);
        assert_eq!(settings.revision(cx), 0, "nothing was written");
    });
    assert_eq!(selected(cx, &workspace), "default");
}

#[gpui_kit::test]
fn a_panel_a_layout_places_in_the_assistant_group_follows_that_group(cx: &mut TestAppContext) {
    let layouts = layouts_of(vec![preset(
        "p3",
        "Mixed",
        &[],
        &["test.panel-b"],
        &["test.panel-a"],
    )]);
    let Windows {
        handle, workspace, ..
    } = windows_over(cx, SettingsDocument::default(), Some(layouts), false);
    register_all(cx, handle, &workspace);
    switch(cx, handle, &workspace, "p3").unwrap();
    assert!(!is_open(cx, &workspace, PANEL_A));
    press(cx, handle, &workspace, Box::new(crate::ToggleRightPanel));
    assert!(
        is_open(cx, &workspace, PANEL_A),
        "the assistant group holds it"
    );
    assert!(!is_open(cx, &workspace, PANEL_B));
    press(cx, handle, &workspace, Box::new(crate::ToggleSidebar));
    assert!(
        is_open(cx, &workspace, PANEL_B),
        "the sidebar shows its own"
    );
    assert!(is_open(cx, &workspace, PANEL_A));
}

#[gpui_kit::test]
fn a_primary_flag_on_another_group_does_not_make_it_the_sidebar(cx: &mut TestAppContext) {
    let layouts = layouts_of(vec![serde_json::json!({
        "id": "p3",
        "name": "Flagged",
        "bar": {
            "left": [{ "group": "assistant", "primary": true, "members": ["test.panel-a"] }],
            "right": [{ "group": "navigation", "members": ["test.panel-b"] }],
        },
    })]);
    let Windows {
        handle, workspace, ..
    } = windows_over(cx, SettingsDocument::default(), Some(layouts), false);
    register_all(cx, handle, &workspace);
    switch(cx, handle, &workspace, "p3").unwrap();

    press(cx, handle, &workspace, Box::new(crate::ToggleSidebar));

    assert!(
        is_open(cx, &workspace, PANEL_B),
        "the sidebar is still navigation"
    );
    assert!(!is_open(cx, &workspace, PANEL_A));
}

#[gpui_kit::test]
fn editing_the_members_of_the_selected_layout_keeps_what_is_open_and_releases_the_rest(
    cx: &mut TestAppContext,
) {
    let Windows {
        handle,
        workspace,
        settings,
    } = windows(cx);
    register_all(cx, handle, &workspace);
    register_expandable(cx, handle, &workspace);
    switch(cx, handle, &workspace, "p1").unwrap();
    cx.update_window(handle, |_, window, cx| {
        workspace.update(cx, |workspace, cx| {
            workspace.open_panel(&EXPANDABLE.id, window, cx)
        })
    })
    .unwrap();
    assert!(is_open(cx, &workspace, PANEL_A));
    assert!(is_open(cx, &workspace, EXPANDABLE));
    let mut left_out = preset(
        "p1",
        "Reading",
        &["test.panel-a"],
        &["test.panel-a", "test.panel-b"],
        &[],
    );
    left_out["tray"] = serde_json::json!(["test.expandable"]);
    let edited = layouts_of(vec![left_out]);
    let task = cx.update(|cx| {
        settings.update::<LayoutSettings>(cx, move |layouts| {
            layouts.presets[0] = edited.presets[0].clone()
        })
    });
    cx.run_until_parked();
    cx.foreground_executor().block_test(task).unwrap();
    assert_eq!(selected(cx, &workspace), "p1");
    assert!(
        is_open(cx, &workspace, PANEL_A),
        "a placed open panel stays"
    );
    assert!(
        !is_open(cx, &workspace, EXPANDABLE),
        "a panel the layout no longer places closes"
    );
}

#[gpui_kit::test]
fn the_legacy_swapped_flag_selects_swapped_sides_without_writing_settings(cx: &mut TestAppContext) {
    let Windows {
        handle,
        workspace,
        settings,
    } = windows_over(cx, SettingsDocument::default(), Some(stored()), true);
    let assistant = register_expandable(cx, handle, &workspace);
    assert_eq!(selected(cx, &workspace), "swapped");
    cx.update(|cx| {
        assert!(assistant.read(cx).docked_left());
        assert_eq!(settings.revision(cx), 0, "booting writes nothing");
    });
}

#[gpui_kit::test]
fn an_edit_made_before_any_switch_reaches_both_windows(cx: &mut TestAppContext) {
    let Windows {
        handle,
        workspace,
        settings,
    } = windows(cx);
    let (other_handle, other) = another_window(cx, &settings);
    register_all(cx, handle, &workspace);
    register_all(cx, other_handle, &other);
    register_expandable(cx, handle, &workspace);
    register_expandable(cx, other_handle, &other);
    let edited = layouts_of(vec![preset(
        "p2",
        "Writing",
        &["test.expandable"],
        &["test.panel-a", "test.panel-b"],
        &["test.expandable"],
    )]);
    let task = cx.update(|cx| {
        settings.update::<LayoutSettings>(cx, move |layouts| {
            layouts.presets[1] = edited.presets[0].clone()
        })
    });
    cx.run_until_parked();
    cx.foreground_executor().block_test(task).unwrap();
    for window in [&workspace, &other] {
        assert_eq!(selected(cx, window), "default");
        assert!(!is_open(cx, window, EXPANDABLE), "an edit is not a switch");
    }
    switch(cx, handle, &workspace, "p2").unwrap();
    for window in [&workspace, &other] {
        assert!(is_open(cx, window, EXPANDABLE), "the edited layout opens");
        assert!(!is_open(cx, window, PANEL_A));
    }
}
