//! One configurable shell panel for tests of the sidebar, the side panel and the footer.
use crate::{Panel, RightPanel, RightPanelEvent};
use gpui_kit::{
    App, AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable, Render,
    SharedString, TestSupportExt as _, Window, div, prelude::*,
};
use nocterm_ui::IconName;
use std::{cell::Cell, rc::Rc};

/// A panel that is also a valid side panel; all mutable state stays private.
pub struct TestPanel {
    focus: FocusHandle,
    element_id: &'static str,
    title: SharedString,
    icon: IconName,
    badge: Option<SharedString>,
    text: SharedString,
    maximized: bool,
    docked_left: bool,
    renders: Option<Rc<Cell<usize>>>,
}

/// Describes a [`TestPanel`]; defaults give a titled "Panel" with the bot icon.
pub struct TestPanelBuilder {
    element_id: &'static str,
    title: SharedString,
    icon: IconName,
    badge: Option<SharedString>,
    text: SharedString,
    renders: Option<Rc<Cell<usize>>>,
}

impl TestPanel {
    /// Starts a panel whose root element has this id, for lookups in rendered frames.
    pub fn builder(element_id: &'static str) -> TestPanelBuilder {
        TestPanelBuilder {
            element_id,
            title: "Panel".into(),
            icon: IconName::Bot,
            badge: None,
            text: SharedString::default(),
            renders: None,
        }
    }

    pub fn is_focused(&self, window: &Window) -> bool {
        self.focus.is_focused(window)
    }

    /// Whether the workspace last asked this panel to present itself maximized.
    pub fn maximized(&self) -> bool {
        self.maximized
    }

    /// Whether the workspace last said this panel sits left of the content.
    pub fn docked_left(&self) -> bool {
        self.docked_left
    }

    pub fn set_badge(&mut self, badge: Option<SharedString>, cx: &mut Context<Self>) {
        self.badge = badge;
        cx.notify();
    }

    pub fn set_text(&mut self, text: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.text = text.into();
        cx.notify();
    }
}

impl TestPanelBuilder {
    pub fn title(mut self, title: impl Into<SharedString>) -> Self {
        self.title = title.into();
        self
    }

    pub fn icon(mut self, icon: IconName) -> Self {
        self.icon = icon;
        self
    }

    pub fn badge(mut self, badge: impl Into<SharedString>) -> Self {
        self.badge = Some(badge.into());
        self
    }

    pub fn text(mut self, text: impl Into<SharedString>) -> Self {
        self.text = text.into();
        self
    }

    /// Counts every render of the panel's own content.
    pub fn render_counter(mut self, renders: Rc<Cell<usize>>) -> Self {
        self.renders = Some(renders);
        self
    }

    pub fn build(self, cx: &mut App) -> Entity<TestPanel> {
        cx.new(|cx| TestPanel {
            focus: cx.focus_handle(),
            element_id: self.element_id,
            title: self.title,
            icon: self.icon,
            badge: self.badge,
            text: self.text,
            maximized: false,
            docked_left: false,
            renders: self.renders,
        })
    }
}

impl EventEmitter<RightPanelEvent> for TestPanel {}
impl Focusable for TestPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}
impl Render for TestPanel {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        if let Some(renders) = &self.renders {
            renders.set(renders.get() + 1);
        }
        div()
            .id(self.element_id)
            .test_support()
            .size_full()
            .track_focus(&self.focus)
            .child(self.text.clone())
    }
}
impl Panel for TestPanel {
    fn title(&self, _: &App) -> SharedString {
        self.title.clone()
    }
    fn icon(&self, _: &App) -> IconName {
        self.icon
    }
    fn badge(&self, _: &App) -> Option<SharedString> {
        self.badge.clone()
    }
}
impl RightPanel for TestPanel {
    fn set_maximized(&mut self, maximized: bool, cx: &mut Context<Self>) {
        self.maximized = maximized;
        cx.notify();
    }
    fn set_docked_left(&mut self, left: bool, _: &mut Context<Self>) {
        self.docked_left = left;
    }
}
