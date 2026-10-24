//! Panels and widgets register against the catalog the window was composed with.
use std::{cell::Cell, cell::RefCell, rc::Rc};

use gpui_kit::{
    AnyWindowHandle, AppContext as _, Context, Entity, IntoElement, Render, Subscription,
    TestAppContext, TestSupportExt as _, Window, div, prelude::*, px, size,
    test::TestWindowExt as _,
};
use nocterm_plugin_model::{ContributionId, ContributionKind, SystemContribution, SystemPlugin};

use super::{PanelRegistration, PanelRequest, RegistrationError, WidgetRegistration};
use crate::{
    Panel, Workspace, WorkspaceEvent,
    plugin::{LOCAL_TERMINAL, NOTIFICATIONS},
    test_support::{
        CORE_PANEL, EXPANDABLE, PANEL_A, PANEL_B, REQUIRED_EXPANDABLE, REQUIRED_PLUGIN,
        RIGHT_WIDGET, SIDEBAR_EXPANDABLE, TEST_CORE, TEST_PLUGIN, TRAY_PANEL, TRAY_WIDGET,
        TestPanel, Toggle, WIDGET, deps_for_core, toggle_id, toggle_presentation,
    },
    workspace::tests::{fixture, fixture_with_deps},
};

/// A window whose catalog lists the shell's contributions, `core` and `plugins`.
fn window(
    cx: &mut TestAppContext,
    core: &[&'static [SystemContribution]],
    plugins: &[&'static SystemPlugin],
) -> (AnyWindowHandle, Entity<Workspace>) {
    let (handle, workspace, _) = fixture_with_deps(cx, |_, cx| deps_for_core(core, plugins, cx));
    (handle, workspace)
}

fn with_everything(cx: &mut TestAppContext) -> (AnyWindowHandle, Entity<Workspace>) {
    window(cx, &[TEST_CORE], &[&TEST_PLUGIN, &REQUIRED_PLUGIN])
}

fn panel_by(
    title: &'static str,
) -> impl Fn(&Workspace, &mut Window, &mut Context<Workspace>) -> Entity<TestPanel> + 'static {
    move |_, _, cx| TestPanel::builder(title).title(title).build(cx)
}

/// What a registration returned, from inside the window.
fn run<R>(
    cx: &mut TestAppContext,
    handle: AnyWindowHandle,
    workspace: &Entity<Workspace>,
    f: impl FnOnce(&mut Workspace, &mut Window, &mut Context<Workspace>) -> R,
) -> R {
    cx.update_window(handle, |_, window, cx| {
        workspace.update(cx, |workspace, cx| f(workspace, window, cx))
    })
    .unwrap()
}

fn register<T: Panel>(
    cx: &mut TestAppContext,
    handle: AnyWindowHandle,
    workspace: &Entity<Workspace>,
    registration: PanelRegistration<T>,
) -> Result<(), RegistrationError> {
    run(cx, handle, workspace, |workspace, window, cx| {
        workspace.register_panel(registration, window, cx)
    })
}

fn id(contribution: &SystemContribution) -> ContributionId {
    contribution.id.clone()
}

#[gpui_kit::test]
fn a_contribution_outside_the_catalog_is_refused(cx: &mut TestAppContext) {
    let (handle, workspace) = window(cx, &[], &[]);
    let refused = register(
        cx,
        handle,
        &workspace,
        PanelRegistration::new(PANEL_A, panel_by("A")),
    );
    assert_eq!(refused, Err(RegistrationError::Unlisted(id(PANEL_A))));
    let refused = run(cx, handle, &workspace, |workspace, window, cx| {
        let widget = WidgetRegistration::new(WIDGET, |_, _, cx| cx.new(|_| Silent));
        workspace.register_widget(widget, window, cx)
    });
    assert_eq!(refused, Err(RegistrationError::Unlisted(id(WIDGET))));
    cx.update(|cx| assert!(!workspace.read(cx).is_registered(&id(PANEL_A))));
}

#[gpui_kit::test]
fn default_dependencies_list_only_the_shells_own_contributions(cx: &mut TestAppContext) {
    let (handle, workspace) = fixture(cx);
    let refused = register(
        cx,
        handle,
        &workspace,
        PanelRegistration::new(PANEL_A, panel_by("A")),
    );
    assert_eq!(refused, Err(RegistrationError::Unlisted(id(PANEL_A))));
    let accepted = run(cx, handle, &workspace, |workspace, window, cx| {
        let widget = WidgetRegistration::new(&NOTIFICATIONS, |_, _, cx| cx.new(|_| Silent));
        workspace.register_widget(widget, window, cx)
    });
    assert_eq!(accepted, Ok(()));
}

#[gpui_kit::test]
fn a_contribution_registers_once(cx: &mut TestAppContext) {
    let (handle, workspace) = with_everything(cx);
    let first = register(
        cx,
        handle,
        &workspace,
        PanelRegistration::new(PANEL_A, panel_by("A")),
    );
    assert_eq!(first, Ok(()));
    cx.update(|cx| assert!(workspace.read(cx).is_registered(&id(PANEL_A))));
    let second = register(
        cx,
        handle,
        &workspace,
        PanelRegistration::new(PANEL_A, panel_by("again")),
    );
    assert_eq!(second, Err(RegistrationError::Duplicate(id(PANEL_A))));
}

#[gpui_kit::test]
fn a_registration_must_match_the_kind_the_catalog_names(cx: &mut TestAppContext) {
    let (handle, workspace) = with_everything(cx);
    let panel_as_widget = run(cx, handle, &workspace, |workspace, window, cx| {
        let widget = WidgetRegistration::new(PANEL_A, |_, _, cx| cx.new(|_| Silent));
        workspace.register_widget(widget, window, cx)
    });
    assert_eq!(
        panel_as_widget,
        Err(RegistrationError::KindMismatch {
            id: id(PANEL_A),
            expected: ContributionKind::Panel,
        })
    );
    let widget_as_panel = register(
        cx,
        handle,
        &workspace,
        PanelRegistration::new(WIDGET, panel_by("W")),
    );
    assert_eq!(
        widget_as_panel,
        Err(RegistrationError::KindMismatch {
            id: id(WIDGET),
            expected: ContributionKind::Widget,
        })
    );
    cx.update(|cx| {
        assert!(!workspace.read(cx).is_registered(&id(PANEL_A)));
        assert!(!workspace.read(cx).is_registered(&id(WIDGET)));
    });
}

#[gpui_kit::test]
fn only_a_panel_that_draws_its_own_header_expands(cx: &mut TestAppContext) {
    let (handle, workspace) = with_everything(cx);
    let titled = register(
        cx,
        handle,
        &workspace,
        PanelRegistration::new(PANEL_A, panel_by("A")).expandable(),
    );
    assert_eq!(titled, Err(RegistrationError::NotExpandable(id(PANEL_A))));
    cx.update(|cx| assert!(!workspace.read(cx).is_registered(&id(PANEL_A))));
    let own_header = register(
        cx,
        handle,
        &workspace,
        PanelRegistration::new(EXPANDABLE, panel_by("E")).expandable(),
    );
    assert_eq!(own_header, Ok(()));
}

#[gpui_kit::test]
fn an_existing_panel_is_accepted_for_core_and_required_contributions_only(cx: &mut TestAppContext) {
    let (handle, workspace) = with_everything(cx);
    let panel = |cx: &mut TestAppContext| cx.update(|cx| TestPanel::builder("p").build(cx));
    let existing = panel(cx);
    let optional = register(
        cx,
        handle,
        &workspace,
        PanelRegistration::with_entity(PANEL_A, existing),
    );
    assert_eq!(
        optional,
        Err(RegistrationError::EagerNotRequired(id(PANEL_A)))
    );
    // The refused registration claimed nothing: a factory may still register the panel.
    let by_factory = register(
        cx,
        handle,
        &workspace,
        PanelRegistration::new(PANEL_A, panel_by("A")),
    );
    assert_eq!(by_factory, Ok(()));
    let existing = panel(cx);
    let core = register(
        cx,
        handle,
        &workspace,
        PanelRegistration::with_entity(CORE_PANEL, existing),
    );
    assert_eq!(core, Ok(()));
    let existing = panel(cx);
    let required = register(
        cx,
        handle,
        &workspace,
        PanelRegistration::with_entity(REQUIRED_EXPANDABLE, existing).expandable(),
    );
    assert_eq!(required, Ok(()));
}

#[gpui_kit::test]
fn a_factory_runs_once_at_registration_and_never_again(cx: &mut TestAppContext) {
    let (handle, workspace) = with_everything(cx);
    let created = Rc::new(Cell::new(0));
    let counted = created.clone();
    let registration = PanelRegistration::new(PANEL_A, move |workspace, window, cx| {
        counted.set(counted.get() + 1);
        panel_by("A")(workspace, window, cx)
    });
    assert_eq!(register(cx, handle, &workspace, registration), Ok(()));
    assert_eq!(created.get(), 1);
    run(cx, handle, &workspace, |workspace, window, cx| {
        workspace.toggle_panel(&id(PANEL_A), window, cx);
        workspace.toggle_panel(&id(PANEL_A), window, cx);
        workspace.open_panel(&id(PANEL_A), window, cx);
        workspace.set_panel_available(&id(PANEL_A), false, window, cx);
        workspace.set_panel_available(&id(PANEL_A), true, window, cx);
    });
    cx.update_window(handle, |_, window, cx| window.render_frame(cx))
        .unwrap();
    assert_eq!(created.get(), 1);
}

#[gpui_kit::test]
fn sidebar_panels_open_and_toggle_by_id(cx: &mut TestAppContext) {
    let (handle, workspace) = with_everything(cx);
    let panels: Rc<RefCell<Vec<Entity<TestPanel>>>> = Rc::default();
    for contribution in [PANEL_A, PANEL_B] {
        let kept = panels.clone();
        let registration = PanelRegistration::new(contribution, move |_, _, cx| {
            let panel = TestPanel::builder(contribution.title).build(cx);
            kept.borrow_mut().push(panel.clone());
            panel
        });
        assert_eq!(register(cx, handle, &workspace, registration), Ok(()));
    }
    let [first, second] = [panels.borrow()[0].clone(), panels.borrow()[1].clone()];
    let state = |cx: &mut TestAppContext| {
        cx.update(|cx| {
            let workspace = workspace.read(cx);
            (
                workspace.primary_group_is_open(),
                workspace.panel_is_open(&id(PANEL_A)),
                workspace.panel_is_open(&id(PANEL_B)),
            )
        })
    };
    // The sidebar shows its first panel from the start.
    assert_eq!(state(cx), (true, true, false));
    run(cx, handle, &workspace, |workspace, window, cx| {
        workspace.toggle_panel(&id(PANEL_B), window, cx)
    });
    assert_eq!(state(cx), (true, false, true));
    cx.update_window(handle, |_, window, cx| {
        assert!(second.read(cx).is_focused(window));
    })
    .unwrap();
    run(cx, handle, &workspace, |workspace, window, cx| {
        workspace.toggle_panel(&id(PANEL_B), window, cx)
    });
    assert_eq!(state(cx), (false, false, false));
    run(cx, handle, &workspace, |workspace, window, cx| {
        workspace.open_panel(&id(PANEL_A), window, cx)
    });
    assert_eq!(state(cx), (true, true, false));
    cx.update_window(handle, |_, window, cx| {
        assert!(first.read(cx).is_focused(window));
    })
    .unwrap();
}

/// The panel behind an expandable registration, with the window it sits in.
struct Expandable {
    handle: AnyWindowHandle,
    workspace: Entity<Workspace>,
    panel: Entity<TestPanel>,
    events: Rc<RefCell<Vec<WorkspaceEvent>>>,
    _subscription: Subscription,
}

fn expandable(cx: &mut TestAppContext) -> Expandable {
    let (handle, workspace) = with_everything(cx);
    let events: Rc<RefCell<Vec<WorkspaceEvent>>> = Rc::default();
    let log = events.clone();
    let subscription = cx.update(|cx| {
        cx.subscribe(&workspace, move |_, event: &WorkspaceEvent, _| {
            log.borrow_mut().push(*event)
        })
    });
    let kept: Rc<RefCell<Option<Entity<TestPanel>>>> = Rc::default();
    let slot = kept.clone();
    let registration = PanelRegistration::new(EXPANDABLE, move |_, _, cx| {
        let panel = TestPanel::builder("expandable")
            .title("Expandable")
            .build(cx);
        *slot.borrow_mut() = Some(panel.clone());
        panel
    })
    .expandable();
    run(cx, handle, &workspace, |workspace, window, cx| {
        window.resize(size(px(1400.), px(700.)));
        workspace
            .register_panel(registration, window, cx)
            .expect("the panel is in the catalog");
    });
    let panel = kept.borrow().clone().expect("the factory ran");
    Expandable {
        handle,
        workspace,
        panel,
        events,
        _subscription: subscription,
    }
}

#[gpui_kit::test]
fn a_side_panel_opens_expands_and_withdraws_by_id(cx: &mut TestAppContext) {
    let shell = expandable(cx);
    let (handle, workspace) = (shell.handle, &shell.workspace);
    let state = |cx: &mut TestAppContext| {
        cx.update(|cx| {
            let workspace = workspace.read(cx);
            [
                workspace.panel_is_available(&id(EXPANDABLE)),
                workspace.panel_is_open(&id(EXPANDABLE)),
                workspace.panel_is_expanded(&id(EXPANDABLE)),
            ]
        })
    };
    assert_eq!(state(cx), [true, false, false]);
    run(cx, handle, workspace, |workspace, window, cx| {
        workspace.toggle_panel(&id(EXPANDABLE), window, cx)
    });
    assert_eq!(state(cx), [true, true, false]);
    cx.update_window(handle, |_, window, cx| {
        assert!(shell.panel.read(cx).is_focused(window));
        assert!(workspace.read(cx).right_panel_is_open());
    })
    .unwrap();
    run(cx, handle, workspace, |workspace, _, cx| {
        workspace.set_panel_expanded(&id(EXPANDABLE), true, cx)
    });
    assert_eq!(state(cx), [true, true, true]);
    cx.update(|cx| assert!(shell.panel.read(cx).maximized()));
    run(cx, handle, workspace, |workspace, window, cx| {
        workspace.set_panel_available(&id(EXPANDABLE), false, window, cx)
    });
    assert_eq!(state(cx), [false, false, false]);
    cx.update(|cx| assert!(!shell.panel.read(cx).maximized()));
}

#[gpui_kit::test]
fn an_expandable_panel_asks_its_group_to_expand_and_close(cx: &mut TestAppContext) {
    let shell = expandable(cx);
    let (handle, workspace) = (shell.handle, &shell.workspace);
    run(cx, handle, workspace, |workspace, window, cx| {
        workspace.open_panel(&id(EXPANDABLE), window, cx)
    });
    let request = |cx: &mut TestAppContext, request| {
        cx.update(|cx| {
            shell
                .panel
                .update(cx, |panel, cx| panel.request(request, cx))
        })
    };
    let expanded = |cx: &mut TestAppContext| {
        cx.update(|cx| workspace.read(cx).panel_is_expanded(&id(EXPANDABLE)))
    };
    request(cx, PanelRequest::ToggleExpanded);
    assert!(expanded(cx));
    request(cx, PanelRequest::ToggleExpanded);
    assert!(!expanded(cx));
    request(cx, PanelRequest::ToggleExpanded);
    request(cx, PanelRequest::Close);
    cx.update(|cx| {
        assert!(!workspace.read(cx).panel_is_open(&id(EXPANDABLE)));
        assert!(!shell.panel.read(cx).maximized());
    });
}

#[gpui_kit::test]
fn an_expandable_panel_learns_the_side_it_is_on(cx: &mut TestAppContext) {
    let shell = expandable(cx);
    cx.update(|cx| assert!(!shell.panel.read(cx).docked_left()));
    cx.update(|cx| {
        shell
            .workspace
            .update(cx, |workspace, cx| workspace.swap_sides(cx))
    });
    cx.update(|cx| assert!(shell.panel.read(cx).docked_left()));
}

/// Registers an expandable panel for `contribution` and returns the panel the factory made.
fn register_expandable(
    cx: &mut TestAppContext,
    handle: AnyWindowHandle,
    workspace: &Entity<Workspace>,
    contribution: &'static SystemContribution,
) -> Entity<TestPanel> {
    let kept: Rc<RefCell<Option<Entity<TestPanel>>>> = Rc::default();
    let slot = kept.clone();
    let registration = PanelRegistration::new(contribution, move |_, _, cx| {
        let panel = TestPanel::builder(contribution.title).build(cx);
        *slot.borrow_mut() = Some(panel.clone());
        panel
    })
    .expandable();
    assert_eq!(register(cx, handle, workspace, registration), Ok(()));
    kept.take().expect("the factory ran")
}

#[gpui_kit::test]
fn each_expandable_panel_learns_the_side_of_its_own_group(cx: &mut TestAppContext) {
    let (handle, workspace) = with_everything(cx);
    let assistant = register_expandable(cx, handle, &workspace, EXPANDABLE);
    let sidebar = register_expandable(cx, handle, &workspace, SIDEBAR_EXPANDABLE);
    let docked_left = |cx: &mut TestAppContext| {
        cx.update(|cx| {
            (
                assistant.read(cx).docked_left(),
                sidebar.read(cx).docked_left(),
            )
        })
    };
    assert_eq!(
        docked_left(cx),
        (false, true),
        "the sidebar opens on the left"
    );
    cx.update(|cx| workspace.update(cx, |workspace, cx| workspace.swap_sides(cx)));
    assert_eq!(docked_left(cx), (true, false));
    // A panel registered while the sides are swapped starts on its group's swapped side.
    let late = register_expandable(cx, handle, &workspace, REQUIRED_EXPANDABLE);
    cx.update(|cx| assert!(late.read(cx).docked_left()));
}

#[gpui_kit::test]
fn a_factory_reads_the_workspace_it_is_registered_in(cx: &mut TestAppContext) {
    let (handle, workspace) = with_everything(cx);
    let first = register(
        cx,
        handle,
        &workspace,
        PanelRegistration::new(PANEL_A, panel_by("A")),
    );
    assert_eq!(first, Ok(()));
    let panel_view = Rc::new(RefCell::new(Vec::new()));
    let log = panel_view.clone();
    let second = register(
        cx,
        handle,
        &workspace,
        PanelRegistration::new(PANEL_B, move |workspace, _, cx| {
            log.borrow_mut().push((
                workspace.is_registered(&id(PANEL_A)),
                workspace.panel_is_open(&id(PANEL_A)),
                workspace.panel_is_available(&id(PANEL_B)),
            ));
            TestPanel::builder("B").build(cx)
        }),
    );
    assert_eq!(second, Ok(()));
    // The first panel shows; the second is registered but not offered while it is made.
    assert_eq!(*panel_view.borrow(), [(true, true, false)]);
    let widget_view = Rc::new(Cell::new(None));
    let log = widget_view.clone();
    let widget = run(cx, handle, &workspace, |workspace, window, cx| {
        let registration = WidgetRegistration::new(WIDGET, move |workspace, _, cx| {
            log.set(Some(workspace.panel_is_open(&id(PANEL_B))));
            cx.new(|_| Silent)
        });
        workspace.register_widget(registration, window, cx)
    });
    assert_eq!(widget, Ok(()));
    assert_eq!(widget_view.get(), Some(false));
}

#[gpui_kit::test]
fn a_panel_that_asks_for_the_user_marks_its_toggle_until_it_stops(cx: &mut TestAppContext) {
    let shell = expandable(cx);
    let look = |cx: &mut TestAppContext| {
        cx.update(|cx| toggle_presentation(shell.workspace.read(cx), Toggle::SidePanel, cx))
    };
    let (quiet, asking) = (look(cx), {
        cx.update(|cx| {
            shell.panel.update(cx, |panel, cx| {
                panel.set_attention(Some("Permission".into()), cx)
            })
        });
        look(cx)
    });
    assert_ne!(quiet, asking, "the toggle shows the request");
    cx.update(|cx| {
        shell
            .panel
            .update(cx, |panel, cx| panel.set_attention(None, cx))
    });
    assert_eq!(look(cx), quiet);
}

#[gpui_kit::test]
fn each_group_that_changes_visibility_is_announced(cx: &mut TestAppContext) {
    let shell = expandable(cx);
    let (handle, workspace) = (shell.handle, &shell.workspace);
    shell.events.borrow_mut().clear();
    run(cx, handle, workspace, |workspace, window, cx| {
        workspace.open_panel(&id(EXPANDABLE), window, cx)
    });
    assert_eq!(
        *shell.events.borrow(),
        [
            WorkspaceEvent::RightPanelVisibilityChanged,
            WorkspaceEvent::PanelVisibilityChanged
        ]
    );
    shell.events.borrow_mut().clear();
    run(cx, handle, workspace, |workspace, window, cx| {
        workspace.toggle_primary_group(window, cx)
    });
    assert_eq!(
        *shell.events.borrow(),
        [WorkspaceEvent::PanelVisibilityChanged]
    );
    shell.events.borrow_mut().clear();
    run(cx, handle, workspace, |workspace, window, cx| {
        // Already showing: nothing changes.
        workspace.open_panel(&id(EXPANDABLE), window, cx)
    });
    assert!(shell.events.borrow().is_empty());
}

#[gpui_kit::test]
fn an_unknown_id_is_never_open_and_changes_nothing(cx: &mut TestAppContext) {
    let shell = expandable(cx);
    let (handle, workspace) = (shell.handle, &shell.workspace);
    let unknown = id(PANEL_A);
    shell.events.borrow_mut().clear();
    run(cx, handle, workspace, |workspace, window, cx| {
        workspace.toggle_panel(&unknown, window, cx);
        workspace.open_panel(&unknown, window, cx);
        workspace.set_panel_available(&unknown, false, window, cx);
        workspace.set_panel_expanded(&unknown, true, cx);
    });
    cx.update(|cx| {
        let workspace = workspace.read(cx);
        assert!(!workspace.panel_is_open(&unknown));
        assert!(!workspace.panel_is_available(&unknown));
        assert!(!workspace.panel_is_expanded(&unknown));
    });
    assert!(shell.events.borrow().is_empty());
}

struct Silent;
impl Render for Silent {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

#[path = "registration_tests/widgets.rs"]
mod widgets;
