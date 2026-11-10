//! Toolbars along the edges of the tab area: created while shown, under the tab bar of the only
//! tab group, beside the tabs on a side, and gone when the layout hides them.
use super::*;
use nocterm_plugin_model::{AutoPlace, Edge, Icon, Placement, SettingsSchema, Zone};
use nocterm_window_layout::{EditOp, EditTarget};

use crate::workspace::{ToolbarRegistration, tests::probe};

const fn toolbar(id: &'static str, edge: Edge, order: u16, auto: AutoPlace) -> SystemContribution {
    SystemContribution {
        id: ContributionId::new(id),
        kind: ContributionKind::Toolbar,
        title: "Bar",
        icon: Icon::Server,
        placement: Placement {
            zone: Zone::Left,
            order,
            group: None,
            auto,
        },
        panel: None,
        edge: Some(edge),
        preview: None,
        pinned: false,
    }
}

static BARS: SystemPlugin = SystemPlugin {
    id: nocterm_plugin_model::PluginId::new("bars"),
    name: "Bars",
    description: "Toolbars for registration tests",
    required: false,
    contributions: &[
        toolbar("bars.top", Edge::Top, 10, AutoPlace::Show),
        toolbar("bars.left", Edge::Left, 10, AutoPlace::Show),
        toolbar("bars.tray", Edge::Bottom, 10, AutoPlace::Tray),
    ],
    settings_section: None,
    settings: SettingsSchema::NONE,
    settings_page: None,
};
static TOP: &SystemContribution = &BARS.contributions[0];
static LEFT: &SystemContribution = &BARS.contributions[1];
static TRAY: &SystemContribution = &BARS.contributions[2];

struct Marker(&'static str);
impl Render for Marker {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().id(self.0).test_support().size_full().child(self.0)
    }
}

fn marker(
    name: &'static str,
    created: Rc<Cell<usize>>,
) -> impl Fn(&Workspace, &mut Window, &mut Context<Workspace>) -> Entity<Marker> + 'static {
    move |_, _, cx| {
        created.set(created.get() + 1);
        cx.new(|_| Marker(name))
    }
}

fn bars(cx: &mut TestAppContext) -> (AnyWindowHandle, Entity<Workspace>) {
    window(cx, &[], &[&BARS])
}

fn register_bar(
    cx: &mut TestAppContext,
    handle: AnyWindowHandle,
    workspace: &Entity<Workspace>,
    contribution: &'static SystemContribution,
    name: &'static str,
    created: &Rc<Cell<usize>>,
) -> Result<(), RegistrationError> {
    let registration = ToolbarRegistration::new(contribution, marker(name, created.clone()));
    run(cx, handle, workspace, |workspace, window, cx| {
        window.resize(size(px(1200.), px(700.)));
        workspace.register_toolbar(registration, window, cx)
    })
}

fn add_tab(cx: &mut TestAppContext, handle: AnyWindowHandle, workspace: &Entity<Workspace>) {
    run(cx, handle, workspace, |workspace, window, cx| {
        let tab = probe(cx, Rc::default());
        workspace.add_item(tab, window, cx);
    });
}

#[gpui_kit::test]
fn a_top_toolbar_is_drawn_under_the_tab_bar_of_the_only_tab_group(cx: &mut TestAppContext) {
    let (handle, workspace) = bars(cx);
    let created = Rc::new(Cell::new(0));
    add_tab(cx, handle, &workspace);
    assert_eq!(
        register_bar(cx, handle, &workspace, TOP, "top-marker", &created),
        Ok(())
    );
    assert_eq!(created.get(), 1);
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let content = window.find("workspace-content").bounds();
        let bar = window.find("top-marker").bounds();
        assert!(
            bar.top() > content.top(),
            "the tab bar sits above the toolbar: {bar:?} in {content:?}"
        );
        assert!(bar.bottom() < content.bottom());
        assert!(
            window.try_find("workspace-tab-area").is_none(),
            "the dock draws it, not the chrome"
        );
    })
    .unwrap();
    assert_eq!(created.get(), 1, "rendering does not create the view again");
}

#[gpui_kit::test]
fn without_a_tab_the_top_toolbar_is_drawn_above_the_empty_area(cx: &mut TestAppContext) {
    let (handle, workspace) = bars(cx);
    let created = Rc::new(Cell::new(0));
    assert_eq!(
        register_bar(cx, handle, &workspace, TOP, "top-marker", &created),
        Ok(())
    );
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let area = window.find("workspace-tab-area").bounds();
        let bar = window.find("top-marker").bounds();
        assert!(area.contains(&bar.origin), "{bar:?} in {area:?}");
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_side_toolbar_is_drawn_beside_the_tabs(cx: &mut TestAppContext) {
    let (handle, workspace) = bars(cx);
    let created = Rc::new(Cell::new(0));
    add_tab(cx, handle, &workspace);
    assert_eq!(
        register_bar(cx, handle, &workspace, LEFT, "left-marker", &created),
        Ok(())
    );
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let content = window.find("workspace-content").bounds();
        let bar = window.find("left-marker").bounds();
        assert!(bar.right() <= content.left(), "{bar:?} beside {content:?}");
        assert!(bar.size.width <= px(ToolbarRegistration::WIDTH));
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_toolbar_left_in_the_tray_is_registered_but_never_created(cx: &mut TestAppContext) {
    let (handle, workspace) = bars(cx);
    let created = Rc::new(Cell::new(0));
    assert_eq!(
        register_bar(cx, handle, &workspace, TRAY, "tray-marker", &created),
        Ok(())
    );
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("tray-marker").is_none());
    })
    .unwrap();
    assert_eq!(created.get(), 0);
    cx.update(|cx| assert!(workspace.read(cx).is_registered(&TRAY.id)));
}

