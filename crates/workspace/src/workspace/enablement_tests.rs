//! Disabling a plugin is a setting that every window follows: its panels and widgets stop
//! existing, its feature keeps working, and enabling it again brings them back closed.
use std::{cell::RefCell, rc::Rc};

use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AnyWindowHandle, AppContext as _, Context, Entity, Focusable as _, IntoElement, Render,
    TestAppContext, WeakEntity, Window, div, px, size,
};
use nocterm_plugin_enablement::PluginEnablement;
use nocterm_plugin_model::{ContributionId, PluginId, SystemContribution};
use nocterm_settings::SettingsDocument;
use nocterm_store::{SettingsConfig, SettingsHandle};
use nocterm_ui::notice;
use nocterm_window_layout::LayoutSettings;

use super::{
    Workspace, WorkspaceEvent,
    layout_switch_tests::{Windows, another_window, windows_over},
    tests::{Probe, probe},
};
use crate::{
    PanelRegistration, WidgetRegistration,
    test_support::{
        CORE_PANEL, PANEL_A, PANEL_B, REQUIRED_EXPANDABLE, REQUIRED_PLUGIN, TEST_CORE, TEST_PLUGIN,
        TestPanel, WIDGET, deps_for_core,
    },
    workspace::tests::fixture_with_deps,
};

/// The panels a window's factories created, recorded weakly so a test can tell they dropped.
type Made = Rc<RefCell<Vec<WeakEntity<TestPanel>>>>;

struct Shell {
    handle: AnyWindowHandle,
    workspace: Entity<Workspace>,
    made: Made,
}

fn id(contribution: &SystemContribution) -> ContributionId {
    contribution.id.clone()
}

