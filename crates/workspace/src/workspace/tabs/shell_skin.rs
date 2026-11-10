//! The outer dock keeps the native skin and draws the window's top toolbars under the tab bar
//! of its only tab group.
use std::{rc::Rc, sync::Arc};

use gpui_kit::{
    AnyElement, AnyView, App, Axis, Context, Div, Stateful, Window,
    base::ResizeHandleContext,
    component::{
        dock::{
            BasePanelView, DockArea, DockAreaRenderer, DockContext, DockSkin, DropIndicator,
            NodeId, PanelState, TabGroupContext, TabGroupRenderer,
        },
        v_flex,
    },
    prelude::*,
};

use super::super::chrome::TopToolbars;

pub(in crate::workspace) struct ShellDockSkin {
    native: Rc<DockSkin>,
    top: Rc<TopToolbars>,
}

impl ShellDockSkin {
    pub(in crate::workspace) fn dock_area(
        id: impl Into<gpui_kit::SharedString>,
        top: Rc<TopToolbars>,
        window: &mut Window,
        cx: &mut App,
    ) -> (gpui_kit::Entity<DockArea>, Rc<DockSkin>) {
        let mut skin = None;
        let area = cx.new(|cx: &mut Context<DockArea>| {
            let native = DockSkin::new(cx);
            skin = Some(native.clone());
            let renderer = Rc::new(Self { native, top });
            DockArea::new(id, None, window, cx).with_renderer(renderer)
        });
        (area, skin.expect("DockSkin constructed with its area"))
    }
}

impl DockAreaRenderer for ShellDockSkin {
    fn frame(&self, window: &mut Window, cx: &mut App) -> Stateful<Div> {
        self.native.frame(window, cx)
    }
    fn split_frame(
        &self,
        node: NodeId,
        axis: Axis,
        window: &mut Window,
        cx: &mut App,
    ) -> Stateful<Div> {
        self.native.split_frame(node, axis, window, cx)
    }
    fn center_frame(&self, window: &mut Window, cx: &mut App) -> Stateful<Div> {
        self.native.center_frame(window, cx)
    }
    fn render_split_handle(
        &self,
        handle: &ResizeHandleContext,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<AnyElement> {
        self.native.render_split_handle(handle, window, cx)
    }
    fn render_dock(
        &self,
        dock: &DockContext,
        content: AnyElement,
        window: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        self.native.render_dock(dock, content, window, cx)
    }
    fn build_placeholder(
        &self,
        state: &PanelState,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<Arc<dyn BasePanelView>> {
        self.native.build_placeholder(state, window, cx)
    }
    fn tab_group_renderer(&self) -> Rc<dyn TabGroupRenderer> {
        Rc::new(ShellTabSkin {
            native: self.native.tab_group_renderer(),
            top: self.top.clone(),
        })
    }
}

struct ShellTabSkin {
    native: Rc<dyn TabGroupRenderer>,
    top: Rc<TopToolbars>,
}

impl TabGroupRenderer for ShellTabSkin {
    fn frame(&self, group: &TabGroupContext, window: &mut Window, cx: &mut App) -> Stateful<Div> {
        self.native.frame(group, window, cx)
    }
    fn content_frame(
        &self,
        group: &TabGroupContext,
        window: &mut Window,
        cx: &mut App,
    ) -> Stateful<Div> {
        self.native.content_frame(group, window, cx)
    }
    fn render_tab_bar(
        &self,
        group: &TabGroupContext,
        window: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        self.native.render_tab_bar(group, window, cx)
    }
    fn render_active_panel(
        &self,
        panel: AnyView,
        group: &TabGroupContext,
        window: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        let content = self.native.render_active_panel(panel, group, window, cx);
        if group.is_collapsed() || self.top.anchor() != Some(group.node()) {
            return content;
        }
        let Some(row) = self.top.row(cx) else {
            return content;
        };
        v_flex()
            .flex_1()
            .min_h_0()
            .child(row)
            .child(content)
            .into_any_element()
    }
    fn render_drop_indicator(
        &self,
        indicator: DropIndicator,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<AnyElement> {
        self.native.render_drop_indicator(indicator, window, cx)
    }
    fn render_empty(
        &self,
        group: &TabGroupContext,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<AnyElement> {
        self.native.render_empty(group, window, cx)
    }
}
