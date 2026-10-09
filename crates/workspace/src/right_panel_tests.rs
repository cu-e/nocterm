//! The right panel opens beside an empty workspace, maximizes and disables.
use super::fixture;
use gpui_kit::{
    App, Context, EventEmitter, FocusHandle, Focusable, TestAppContext, TestSupportExt as _,
    Window, component::floating::FloatingCards, div, prelude::*, px, test::TestWindowExt as _,
};

pub(crate) struct RightProbe {
    pub(crate) focus: FocusHandle,
    pub(crate) maximized: bool,
}
impl EventEmitter<crate::RightPanelEvent> for RightProbe {}
impl Focusable for RightProbe {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}
impl Render for RightProbe {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("right-probe")
            .test_support()
            .size_full()
            .track_focus(&self.focus)
            .child("AI")
    }
}
impl crate::Panel for RightProbe {
    fn title(&self, _: &App) -> gpui_kit::SharedString {
        "AI Agents".into()
    }
    fn icon(&self, _: &App) -> nocterm_ui::IconName {
        nocterm_ui::IconName::Bot
    }
}
impl crate::RightPanel for RightProbe {
    fn set_maximized(&mut self, maximized: bool, cx: &mut Context<Self>) {
        self.maximized = maximized;
        cx.notify();
    }
}

#[gpui_kit::test]
fn independent_right_panel_supports_empty_workspace_maximize_and_disable(cx: &mut TestAppContext) {
    let (handle, workspace) = fixture(cx);
    cx.update_window(handle, |_, window, cx| {
        window.resize(gpui_kit::size(px(1800.), px(760.)));
        let panel = cx.new(|cx| RightProbe {
            focus: cx.focus_handle(),
            maximized: false,
        });
        workspace.update(cx, |workspace, cx| {
            workspace.set_right_panel(panel.clone(), window, cx);
            workspace.toggle_right_panel(window, cx);
            assert!(!workspace.right_panel_is_open());
            workspace.set_right_panel_available(true, window, cx);
            workspace.toggle_right_panel(window, cx);
            assert!(workspace.right_panel_is_open());
            assert!(panel.read(cx).focus.is_focused(window));
        });
        window.render_frame(cx);
        assert!(window.try_find("right-probe").is_some());
        assert!(window.try_find("empty-new-tab").is_some());
        assert!(window.try_find("open-settings").is_none());
        let toggle = window.find("toggle-right-panel").bounds();
        assert!(toggle.left() > window.find("toggle-local-terminal").bounds().right());
        workspace.update(cx, |workspace, cx| {
            workspace.set_right_panel_maximized(true, cx)
        });
        window.render_frame(cx);
        assert!(window.try_find("empty-new-tab").is_none());
        assert!(window.try_find("toggle-right-panel").is_some());
        assert!(panel.read(cx).maximized);
        let inset = FloatingCards::get(cx).map_or(px(0.), |cards| cards.gap * 2. + px(2.));
        let probe = window.find("right-probe").bounds().size.width;
        assert!((probe - (window.viewport_size().width - inset)).abs() < px(2.));
        workspace.update(cx, |workspace, cx| {
            workspace.set_right_panel_available(false, window, cx)
        });
        window.render_frame(cx);
        assert!(window.try_find("toggle-right-panel").is_none());
        assert!(window.try_find("right-probe").is_none());
        assert!(window.try_find("empty-new-tab").is_some());
        assert!(!panel.read(cx).maximized);
        assert!(workspace.read(cx).focus_handle.is_focused(window));
    })
    .unwrap();
}