fn enablement(disabled: &[&'static str]) -> PluginEnablement {
    PluginEnablement {
        disabled: disabled.iter().map(|id| PluginId::new(id)).collect(),
    }
}

/// Registers panels whose factories create a new entity each time and keep no handle to it.
fn register_panels(
    cx: &mut TestAppContext,
    handle: AnyWindowHandle,
    workspace: &Entity<Workspace>,
    panels: &[&'static SystemContribution],
) -> Made {
    let made = Made::default();
    cx.update_window(handle, |_, window, cx| {
        window.resize(size(px(1400.), px(700.)));
        workspace.update(cx, |workspace, cx| {
            for contribution in panels.iter().copied() {
                let record = made.clone();
                let registration = PanelRegistration::new(contribution, move |_, _, cx| {
                    let panel = TestPanel::builder(contribution.title).build(cx);
                    record.borrow_mut().push(panel.downgrade());
                    panel
                });
                workspace
                    .register_panel(registration, window, cx)
                    .expect("the panel is in the test catalog");
            }
        });
        window.render_frame(cx);
    })
    .unwrap();
    made
}

/// Two windows over the same settings, with the test plugin's sidebar panels registered in
/// both. `layouts` is stored in the settings when given.
fn two_windows(
    cx: &mut TestAppContext,
    disabled: &[&'static str],
    layouts: Option<LayoutSettings>,
) -> (Shell, Shell, SettingsHandle) {
    let mut document = SettingsDocument::default();
    document.register::<PluginEnablement>();
    document.set(enablement(disabled));
    let Windows {
        handle,
        workspace,
        settings,
    } = windows_over(cx, document, layouts, false);
    let (other_handle, other) = another_window(cx, &settings);
    let panels = [PANEL_A, PANEL_B];
    let made = register_panels(cx, handle, &workspace, &panels);
    let other_made = register_panels(cx, other_handle, &other, &panels);
    (
        Shell {
            handle,
            workspace,
            made,
        },
        Shell {
            handle: other_handle,
            workspace: other,
            made: other_made,
        },
        settings,
    )
}

/// Writes `[plugins]` and lets every window follow.
fn write_disabled(cx: &mut TestAppContext, settings: &SettingsHandle, disabled: &[&'static str]) {
    let next = enablement(disabled);
    let task = cx.update(|cx| settings.update::<PluginEnablement>(cx, move |value| *value = next));
    cx.run_until_parked();
    cx.foreground_executor().block_test(task).unwrap();
    cx.run_until_parked();
}

fn run<R>(
    cx: &mut TestAppContext,
    window: &Shell,
    f: impl FnOnce(&mut Workspace, &mut Window, &mut Context<Workspace>) -> R,
) -> R {
    cx.update_window(window.handle, |_, gpui_window, cx| {
        window
            .workspace
            .update(cx, |workspace, cx| f(workspace, gpui_window, cx))
    })
    .unwrap()
}

fn alive(window: &Shell) -> usize {
    window
        .made
        .borrow()
        .iter()
        .filter(|panel| panel.upgrade().is_some())
        .count()
}

fn is_open(cx: &mut TestAppContext, window: &Shell, panel: &SystemContribution) -> bool {
    cx.update(|cx| window.workspace.read(cx).panel_is_open(&id(panel)))
}

fn listed(cx: &mut TestAppContext, window: &Shell) -> usize {
    cx.update(|cx| {
        window
            .workspace
            .read(cx)
            .panel_menu()
            .iter()
            .flatten()
            .count()
    })
}

fn activate(cx: &mut TestAppContext, window: &Shell) {
    cx.update_window(window.handle, |_, gpui_window, cx| {
        gpui_window.activate_window();
        gpui_window.render_frame(cx);
    })
    .unwrap();
}

fn open(cx: &mut TestAppContext, window: &Shell, panel: &SystemContribution) {
    run(cx, window, |workspace, gpui_window, cx| {
        workspace.open_panel(&id(panel), gpui_window, cx)
    });
}

#[gpui_kit::test]
fn disabling_a_plugin_closes_and_drops_its_panels_in_every_window(cx: &mut TestAppContext) {
    let (first, second, settings) = two_windows(cx, &[], None);
    open(cx, &first, PANEL_A);
    open(cx, &second, PANEL_B);
    let events = Rc::new(RefCell::new(Vec::new()));
    let seen = events.clone();
    let _subscription = cx.update(|cx| {
        cx.subscribe(&first.workspace, move |_, event, _| {
            seen.borrow_mut().push(*event)
        })
    });
    assert_eq!((alive(&first), alive(&second)), (2, 2));
    assert_eq!((listed(cx, &first), listed(cx, &second)), (2, 2));

    write_disabled(cx, &settings, &["test"]);

    for window in [&first, &second] {
        assert!(!is_open(cx, window, PANEL_A));
        assert!(!is_open(cx, window, PANEL_B));
        assert_eq!(alive(window), 0, "the panel entities are gone");
        assert_eq!(listed(cx, window), 0, "Window > Panels omits them");
        cx.update(|cx| assert!(!window.workspace.read(cx).panel_is_enabled(&id(PANEL_A))));
    }
    assert!(
        events
            .borrow()
            .contains(&WorkspaceEvent::PanelVisibilityChanged)
    );
}

/// Adds a tab to the window, so there is a central item for focus to return to.
fn add_tab(cx: &mut TestAppContext, window: &Shell) -> Entity<Probe> {
    cx.update_window(window.handle, |_, gpui_window, cx| {
        let tab = probe(cx, Rc::default());
        window.workspace.update(cx, |workspace, cx| {
            workspace.add_item(tab.clone(), gpui_window, cx)
        });
        tab
    })
    .unwrap()
}

#[gpui_kit::test]
fn disabling_a_plugin_whose_panel_holds_focus_moves_focus_to_the_central_item(
    cx: &mut TestAppContext,
) {
    let (first, _, settings) = two_windows(cx, &[], None);
    activate(cx, &first);
    let tab = add_tab(cx, &first);
    // PANEL_A shows from the start; opening PANEL_B moves focus into it.
    open(cx, &first, PANEL_B);
    let panel = first.made.borrow()[1].clone();
    cx.update_window(first.handle, |_, window, cx| {
        assert!(panel.upgrade().unwrap().read(cx).is_focused(window));
    })
    .unwrap();

    write_disabled(cx, &settings, &["test"]);

    assert_eq!(alive(&first), 0);
    cx.update_window(first.handle, |_, window, cx| {
        assert!(tab.focus_handle(cx).is_focused(window));
    })
    .unwrap();
}

#[gpui_kit::test]
fn disabling_a_plugin_leaves_focus_alone_when_it_was_outside_the_panel(cx: &mut TestAppContext) {
    let (first, _, settings) = two_windows(cx, &[], None);
    activate(cx, &first);
    add_tab(cx, &first);
    open(cx, &first, PANEL_A);
    let elsewhere = cx
        .update_window(first.handle, |_, window, cx| {
            let handle = cx.focus_handle();
            window.focus(&handle, cx);
            handle
        })
        .unwrap();

    write_disabled(cx, &settings, &["test"]);

    assert!(!is_open(cx, &first, PANEL_A));
    cx.update_window(first.handle, |_, window, _| {
        assert!(
            elsewhere.is_focused(window),
            "the focus stayed where it was"
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn enabling_a_plugin_again_creates_its_panels_closed(cx: &mut TestAppContext) {
    let (first, second, settings) = two_windows(cx, &[], None);
    open(cx, &first, PANEL_A);
    write_disabled(cx, &settings, &["test"]);

    write_disabled(cx, &settings, &[]);

    for window in [&first, &second] {
        assert_eq!(alive(window), 2, "new panels, the old ones are gone");
        assert_eq!(window.made.borrow().len(), 4, "two panels, created twice");
        assert!(!is_open(cx, window, PANEL_A), "created closed");
        assert!(!is_open(cx, window, PANEL_B));
        assert_eq!(listed(cx, window), 2);
        cx.update(|cx| {
            let workspace = window.workspace.read(cx);
            assert!(workspace.panel_is_enabled(&id(PANEL_A)));
            assert!(workspace.panel_is_available(&id(PANEL_A)));
        });
    }
    open(cx, &first, PANEL_A);
    assert!(is_open(cx, &first, PANEL_A), "the new panel opens");
}

#[gpui_kit::test]
fn a_window_composed_while_the_plugin_is_disabled_creates_nothing_until_it_is_enabled(
    cx: &mut TestAppContext,
) {
    let (first, _, settings) = two_windows(cx, &["test"], None);
    assert_eq!(alive(&first), 0);
    assert_eq!(first.made.borrow().len(), 0, "no factory ran");

    write_disabled(cx, &settings, &[]);

    assert_eq!(alive(&first), 2);
}

#[gpui_kit::test]
fn enabling_a_plugin_does_not_apply_the_layouts_open_set(cx: &mut TestAppContext) {
    let layouts: LayoutSettings = serde_json::from_value(serde_json::json!({
        "active": "p1",
        "presets": [{
            "id": "p1",
            "name": "Reading",
            "open": ["test.panel-b"],
            "bar": { "left": [{
                "group": "navigation",
                "primary": true,
                "members": ["test.panel-a", "test.panel-b"],
            }] },
        }],
    }))
    .unwrap();
    let (first, _, settings) = two_windows(cx, &["test"], Some(layouts));
    assert!(!is_open(cx, &first, PANEL_B));

    write_disabled(cx, &settings, &[]);

    assert!(!is_open(cx, &first, PANEL_B), "an edit is not a switch");
    open(cx, &first, PANEL_B);
    assert!(is_open(cx, &first, PANEL_B));
}

#[gpui_kit::test]
fn opening_a_disabled_panel_says_so_and_opens_nothing(cx: &mut TestAppContext) {
    let (first, _, _) = two_windows(cx, &["test"], None);

    open(cx, &first, PANEL_A);
    run(cx, &first, |workspace, window, cx| {
        workspace.toggle_panel(&id(PANEL_B), window, cx)
    });

    assert!(!is_open(cx, &first, PANEL_A));
    assert!(!is_open(cx, &first, PANEL_B));
    cx.update_window(first.handle, |_, window, cx| {
        assert_eq!(notice::count(window, cx), 1, "one notice, repeated");
        notice::remove(window, cx, "plugin-disabled");
        assert_eq!(notice::count(window, cx), 0, "it carries the key");
    })
    .unwrap();
}

#[gpui_kit::test]
fn opening_an_enabled_panel_posts_no_notice(cx: &mut TestAppContext) {
    let (first, _, _) = two_windows(cx, &[], None);

    open(cx, &first, PANEL_A);

    assert!(is_open(cx, &first, PANEL_A));
    cx.update_window(first.handle, |_, window, cx| {
        assert_eq!(notice::count(window, cx), 0);
    })
    .unwrap();
}

#[gpui_kit::test]
fn core_required_and_unknown_ids_in_the_settings_disable_nothing(cx: &mut TestAppContext) {
    let mut document = SettingsDocument::default();
    document.register::<PluginEnablement>();
    document.set(enablement(&["shell", "test-required", "gone"]));
    let (handle, workspace, _) = fixture_with_deps(cx, move |_, cx| {
        let settings = SettingsConfig::in_memory(document).install(cx);
        deps_for_core(&[TEST_CORE], &[&TEST_PLUGIN, &REQUIRED_PLUGIN], cx).with_settings(settings)
    });
    let made = register_panels(cx, handle, &workspace, &[CORE_PANEL, REQUIRED_EXPANDABLE]);
    let window = Shell {
        handle,
        workspace,
        made,
    };

    assert_eq!(alive(&window), 2);
    cx.update(|cx| {
        let workspace = window.workspace.read(cx);
        assert!(workspace.panel_is_enabled(&id(CORE_PANEL)));
        assert!(workspace.panel_is_enabled(&id(REQUIRED_EXPANDABLE)));
    });
    open(cx, &window, REQUIRED_EXPANDABLE);
    assert!(is_open(cx, &window, REQUIRED_EXPANDABLE));
}

struct Marker;
impl Render for Marker {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

#[gpui_kit::test]
fn a_disabled_widget_is_never_created_and_a_disabled_one_is_dropped(cx: &mut TestAppContext) {
    let (first, _, settings) = two_windows(cx, &["test"], None);
    let views: Rc<RefCell<Vec<WeakEntity<Marker>>>> = Rc::default();
    let record = views.clone();
    run(cx, &first, |workspace, window, cx| {
        let registration = WidgetRegistration::new(WIDGET, move |_, _, cx| {
            let view = cx.new(|_| Marker);
            record.borrow_mut().push(view.downgrade());
            view
        });
        workspace
            .register_widget(registration, window, cx)
            .expect("the widget is in the test catalog");
    });
    assert!(views.borrow().is_empty(), "the factory never ran");

    write_disabled(cx, &settings, &[]);
    let live = |views: &Rc<RefCell<Vec<WeakEntity<Marker>>>>| {
        views
            .borrow()
            .iter()
            .filter(|view| view.upgrade().is_some())
            .count()
    };
    assert_eq!(live(&views), 1, "enabling places the widget");

    write_disabled(cx, &settings, &["test"]);
    assert_eq!(live(&views), 0, "disabling drops the view");
}
