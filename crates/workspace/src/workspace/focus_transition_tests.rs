//! Today's sidebar and side-panel transitions: visibility, expansion, availability,
//! and where focus lands, one test per behaviour of the transition table.
use crate::test_support::{EXPANDABLE, PANEL_A, PANEL_B, Toggle, toggle_id};
use gpui_kit::{
    AnyWindowHandle, App, AppContext as _, Entity, Focusable, Subscription, TestAppContext, Window,
    point, px, size, test::TestWindowExt as _,
};
use std::{cell::Cell, rc::Rc};

use crate::test_support::TestPanel;
use crate::workspace::tests::test_plugin::{fixture_with_test_plugin, register_built};
use crate::workspace::tests::{Probe, probe};
use crate::{NextPanel, PanelRequest, ToggleRightPanelMaximized, Workspace, WorkspaceEvent};
use nocterm_plugin_model::{ContributionId, SystemContribution};

/// Where the window's focus currently is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Focus {
    Navigation(usize),
    SidePanel,
    Central,
    Elsewhere,
}

/// A window with two sidebar panels, an available-on-demand side panel and one tab.
struct Shell {
    handle: AnyWindowHandle,
    workspace: Entity<Workspace>,
    layout: nocterm_ui::LayoutHandle,
    navigation: [Entity<TestPanel>; 2],
    side: Entity<TestPanel>,
    central: Entity<Probe>,
    visibility_events: Rc<Cell<usize>>,
    _events: Subscription,
}

fn id(contribution: &SystemContribution) -> ContributionId {
    contribution.id.clone()
}

fn shell(cx: &mut TestAppContext) -> Shell {
    let (handle, workspace, layout) = fixture_with_test_plugin(cx);
    let visibility_events = Rc::new(Cell::new(0));
    let seen = visibility_events.clone();
    let events = cx.update(|cx| {
        cx.subscribe(&workspace, move |_, event, _| {
            if *event == WorkspaceEvent::PanelVisibilityChanged {
                seen.set(seen.get() + 1);
            }
        })
    });
    let (navigation, side, central) = cx
        .update_window(handle, |_, window, cx| {
            window.resize(size(px(1400.), px(700.)));
            let navigation = [
                TestPanel::builder("navigation-0").title("Files").build(cx),
                TestPanel::builder("navigation-1")
                    .title("Servers")
                    .build(cx),
            ];
            let side = TestPanel::builder("side-panel")
                .title("AI Agents")
                .build(cx);
            let central = probe(cx, Rc::default());
            workspace.update(cx, |workspace, cx| {
                register_built(workspace, PANEL_A, &navigation[0], window, cx);
                register_built(workspace, PANEL_B, &navigation[1], window, cx);
                register_built(workspace, EXPANDABLE, &side, window, cx);
                // The host offers the side panel on demand.
                workspace.set_panel_available(&id(EXPANDABLE), false, window, cx);
                workspace.add_item(central.clone(), window, cx);
            });
            window.render_frame(cx);
            (navigation, side, central)
        })
        .unwrap();
    Shell {
        handle,
        workspace,
        layout,
        navigation,
        side,
        central,
        visibility_events,
        _events: events,
    }
}

impl Shell {
    fn run<R>(&self, cx: &mut TestAppContext, f: impl FnOnce(&mut Window, &mut App) -> R) -> R {
        let result = cx.update_window(self.handle, |_, window, cx| f(window, cx));
        cx.run_until_parked();
        result.unwrap()
    }

    fn update(
        &self,
        cx: &mut TestAppContext,
        f: impl FnOnce(&mut Workspace, &mut Window, &mut gpui_kit::Context<Workspace>),
    ) {
        self.run(cx, |window, cx| {
            self.workspace
                .update(cx, |workspace, cx| f(workspace, window, cx))
        });
    }

    fn open_side_panel(&self, cx: &mut TestAppContext) {
        self.update(cx, |workspace, window, cx| {
            workspace.set_panel_available(&id(EXPANDABLE), true, window, cx);
            workspace.toggle_panel(&id(EXPANDABLE), window, cx);
        });
    }

