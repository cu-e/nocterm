//! The body's columns: resizing, hiding a sidebar panel from its own
//! button, and swapping sides.
use gpui_kit::{
    App, AppContext as _, Context, FocusHandle, Focusable, Render, TestAppContext,
    TestSupportExt as _, Window, div, prelude::*, px, test::TestWindowExt as _,
};

use super::{Arrangement, Column};
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
        side_panel,
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
        layout.set("workspace.right_panel", px(420.), cx).unwrap();
        let right = cx.new(|cx| RightProbe {
            focus: cx.focus_handle(),
            maximized: false,
        });
        workspace.update(cx, |workspace, cx| {
            workspace.set_right_panel(right.clone(), window, cx);
            workspace.set_right_panel_available(true, window, cx);
            workspace.toggle_right_panel(window, cx);
        });
        window.render_frame(cx);
        let normal = window.find("right-probe").bounds().size.width;
        workspace.update(cx, |workspace, cx| {
            workspace.set_right_panel_maximized(true, cx)
        });
        window.render_frame(cx);
        assert!(window.find("right-probe").bounds().size.width > normal);
        right.update(cx, |_, cx| cx.emit(crate::RightPanelEvent::Widen(px(200.))));
        window.resize(gpui_kit::size(px(1500.), px(700.)));
        window.render_frame(cx);
        assert_eq!(layout.get("workspace.right_panel", cx), Some(px(420.)));
        workspace.update(cx, |workspace, cx| {
            workspace.set_right_panel_maximized(false, cx)
        });
        window.render_frame(cx);
        assert_eq!(window.find("right-probe").bounds().size.width, normal);
        assert_eq!(layout.get("workspace.right_panel", cx), Some(px(420.)));
    })
    .unwrap();
}
