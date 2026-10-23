//! The body's columns: resizing, hiding a sidebar panel from its own
//! button, and swapping sides.
use gpui_kit::{
    App, AppContext as _, Context, Entity, FocusHandle, Focusable, Render, TestAppContext,
    TestSupportExt as _, Window, div, prelude::*, px, test::TestWindowExt as _,
};

use super::{Arrangement, Column, PanelPresentation};
use crate::workspace::tests::{RightProbe, fixture};

struct SideProbe {
    focus: FocusHandle,
}
impl Focusable for SideProbe {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}
impl Render for SideProbe {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("side-probe")
            .test_support()
            .size_full()
            .track_focus(&self.focus)
    }
}
impl crate::Panel for SideProbe {
    fn title(&self, _: &App) -> gpui_kit::SharedString {
        "Servers".into()
    }
    fn icon(&self, _: &App) -> nocterm_ui::IconName {
        nocterm_ui::IconName::Server
    }
}

#[test]
fn hidden_columns_leave_the_group_and_swapping_mirrors_it() {
    let arrangement = |sidebar, side_panel, swapped| Arrangement {
        sidebar,
        side_panel: PanelPresentation::new(side_panel, false),
        swapped,
    };
    assert_eq!(
        arrangement(true, false, false).columns(),
        [Column::Sidebar, Column::Content]
    );
    assert_eq!(
        arrangement(true, true, true).columns(),
        [Column::SidePanel, Column::Content, Column::Sidebar]
    );
    for swapped in [false, true] {
        let expanded = Arrangement {
            sidebar: true,
            side_panel: PanelPresentation::Expanded,
            swapped,
        };
        let expected = if swapped {
            [Column::SidePanel, Column::Sidebar]
        } else {
            [Column::Sidebar, Column::SidePanel]
        };
        assert_eq!(expanded.columns(), expected);
        assert_eq!(
            Arrangement {
                sidebar: false,
                ..expanded
            }
            .columns(),
            [Column::SidePanel]
        );
    }
}

#[gpui_kit::test]
fn sidebar_resizes_with_the_side_panel_closed_and_its_button_hides_it(cx: &mut TestAppContext) {
    let (handle, workspace) = fixture(cx);
    cx.update_window(handle, |_, window, cx| {
        window.resize(gpui_kit::size(px(1400.), px(700.)));
        let side = cx.new(|cx| SideProbe {
            focus: cx.focus_handle(),
        });
        let right = cx.new(|cx| RightProbe {
            focus: cx.focus_handle(),
            maximized: false,
        });
        workspace.update(cx, |workspace, cx| {
            workspace.add_panel(side, cx);
            workspace.set_right_panel(right, window, cx);
            workspace.set_right_panel_available(true, window, cx);
        });
        window.render_frame(cx);
        assert!(!workspace.read(cx).right_panel_is_open());
        let before = window.find("side-probe").bounds().size.width;
        let bounds = window.find("side-probe").bounds();
        let inset = gpui_kit::component::floating::FloatingCards::get(cx)
            .map_or(px(0.), |cards| cards.margin() + px(1.));
        let from = gpui_kit::point(bounds.right() + inset, bounds.center().y);
        window.drag(from, gpui_kit::point(from.x + px(150.), from.y), cx);
        window.render_frame(cx);
        let after = window.find("side-probe").bounds().size.width;
        assert!(after > before + px(100.), "{before:?} → {after:?}");

        // The shown panel's own button hides the sidebar, and shows it again.
        window.click(("sidebar-panel", 0usize), cx);
        window.render_frame(cx);
        assert!(window.try_find("side-probe").is_none());
        window.click(("sidebar-panel", 0usize), cx);
        window.render_frame(cx);
        assert!(window.try_find("side-probe").is_some());
    })
    .unwrap();
}