    fn focus_central(&self, cx: &mut TestAppContext) {
        self.run(cx, |window, cx| {
            window.focus(&self.central.focus_handle(cx), cx)
        });
    }

    fn focus_navigation(&self, ix: usize, cx: &mut TestAppContext) {
        self.update(cx, |workspace, window, cx| {
            workspace.activate_panel(ix, window, cx)
        });
    }

    fn focus(&self, cx: &mut TestAppContext) -> Focus {
        self.run(cx, |window, cx| {
            if self.side.focus_handle(cx).contains_focused(window, cx) {
                return Focus::SidePanel;
            }
            if self.central.focus_handle(cx).is_focused(window) {
                return Focus::Central;
            }
            (0..2)
                .find(|&ix| self.navigation[ix].focus_handle(cx).is_focused(window))
                .map_or(Focus::Elsewhere, Focus::Navigation)
        })
    }

    fn sidebar_open(&self, cx: &TestAppContext) -> bool {
        self.workspace
            .read_with(cx, |w, _| w.primary_group_is_open())
    }

    fn side_open(&self, cx: &TestAppContext) -> bool {
        self.workspace
            .read_with(cx, |w, _| w.panel_is_open(&id(EXPANDABLE)))
    }

    fn side_maximized(&self, cx: &TestAppContext) -> bool {
        self.workspace
            .read_with(cx, |w, _| w.panel_is_expanded(&id(EXPANDABLE)))
    }

    /// Which sidebar panel is rendered, if the sidebar is showing.
    fn shown_navigation(&self, cx: &mut TestAppContext) -> Option<usize> {
        self.run(cx, |window, cx| {
            window.render_frame(cx);
            (0..2).find(|ix| window.try_find(format!("navigation-{ix}")).is_some())
        })
    }
}

#[gpui_kit::test]
fn a_new_window_shows_an_empty_sidebar_and_keeps_the_unoffered_side_panel_away(
    cx: &mut TestAppContext,
) {
    let (handle, workspace, _) = fixture_with_test_plugin(cx);
    cx.update_window(handle, |_, window, cx| {
        let side = TestPanel::builder("side-panel").build(cx);
        workspace.update(cx, |workspace, cx| {
            assert!(workspace.primary_group_is_open());
            assert!(!workspace.is_registered(&id(EXPANDABLE)));
            register_built(workspace, EXPANDABLE, &side, window, cx);
            workspace.set_panel_available(&id(EXPANDABLE), false, window, cx);
            assert!(
                !workspace.panel_is_available(&id(EXPANDABLE))
                    && !workspace.panel_is_open(&id(EXPANDABLE))
            );
            workspace.set_panel_available(&id(EXPANDABLE), true, window, cx);
            assert!(
                workspace.panel_is_available(&id(EXPANDABLE))
                    && !workspace.panel_is_open(&id(EXPANDABLE))
            );
        });
    })
    .unwrap();
}

#[gpui_kit::test]
fn showing_a_hidden_sidebar_keeps_its_selection_and_the_focus(cx: &mut TestAppContext) {
    let shell = shell(cx);
    shell.focus_navigation(1, cx);
    shell.update(cx, |w, window, cx| w.toggle_sidebar(window, cx));
    assert!(!shell.sidebar_open(cx));
    shell.focus_central(cx);
    shell.update(cx, |w, window, cx| w.toggle_sidebar(window, cx));
    assert!(shell.sidebar_open(cx));
    assert_eq!(shell.shown_navigation(cx), Some(1));
    assert_eq!(shell.focus(cx), Focus::Central);
    assert_eq!(
        shell.visibility_events.get(),
        2,
        "hiding and showing the sidebar"
    );
}

#[gpui_kit::test]
fn hiding_the_sidebar_beside_an_expanded_side_panel_moves_focus_only_from_the_sidebar(
    cx: &mut TestAppContext,
) {
    let shell = shell(cx);
    shell.open_side_panel(cx);
    shell.update(cx, |w, _, cx| {
        w.set_panel_expanded(&id(EXPANDABLE), true, cx)
    });
    shell.focus_central(cx);
    shell.update(cx, |w, window, cx| w.toggle_sidebar(window, cx));
    assert!(!shell.sidebar_open(cx));
    assert!(shell.side_maximized(cx));
    assert_eq!(shell.focus(cx), Focus::Central);
    shell.update(cx, |w, window, cx| w.toggle_sidebar(window, cx));
    shell.focus_navigation(0, cx);
    shell.update(cx, |w, window, cx| w.toggle_sidebar(window, cx));
    assert!(!shell.sidebar_open(cx));
    assert_eq!(shell.focus(cx), Focus::SidePanel);
}