#[gpui_kit::test]
fn only_a_toolbar_of_the_catalog_registers_as_a_toolbar(cx: &mut TestAppContext) {
    let (handle, workspace) = window(cx, &[], &[&BARS, &TEST_PLUGIN]);
    let created = Rc::new(Cell::new(0));
    assert_eq!(
        register_bar(cx, handle, &workspace, WIDGET, "widget", &created),
        Err(RegistrationError::KindMismatch {
            id: WIDGET.id.clone(),
            expected: ContributionKind::Widget,
        })
    );
    assert_eq!(
        register_bar(cx, handle, &workspace, TOP, "top-marker", &created),
        Ok(())
    );
    assert_eq!(
        register_bar(cx, handle, &workspace, TOP, "again", &created),
        Err(RegistrationError::Duplicate(TOP.id.clone()))
    );
    assert_eq!(created.get(), 1, "a refused toolbar is not created");
}

#[gpui_kit::test]
fn moving_a_toolbar_to_another_edge_and_hiding_it_follow_the_layout(cx: &mut TestAppContext) {
    let (handle, workspace) = bars(cx);
    let created = Rc::new(Cell::new(0));
    add_tab(cx, handle, &workspace);
    assert_eq!(
        register_bar(cx, handle, &workspace, TOP, "top-marker", &created),
        Ok(())
    );
    let edit = |cx: &mut TestAppContext, op: EditOp| {
        let task = run(cx, handle, &workspace, |workspace, _, cx| {
            workspace.shell.edit(EditTarget::Selected, op, cx)
        });
        cx.run_until_parked();
        drop(task);
    };
    edit(
        cx,
        EditOp::ShowToolbar {
            id: TOP.id.clone(),
            edge: Edge::Bottom,
        },
    );
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let content = window.find("workspace-content").bounds();
        let bar = window.find("top-marker").bounds();
        assert!(bar.top() >= content.bottom(), "{bar:?} under {content:?}");
    })
    .unwrap();
    assert_eq!(created.get(), 1, "a moved toolbar keeps its view");

    edit(cx, EditOp::RemoveToTray { id: TOP.id.clone() });
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("top-marker").is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_toolbar_turned_onto_a_side_is_drawn_anew_and_its_observers_hear_of_it(
    cx: &mut TestAppContext,
) {
    let (handle, workspace) = bars(cx);
    let created = Rc::new(Cell::new(0));
    add_tab(cx, handle, &workspace);
    register_bar(cx, handle, &workspace, TOP, "top-marker", &created).unwrap();
    let heard = Rc::new(Cell::new(0));
    let _listening = cx.update(|cx| {
        let heard = heard.clone();
        cx.subscribe(&workspace, move |_, event: &crate::WorkspaceEvent, _| {
            if *event == crate::WorkspaceEvent::ToolbarsChanged {
                heard.set(heard.get() + 1);
            }
        })
    });
    let task = run(cx, handle, &workspace, |workspace, _, cx| {
        let op = EditOp::ShowToolbar {
            id: TOP.id.clone(),
            edge: Edge::Right,
        };
        workspace.shell.edit(EditTarget::Selected, op, cx)
    });
    cx.run_until_parked();
    drop(task);

    assert_eq!(
        created.get(),
        2,
        "a row turned into a column is drawn again"
    );
    assert_eq!(heard.get(), 1);
    assert_eq!(
        cx.read(|cx| workspace.read(cx).toolbar_edge(&TOP.id)),
        Some(Edge::Right)
    );

    // The first move made a layout of the user's own; a move within it is heard too.
    let task = run(cx, handle, &workspace, |workspace, _, cx| {
        let op = EditOp::ShowToolbar {
            id: TOP.id.clone(),
            edge: Edge::Left,
        };
        workspace.shell.edit(EditTarget::Selected, op, cx)
    });
    cx.run_until_parked();
    drop(task);
    assert_eq!(heard.get(), 2, "a move within the user's layout is heard");
    assert_eq!(
        cx.read(|cx| workspace.read(cx).toolbar_edge(&TOP.id)),
        Some(Edge::Left)
    );
}

#[gpui_kit::test]
fn a_toolbar_of_its_plugins_own_tabs_is_listed_and_placed_but_never_drawn_by_the_window(
    cx: &mut TestAppContext,
) {
    let (handle, workspace) = bars(cx);
    add_tab(cx, handle, &workspace);
    run(cx, handle, &workspace, |workspace, window, cx| {
        workspace.register_toolbar(ToolbarRegistration::own_tabs(&TOP.id), window, cx)
    })
    .expect("a toolbar registers without a view");

    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("workspace-toolbars-top").is_none());
    })
    .unwrap();
    assert_eq!(
        cx.read(|cx| workspace.read(cx).toolbar_edge(&TOP.id)),
        Some(Edge::Top),
        "the tabs that draw it learn its edge from the window"
    );
}