#[gpui_kit::test]
fn swapping_puts_the_side_panel_on_the_left(cx: &mut TestAppContext) {
    let (handle, workspace) = fixture(cx);
    cx.update_window(handle, |_, window, cx| {
        window.resize(gpui_kit::size(px(1400.), px(700.)));
        let side = cx.new(|cx| SideProbe {
            focus: cx.focus_handle(),
        });
        let right = cx.new(|cx| RightProbe {
            focus: cx.focus_handle(),
            maximized: false,
        });
        workspace.update(cx, |workspace, cx| {
            workspace.add_panel(side, cx);
            workspace.set_right_panel(right, window, cx);
            workspace.set_right_panel_available(true, window, cx);
            workspace.toggle_right_panel(window, cx);
        });
        window.render_frame(cx);
        assert!(
            window.find("side-probe").bounds().left() < window.find("right-probe").bounds().left()
        );
        window.dispatch_action(Box::new(crate::SwapSides), cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(
            window.find("right-probe").bounds().left() < window.find("side-probe").bounds().left()
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn hidden_and_reordered_columns_keep_each_owners_visible_width(cx: &mut TestAppContext) {
    let (handle, workspace, layout) = crate::workspace::tests::fixture_with_layout(cx);
    cx.update_window(handle, |_, window, cx| {
        window.resize(gpui_kit::size(px(1400.), px(700.)));
        layout.set("workspace.sidebar", px(260.), cx).unwrap();
        layout.set("workspace.right_panel", px(420.), cx).unwrap();
        let side = cx.new(|cx| SideProbe {
            focus: cx.focus_handle(),
        });
        let right = cx.new(|cx| RightProbe {
            focus: cx.focus_handle(),
            maximized: false,
        });
        workspace.update(cx, |workspace, cx| {
            workspace.add_panel(side, cx);
            workspace.set_right_panel(right, window, cx);
            workspace.set_right_panel_available(true, window, cx);
            workspace.toggle_right_panel(window, cx);
        });
        window.render_frame(cx);
        let sidebar_width = window.find("side-probe").bounds().size.width;
        let right_width = window.find("right-probe").bounds().size.width;
        workspace.update(cx, |workspace, cx| workspace.toggle_sidebar(window, cx));
        window.render_frame(cx);
        assert!(window.try_find("side-probe").is_none());
        assert_eq!(window.find("right-probe").bounds().size.width, right_width);
        workspace.update(cx, |workspace, cx| {
            workspace.swap_sides(cx);
            workspace.toggle_sidebar(window, cx);
        });
        window.render_frame(cx);
        assert!(
            window.find("right-probe").bounds().left() < window.find("side-probe").bounds().left()
        );
        assert_eq!(window.find("side-probe").bounds().size.width, sidebar_width);
        assert_eq!(window.find("right-probe").bounds().size.width, right_width);
        assert_eq!(layout.get("workspace.sidebar", cx), Some(px(260.)));
        assert_eq!(layout.get("workspace.right_panel", cx), Some(px(420.)));
    })
    .unwrap();
}

#[gpui_kit::test]
fn maximized_panel_does_not_persist_transient_window_or_widen_sizes(cx: &mut TestAppContext) {
    let (handle, workspace, layout) = crate::workspace::tests::fixture_with_layout(cx);
    cx.update_window(handle, |_, window, cx| {
        window.resize(gpui_kit::size(px(1400.), px(700.)));
        layout.set("workspace.sidebar", px(260.), cx).unwrap();
        layout.set("workspace.right_panel", px(420.), cx).unwrap();
        let (_, right) = install_columns(&workspace, window, cx);
        window.render_frame(cx);
        let normal = window.find("right-probe").bounds().size.width;
        let sidebar = window.find("side-probe").bounds().size.width;
        workspace.update(cx, |workspace, cx| {
            workspace.set_right_panel_maximized(true, cx)
        });
        window.render_frame(cx);
        assert!(window.find("right-probe").bounds().size.width > normal);
        let bounds = window.find("side-probe").bounds();
        let inset = gpui_kit::component::floating::FloatingCards::get(cx)
            .map_or(px(0.), |cards| cards.margin() + px(1.));
        let from = gpui_kit::point(bounds.right() + inset, bounds.center().y);
        window.drag(from, gpui_kit::point(from.x + px(80.), from.y), cx);
        right.update(cx, |_, cx| cx.emit(crate::RightPanelEvent::Widen(px(200.))));
        window.resize(gpui_kit::size(px(1500.), px(700.)));
        window.render_frame(cx);
        assert_eq!(layout.get("workspace.sidebar", cx), Some(px(260.)));
        assert_eq!(layout.get("workspace.right_panel", cx), Some(px(420.)));
        workspace.update(cx, |workspace, cx| {
            workspace.set_right_panel_maximized(false, cx)
        });
        window.render_frame(cx);
        assert_eq!(window.find("right-probe").bounds().size.width, normal);
        assert_eq!(window.find("side-probe").bounds().size.width, sidebar);
        assert_eq!(layout.get("workspace.sidebar", cx), Some(px(260.)));
        assert_eq!(layout.get("workspace.right_panel", cx), Some(px(420.)));
    })
    .unwrap();
}

fn install_columns(
    workspace: &Entity<crate::Workspace>,
    window: &mut Window,
    cx: &mut App,
) -> (Entity<SideProbe>, Entity<RightProbe>) {
    let side = cx.new(|cx| SideProbe {
        focus: cx.focus_handle(),
    });
    let right = cx.new(|cx| RightProbe {
        focus: cx.focus_handle(),
        maximized: false,
    });
    workspace.update(cx, |workspace, cx| {
        workspace.add_panel(side.clone(), cx);
        workspace.set_right_panel(right.clone(), window, cx);
        workspace.set_right_panel_available(true, window, cx);
        workspace.toggle_right_panel(window, cx);
    });
    (side, right)
}

fn assert_expanded_columns(window: &mut Window, cx: &mut App, swapped: bool) {
    window.render_frame(cx);
    window.render_frame(cx);
    let side = window.find("side-probe").bounds();
    let right = window.find("right-probe").bounds();
    assert!(window.try_find("empty-new-tab").is_none());
    assert!(window.try_find("toggle-right-panel").is_some());
    assert!(side.size.width > px(100.) && right.size.width > px(0.));
    if swapped {
        assert!(right.right() <= side.left());
    } else {
        assert!(side.right() <= right.left());
    }
    assert!(side.left() >= px(0.) && right.left() >= px(0.));
    assert!(side.right() <= window.viewport_size().width);
    assert!(right.right() <= window.viewport_size().width);
}

#[gpui_kit::test]
fn expanded_chat_keeps_navigation_beside_it_at_supported_window_widths(cx: &mut TestAppContext) {
    let (handle, workspace, layout) = crate::workspace::tests::fixture_with_layout(cx);
    cx.update_window(handle, |_, window, cx| {
        window.resize(gpui_kit::size(px(1400.), px(700.)));
        layout.set("workspace.sidebar", px(260.), cx).unwrap();
        let _ = install_columns(&workspace, window, cx);
        window.render_frame(cx);
        let sidebar_width = window.find("side-probe").bounds().size.width;
        let docked_width = window.find("right-probe").bounds().size.width;
        workspace.update(cx, |workspace, cx| {
            workspace.set_right_panel_maximized(true, cx)
        });
        assert_expanded_columns(window, cx, false);
        assert!(window.find("right-probe").bounds().size.width > docked_width);
        for swapped in [false, true] {
            if swapped {
                workspace.update(cx, |workspace, cx| workspace.swap_sides(cx));
            }
            for width in [600., 1000., 1400.] {
                window.resize(gpui_kit::size(px(width), px(700.)));
                assert_expanded_columns(window, cx, swapped);
                assert_eq!(window.find("side-probe").bounds().size.width, sidebar_width);
            }
        }
        workspace.update(cx, |workspace, cx| {
            workspace.set_right_panel_available(false, window, cx)
        });
        window.render_frame(cx);
        assert!(window.try_find("empty-new-tab").is_some());
        assert!(window.try_find("side-probe").is_some());
        assert!(window.try_find("right-probe").is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn chat_expand_restore_and_close_events_preserve_visible_navigation(cx: &mut TestAppContext) {
    let (handle, workspace) = fixture(cx);
    cx.run_until_parked();
    let (right, sidebar_width, docked_width) = cx
        .update_window(handle, |_, window, cx| {
            window.resize(gpui_kit::size(px(1400.), px(700.)));
            let (_, right) = install_columns(&workspace, window, cx);
            window.render_frame(cx);
            window.render_frame(cx);
            (
                right,
                window.find("side-probe").bounds().size.width,
                window.find("right-probe").bounds().size.width,
            )
        })
        .unwrap();
    use crate::RightPanelEvent::{Close, ToggleMaximized};
    for (event, expanded, open) in [
        (ToggleMaximized, true, true),
        (ToggleMaximized, false, true),
        (ToggleMaximized, true, true),
        (Close, false, false),
    ] {
        cx.update(|cx| right.update(cx, |_, cx| cx.emit(event)));
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.render_frame(cx);
            assert_eq!(
                window.find("side-probe").bounds().size.width,
                sidebar_width,
                "after {event:?}, expanded={expanded}, open={open}"
            );
            assert_eq!(window.try_find("empty-new-tab").is_none(), expanded);
            assert_eq!(window.try_find("right-probe").is_some(), open);
            if expanded {
                assert_expanded_columns(window, cx, false);
                assert!(window.find("right-probe").bounds().size.width > docked_width);
            } else if open {
                assert_eq!(window.find("right-probe").bounds().size.width, docked_width);
            }
        })
        .unwrap();
    }
}

#[gpui_kit::test]
fn sidebar_controls_preserve_expansion_and_focus_visible_chat_when_closed(cx: &mut TestAppContext) {
    let (handle, workspace) = fixture(cx);
    cx.update_window(handle, |_, window, cx| {
        window.resize(gpui_kit::size(px(1400.), px(700.)));
        let (_, right) = install_columns(&workspace, window, cx);
        let other = cx.new(|cx| SideProbe {
            focus: cx.focus_handle(),
        });
        workspace.update(cx, |workspace, cx| {
            workspace.add_panel(other.clone(), cx);
            workspace.set_right_panel_maximized(true, cx);
        });
        assert_expanded_columns(window, cx, false);
        window.click(("sidebar-panel", 1usize), cx);
        assert_expanded_columns(window, cx, false);
        assert!(other.focus_handle(cx).is_focused(window));
        window.click(("sidebar-panel", 1usize), cx);
        window.render_frame(cx);
        assert!(window.try_find("side-probe").is_none());
        assert!(window.try_find("empty-new-tab").is_none());
        assert!(right.focus_handle(cx).is_focused(window));
        let full_width = window.find("right-probe").bounds().size.width;
        workspace.update(cx, |workspace, cx| workspace.toggle_sidebar(window, cx));
        assert_expanded_columns(window, cx, false);
        assert!(window.find("right-probe").bounds().size.width < full_width);
        workspace.update(cx, |workspace, cx| {
            workspace.swap_sides(cx);
            workspace.toggle_sidebar(window, cx);
        });
        window.render_frame(cx);
        assert!(window.try_find("empty-new-tab").is_none());
        window.click(("sidebar-panel", 0usize), cx);
        assert_expanded_columns(window, cx, true);
    })
    .unwrap();
}

#[gpui_kit::test]
fn expanded_resize_receipts_cannot_overwrite_remembered_docked_widths(cx: &mut TestAppContext) {
    cx.update(|cx| {
        let layout = nocterm_ui::LayoutHandle::in_memory(cx);
        layout.set("workspace.sidebar", px(260.), cx).unwrap();
        layout.set("workspace.right_panel", px(420.), cx).unwrap();
        let body = super::Body::new(layout.clone());
        for swapped in [false, true] {
            let expanded = Arrangement {
                sidebar: true,
                side_panel: PanelPresentation::Expanded,
                swapped,
            };
            body.resized(expanded, vec![px(350.), px(1050.)], cx);
        }
        assert_eq!(layout.get("workspace.sidebar", cx), Some(px(260.)));
        assert_eq!(layout.get("workspace.right_panel", cx), Some(px(420.)));
    });
}