#[gpui_kit::test]
fn hiding_the_sidebar_focuses_the_active_tab_only_when_the_sidebar_held_focus(
    cx: &mut TestAppContext,
) {
    let shell = shell(cx);
    shell.open_side_panel(cx);
    assert_eq!(shell.focus(cx), Focus::SidePanel);
    shell.update(cx, |w, window, cx| w.toggle_sidebar(window, cx));
    assert!(!shell.sidebar_open(cx));
    assert!(shell.side_open(cx));
    assert_eq!(shell.focus(cx), Focus::SidePanel);
    shell.update(cx, |w, window, cx| w.toggle_sidebar(window, cx));
    shell.focus_navigation(0, cx);
    shell.update(cx, |w, window, cx| w.toggle_sidebar(window, cx));
    assert_eq!(shell.focus(cx), Focus::Central);
}

#[gpui_kit::test]
fn clicking_the_showing_panel_button_hides_the_sidebar_like_the_toggle(cx: &mut TestAppContext) {
    let shell = shell(cx);
    shell.open_side_panel(cx);
    shell.focus_navigation(0, cx);
    shell.run(cx, |window, cx| {
        window.render_frame(cx);
        window.click(toggle_id(Toggle::Navigation(0)), cx);
    });
    assert!(!shell.sidebar_open(cx));
    assert_eq!(shell.focus(cx), Focus::Central);
    shell.run(cx, |window, cx| {
        window.render_frame(cx);
        window.click(toggle_id(Toggle::Navigation(0)), cx);
    });
    assert!(shell.sidebar_open(cx));
    assert_eq!(shell.focus(cx), Focus::Navigation(0));
    shell.update(cx, |w, _, cx| {
        w.set_panel_expanded(&id(EXPANDABLE), true, cx)
    });
    shell.run(cx, |window, cx| {
        window.render_frame(cx);
        window.click(toggle_id(Toggle::Navigation(0)), cx);
    });
    assert!(!shell.sidebar_open(cx));
    assert_eq!(shell.focus(cx), Focus::SidePanel);
}

#[gpui_kit::test]
fn next_panel_on_a_hidden_sidebar_shows_the_selected_panel_without_advancing(
    cx: &mut TestAppContext,
) {
    let shell = shell(cx);
    shell.focus_navigation(1, cx);
    shell.update(cx, |w, window, cx| w.toggle_sidebar(window, cx));
    shell.run(cx, |window, cx| {
        window.dispatch_action(Box::new(NextPanel), cx)
    });
    assert!(shell.sidebar_open(cx));
    assert_eq!(shell.shown_navigation(cx), Some(1));
    assert_eq!(shell.focus(cx), Focus::Navigation(1));
}

#[gpui_kit::test]
fn next_panel_without_sidebar_panels_does_nothing(cx: &mut TestAppContext) {
    let (handle, workspace, _) = fixture_with_test_plugin(cx);
    cx.update_window(handle, |_, window, cx| {
        let central = probe(cx, Rc::default());
        workspace.update(cx, |w, cx| w.add_item(central.clone(), window, cx));
        window.focus(&central.focus_handle(cx), cx);
        window.dispatch_action(Box::new(NextPanel), cx);
        assert!(workspace.read(cx).primary_group_is_open());
        assert!(central.focus_handle(cx).is_focused(window));
    })
    .unwrap();
}

