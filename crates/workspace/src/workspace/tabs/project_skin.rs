//! Inner terminal panes reuse native controls without another floating card.
use std::{rc::Rc, sync::Arc};

use gpui_kit::{
    AnyElement, AnyView, App, Context, Div, Empty, Stateful, StyleRefinement, Window,
    base::ResizeHandleContext,
    component::dock::{
        BasePanelView, ClosePanel, DockArea, DockAreaRenderer, DockContext, DockSkin,
        DropIndicator, PanelHandle, PanelState, TabGroupContext, TabGroupRenderer, ToggleZoom,
    },
    div,
    prelude::*,
};

pub(in crate::workspace) struct ProjectDockSkin {
    native: Rc<DockSkin>,
}
impl ProjectDockSkin {
    pub(in crate::workspace) fn dock_area(
        id: impl Into<gpui_kit::SharedString>,
        window: &mut Window,
        cx: &mut App,
    ) -> (gpui_kit::Entity<DockArea>, Rc<DockSkin>) {
        let mut skin = None;
        let area = cx.new(|cx: &mut Context<DockArea>| {
            let native = DockSkin::new(cx);
            skin = Some(native.clone());
            let renderer = Rc::new(Self { native });
            DockArea::new(id, None, window, cx).with_renderer(renderer)
        });
        (area, skin.expect("DockSkin constructed with its area"))
    }
}
impl DockAreaRenderer for ProjectDockSkin {
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
        Rc::new(ProjectTabSkin {
            native: self.native.tab_group_renderer(),
        })
    }
}

struct ProjectTabSkin {
    native: Rc<dyn TabGroupRenderer>,
}
impl TabGroupRenderer for ProjectTabSkin {
    fn frame(&self, group: &TabGroupContext, _: &mut Window, cx: &mut App) -> Stateful<Div> {
        let zoom_control = group.active_panel().is_some_and(|panel| {
            panel.zoomable(cx)
                && PanelHandle::of(panel).is_some_and(|handle| handle.zoom_control(cx).is_some())
        });
        div()
            .id("project-terminal-pane")
            .when(!group.is_collapsed(), |frame| {
                frame
                    .on_action({
                        let group = group.clone();
                        move |_: &ToggleZoom, window, cx| {
                            if group.is_zoomed() || zoom_control {
                                group.toggle_zoom(window, cx);
                            }
                        }
                    })
                    .on_action({
                        let group = group.clone();
                        move |_: &ClosePanel, window, cx| {
                            if let Some(panel) = group.active_panel() {
                                group.close(panel.panel_id(cx), window, cx);
                            }
                        }
                    })
            })
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
        _: &mut Window,
        _: &mut App,
    ) -> AnyElement {
        if group.is_collapsed() {
            return Empty.into_any_element();
        }
        div()
            .relative()
            .flex_1()
            .min_h_0()
            .child(panel.cached(StyleRefinement::default().absolute().size_full()))
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
}
