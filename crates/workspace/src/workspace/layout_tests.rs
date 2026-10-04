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
        let state = workspace.update(cx, |workspace, cx| {
            let arrangement = workspace.arrangement(cx);
            assert!(!arrangement.side_panel);
            workspace.body.state(arrangement, cx)
        });
        let before = window.find("side-probe").bounds().size.width;
        state.update(cx, |state, cx| {
            state.resize_panel(0, before + px(150.), window, cx)
        });
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