#[gpui_kit::test]
fn activating_a_missing_sidebar_panel_changes_nothing(cx: &mut TestAppContext) {
    let shell = shell(cx);
    shell.focus_navigation(1, cx);
    shell.focus_central(cx);
    shell.update(cx, |w, window, cx| w.activate_panel(7, window, cx));
    assert!(shell.sidebar_open(cx));
    assert_eq!(shell.shown_navigation(cx), Some(1));
    assert_eq!(shell.focus(cx), Focus::Central);
    shell.update(cx, |w, window, cx| w.toggle_sidebar(window, cx));
    shell.update(cx, |w, window, cx| w.activate_panel(7, window, cx));
    assert!(!shell.sidebar_open(cx));
}

#[gpui_kit::test]
fn toggling_a_panel_that_is_not_registered_does_nothing(cx: &mut TestAppContext) {
    let shell = shell(cx);
    shell.focus_navigation(1, cx);
    shell.focus_central(cx);
    let unknown = ContributionId::new("test.unknown");
    shell.update(cx, |w, window, cx| {
        w.toggle_panel(&unknown, window, cx);
        w.open_panel(&unknown, window, cx);
    });
    assert!(shell.sidebar_open(cx));
    assert_eq!(shell.shown_navigation(cx), Some(1));
    assert_eq!(shell.focus(cx), Focus::Central);
    assert_eq!(shell.visibility_events.get(), 0);
}

#[gpui_kit::test]
fn toggling_the_showing_panel_by_id_hides_the_sidebar_like_its_button(cx: &mut TestAppContext) {
    let shell = shell(cx);
    shell.update(cx, |w, window, cx| w.open_panel(&id(PANEL_B), window, cx));
    assert!(shell.sidebar_open(cx));
    assert_eq!(shell.focus(cx), Focus::Navigation(1));
    shell.update(cx, |w, window, cx| w.toggle_panel(&id(PANEL_B), window, cx));
    assert!(!shell.sidebar_open(cx));
    assert_eq!(shell.focus(cx), Focus::Central);
    shell.update(cx, |w, window, cx| w.toggle_panel(&id(PANEL_B), window, cx));
    assert!(shell.sidebar_open(cx));
    assert_eq!(shell.focus(cx), Focus::Navigation(1));
}

#[gpui_kit::test]
fn closing_the_side_panel_while_it_has_no_focus_leaves_focus_where_it_is(cx: &mut TestAppContext) {
    let shell = shell(cx);
    shell.open_side_panel(cx);
    assert_eq!(shell.visibility_events.get(), 1);
    shell.focus_navigation(1, cx);
    shell.update(cx, |w, window, cx| {
        w.toggle_panel(&id(EXPANDABLE), window, cx)
    });
    assert!(!shell.side_open(cx));
    assert_eq!(shell.visibility_events.get(), 2);
    assert_eq!(shell.focus(cx), Focus::Navigation(1));
}

#[gpui_kit::test]
fn withdrawing_an_open_side_panel_closes_it_and_refocuses_only_if_it_held_focus(
    cx: &mut TestAppContext,
) {
    let shell = shell(cx);
    shell.open_side_panel(cx);
    shell.focus_navigation(0, cx);
    shell.update(cx, |w, window, cx| {
        w.set_panel_available(&id(EXPANDABLE), false, window, cx)
    });
    assert!(
        !shell.side_open(cx)
            && !shell
                .workspace
                .read_with(cx, |w, _| w.panel_is_available(&id(EXPANDABLE)))
    );
    assert_eq!(shell.visibility_events.get(), 2);
    assert_eq!(shell.focus(cx), Focus::Navigation(0));

    shell.update(cx, |w, window, cx| {
        w.set_panel_available(&id(EXPANDABLE), true, window, cx);
        w.toggle_panel(&id(EXPANDABLE), window, cx);
    });
    assert_eq!(shell.focus(cx), Focus::SidePanel);
    shell.update(cx, |w, window, cx| {
        w.set_panel_available(&id(EXPANDABLE), false, window, cx)
    });
    assert_eq!(shell.focus(cx), Focus::Central);
}

#[gpui_kit::test]
fn withdrawing_a_closed_side_panel_is_silent(cx: &mut TestAppContext) {
    let shell = shell(cx);
    shell.update(cx, |w, window, cx| {
        w.set_panel_available(&id(EXPANDABLE), true, window, cx)
    });
    shell.focus_navigation(1, cx);
    shell.update(cx, |w, window, cx| {
        w.set_panel_available(&id(EXPANDABLE), false, window, cx)
    });
    assert!(
        !shell
            .workspace
            .read_with(cx, |w, _| w.panel_is_available(&id(EXPANDABLE)))
    );
    assert_eq!(shell.visibility_events.get(), 0);
    assert_eq!(shell.focus(cx), Focus::Navigation(1));
}

#[gpui_kit::test]
fn a_side_panel_that_is_not_open_cannot_be_expanded(cx: &mut TestAppContext) {
    let shell = shell(cx);
    shell.update(cx, |w, _, cx| {
        w.set_panel_expanded(&id(EXPANDABLE), true, cx)
    });
    assert!(!shell.side_maximized(cx) && !shell.side.read_with(cx, |p, _| p.maximized()));
    shell.update(cx, |w, window, cx| {
        w.set_panel_available(&id(EXPANDABLE), true, window, cx);
        w.set_panel_expanded(&id(EXPANDABLE), true, cx);
    });
    assert!(!shell.side_maximized(cx) && !shell.side.read_with(cx, |p, _| p.maximized()));
    shell.run(cx, |window, cx| {
        window.dispatch_action(Box::new(ToggleRightPanelMaximized), cx)
    });
    shell
        .side
        .update(cx, |_, cx| cx.emit(PanelRequest::ToggleExpanded));
    cx.run_until_parked();
    assert!(!shell.side_maximized(cx));
}

#[gpui_kit::test]
fn the_expand_action_and_the_panels_own_request_flip_an_open_side_panel(cx: &mut TestAppContext) {
    let shell = shell(cx);
    shell.open_side_panel(cx);
    shell.run(cx, |window, cx| {
        window.dispatch_action(Box::new(ToggleRightPanelMaximized), cx)
    });
    assert!(shell.side_maximized(cx) && shell.side.read_with(cx, |p, _| p.maximized()));
    shell
        .side
        .update(cx, |_, cx| cx.emit(PanelRequest::ToggleExpanded));
    cx.run_until_parked();
    assert!(!shell.side_maximized(cx) && !shell.side.read_with(cx, |p, _| p.maximized()));
}

/// Drags the edge between the sidebar and the body, as a user resizes it.
fn drag_sidebar_edge(shell: &Shell, by: f32, cx: &mut TestAppContext) {
    shell.run(cx, |window, cx| {
        window.render_frame(cx);
        let bounds = window.find("navigation-0").bounds();
        let inset = gpui_kit::component::floating::FloatingCards::get(cx)
            .map_or(px(0.), |cards| cards.margin() + px(1.));
        let from = point(bounds.right() + inset, bounds.center().y);
        window.drag(from, point(from.x + px(by), from.y), cx);
        window.render_frame(cx);
    });
}

#[gpui_kit::test]
fn resizing_the_body_while_the_side_panel_is_expanded_changes_no_remembered_width(
    cx: &mut TestAppContext,
) {
    let shell = shell(cx);
    shell.update(cx, |_, _, cx| {
        shell.layout.set("workspace.sidebar", px(260.), cx).unwrap();
        shell
            .layout
            .set("workspace.right_panel", px(420.), cx)
            .unwrap();
    });
    shell.open_side_panel(cx);
    shell.update(cx, |w, _, cx| {
        w.set_panel_expanded(&id(EXPANDABLE), true, cx)
    });
    drag_sidebar_edge(&shell, 80., cx);
    shell.run(cx, |window, cx| {
        window.resize(size(px(1500.), px(700.)));
        window.render_frame(cx);
    });
    cx.update(|cx| {
        assert_eq!(shell.layout.get("workspace.sidebar", cx), Some(px(260.)));
        assert_eq!(
            shell.layout.get("workspace.right_panel", cx),
            Some(px(420.))
        );
    });
}

#[gpui_kit::test]
fn widening_is_ignored_while_the_side_panel_is_expanded_or_not_open(cx: &mut TestAppContext) {
    let shell = shell(cx);
    shell.update(cx, |_, _, cx| {
        shell
            .layout
            .set("workspace.right_panel", px(420.), cx)
            .unwrap();
    });
    shell.update(cx, |w, window, cx| {
        w.set_panel_available(&id(EXPANDABLE), true, window, cx)
    });
    shell
        .side
        .update(cx, |_, cx| cx.emit(PanelRequest::Widen(px(200.))));
    cx.run_until_parked();
    shell.update(cx, |w, window, cx| {
        w.toggle_panel(&id(EXPANDABLE), window, cx)
    });
    shell.update(cx, |w, _, cx| {
        w.set_panel_expanded(&id(EXPANDABLE), true, cx)
    });
    shell
        .side
        .update(cx, |_, cx| cx.emit(PanelRequest::Widen(px(200.))));
    cx.run_until_parked();
    shell.update(cx, |w, _, cx| {
        w.set_panel_expanded(&id(EXPANDABLE), false, cx)
    });
    let width = shell.run(cx, |window, cx| {
        window.render_frame(cx);
        window.find("side-panel").bounds().size.width
    });
    cx.update(|cx| {
        assert_eq!(
            shell.layout.get("workspace.right_panel", cx),
            Some(px(420.))
        );
    });
    assert!(width < px(500.), "{width:?}");
}

fn expanded_shell_with_a_second_tab(cx: &mut TestAppContext) -> (Shell, Entity<Probe>) {
    let shell = shell(cx);
    let other = shell.run(cx, |_, cx| probe(cx, Rc::default()));
    shell.update(cx, |w, window, cx| w.add_item(other.clone(), window, cx));
    shell.open_side_panel(cx);
    shell.update(cx, |w, _, cx| {
        w.set_panel_expanded(&id(EXPANDABLE), true, cx)
    });
    assert!(shell.side_maximized(cx));
    (shell, other)
}

#[gpui_kit::test]
fn activating_a_tab_by_id_collapses_an_expanded_side_panel_and_focuses_the_tab(
    cx: &mut TestAppContext,
) {
    let (shell, other) = expanded_shell_with_a_second_tab(cx);
    let target = shell.central.entity_id();
    assert_ne!(
        shell
            .workspace
            .read_with(cx, |w, cx| w.active_item(cx).unwrap().item_id()),
        target
    );
    shell.update(cx, |w, window, cx| {
        assert!(w.activate_item_by_id(target, window, cx))
    });
    assert!(!shell.side_maximized(cx) && shell.side_open(cx));
    assert_eq!(
        shell
            .workspace
            .read_with(cx, |w, cx| w.active_item(cx).unwrap().item_id()),
        target
    );
    assert_eq!(shell.focus(cx), Focus::Central);
    drop(other);
}

#[gpui_kit::test]
fn a_new_active_tab_collapses_an_expanded_side_panel_and_takes_focus_from_the_sidebar(
    cx: &mut TestAppContext,
) {
    let (shell, _) = expanded_shell_with_a_second_tab(cx);
    shell.focus_navigation(0, cx);
    let third = shell.run(cx, |_, cx| probe(cx, Rc::default()));
    shell.update(cx, |w, window, cx| w.add_item(third.clone(), window, cx));
    assert!(!shell.side_maximized(cx) && shell.side_open(cx));
    shell.run(cx, |window, cx| {
        assert!(third.focus_handle(cx).is_focused(window))
    });
}

#[gpui_kit::test]
fn toggling_the_local_terminal_collapses_an_expanded_side_panel(cx: &mut TestAppContext) {
    let (shell, _) = expanded_shell_with_a_second_tab(cx);
    let local = shell.run(cx, |_, cx| probe(cx, Rc::default()));
    shell.update(cx, |w, window, cx| {
        w.set_local_terminal(local.clone(), window, cx)
    });
    shell.update(cx, |w, _, cx| {
        w.set_panel_expanded(&id(EXPANDABLE), true, cx)
    });
    assert!(shell.side_maximized(cx));
    shell.update(cx, |w, window, cx| w.toggle_local_terminal(window, cx));
    assert!(!shell.side_maximized(cx) && shell.side_open(cx));
    assert!(
        !shell
            .workspace
            .read_with(cx, |w, cx| w.local_terminal_is_visible(cx))
    );
}
