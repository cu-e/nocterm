//! Window effects stay here; native tab mutations belong to the original surface.
use super::*;
use crate::Panel;
use gpui_kit::{
    InteractiveElement,
    component::{Placement, input},
};
use panel_groups::{PanelFocus, PanelTransition};
impl Workspace {
    pub fn activate_item_by_id(
        &mut self,
        id: EntityId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        self.set_right_panel_maximized(false, cx);
        self.surfaces.outer().update(cx, |surface, cx| {
            surface.activate_item_by_id(id, window, cx)
        })
    }
    pub(crate) fn close_item_by_id(
        &mut self,
        id: EntityId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.surfaces
            .outer()
            .update(cx, |surface, cx| surface.close_item_by_id(id, window, cx))
    }
    pub fn focus_terminal(
        &mut self,
        id: EntityId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        self.surfaces
            .outer()
            .update(cx, |surface, cx| surface.focus_terminal(id, window, cx))
    }
    pub fn close_tabs(
        &mut self,
        id: EntityId,
        scope: TabCloseScope,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.surfaces
            .outer()
            .update(cx, |surface, cx| surface.close_tabs(id, scope, window, cx))
    }
    pub fn split_item(
        &mut self,
        id: EntityId,
        placement: Placement,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.surfaces.outer().update(cx, |surface, cx| {
            surface.split_item(id, placement, window, cx)
        })
    }
    pub fn new_tab_group(&mut self, id: EntityId, window: &mut Window, cx: &mut Context<Self>) {
        self.surfaces.outer().update(cx, |surface, cx| {
            surface.content_mut().new_tab_group(id, window, cx);
            cx.notify();
        })
    }
    pub fn group_tab_with(
        &mut self,
        id: EntityId,
        other: EntityId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.surfaces.outer().update(cx, |surface, cx| {
            surface.content_mut().group_tab_with(id, other, window, cx);
            cx.notify();
        })
    }
    pub fn ungroup_tab(&mut self, id: EntityId, window: &mut Window, cx: &mut Context<Self>) {
        self.surfaces.outer().update(cx, |surface, cx| {
            surface.content_mut().ungroup_tab(id, window, cx);
            cx.notify();
        })
    }
    pub fn dissolve_tab_group(
        &mut self,
        id: EntityId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.surfaces.outer().update(cx, |surface, cx| {
            surface.content_mut().dissolve_tab_group(id, window, cx);
            cx.notify();
        })
    }
    pub fn close_tab_group(&mut self, id: EntityId, window: &mut Window, cx: &mut Context<Self>) {
        self.surfaces
            .outer()
            .update(cx, |surface, cx| surface.close_tab_group(id, window, cx))
    }
    pub(super) fn group_program(
        &mut self,
        parent: Option<EntityId>,
        opened_from: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.surfaces.outer().update(cx, |surface, cx| {
            surface
                .content_mut()
                .group_program(parent, opened_from, window, cx);
            cx.notify();
        })
    }
    pub fn close_local_terminal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.surfaces
            .outer()
            .update(cx, |surface, cx| surface.close_local_terminal(window, cx))
    }
    pub fn activate_item(&mut self, ordinal: usize, window: &mut Window, cx: &mut Context<Self>) {
        let id = self.items(cx).nth(ordinal).map(|item| item.item_id());
        if let Some(id) = id {
            self.activate_item_by_id(id, window, cx);
        }
    }
    pub fn close_item(&mut self, ordinal: usize, window: &mut Window, cx: &mut Context<Self>) {
        let id = self.items(cx).nth(ordinal).map(|item| item.item_id());
        if let Some(id) = id {
            self.close_item_by_id(id, window, cx);
        }
    }
    pub(super) fn close_active_tabs(
        &mut self,
        scope: TabCloseScope,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(id) = self.command_item(window, cx).map(|item| item.item_id()) {
            self.command_surface(window, cx)
                .update(cx, |surface, cx| surface.close_tabs(id, scope, window, cx));
        }
    }
    pub fn split_active(
        &mut self,
        placement: Placement,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(id) = self.command_item(window, cx).map(|item| item.item_id()) {
            self.command_surface(window, cx).update(cx, |surface, cx| {
                surface.split_item(id, placement, window, cx)
            });
        }
    }
    pub fn focus_pane(&mut self, offset: isize, window: &mut Window, cx: &mut Context<Self>) {
        let surface = self.command_surface(window, cx);
        let target = self.command_item(window, cx).and_then(|item| {
            surface
                .read(cx)
                .tabs()
                .next_pane(item.item_id(), offset, cx)
        });
        if let Some(id) = target {
            surface.update(cx, |surface, cx| {
                surface.activate_item_by_id(id, window, cx)
            });
        }
    }
    pub(super) fn cycle_tab(&mut self, offset: isize, window: &mut Window, cx: &mut Context<Self>) {
        let surface = self.command_surface(window, cx);
        let target = self
            .command_item(window, cx)
            .and_then(|item| surface.read(cx).tabs().next_tab(item.item_id(), offset, cx));
        if let Some(id) = target {
            surface.update(cx, |surface, cx| {
                surface.activate_item_by_id(id, window, cx)
            });
        }
    }
    pub(super) fn toggle_sidebar_panel(
        &mut self,
        ix: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.panels.is_showing(GroupKey::Navigation, ix) {
            self.toggle_sidebar(window, cx);
        } else {
            self.activate_panel(ix, window, cx);
        }
    }
    pub(super) fn body_resized(
        &mut self,
        arrangement: &layout::Arrangement,
        sizes: Vec<Pixels>,
        cx: &mut Context<Self>,
    ) {
        if self.panels.expanded().is_none() {
            self.body.resized(arrangement, sizes, cx);
        }
    }
    pub fn activate_panel(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(transition) = self.panels.activate(GroupKey::Navigation, ix, cx) {
            self.apply_panel_transition(transition, window, cx);
        }
    }
    pub fn activate_panel_of<T: Panel>(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(ix) = self.panel_index_of::<T>() {
            self.activate_panel(ix, window, cx);
        }
    }
    pub fn toggle_panel_of<T: Panel>(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.panel_index_of::<T>() {
            Some(ix) if self.panels.is_showing(GroupKey::Navigation, ix) => {
                self.toggle_sidebar(window, cx)
            }
            Some(ix) => self.activate_panel(ix, window, cx),
            None => {}
        }
    }
    fn panel_index_of<T: Panel>(&self) -> Option<usize> {
        self.panels.index_of::<T>(GroupKey::Navigation)
    }
    pub fn toggle_sidebar(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(transition) = self.panels.toggle(GroupKey::Navigation, window, cx) {
            self.apply_panel_transition(transition, window, cx);
        }
    }
    pub fn set_right_panel_available(
        &mut self,
        available: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(transition) =
            self.panels
                .set_available(GroupKey::Assistant, available, window, cx)
        {
            self.apply_panel_transition(transition, window, cx);
        }
    }
    pub fn set_right_panel_attention(&mut self, attention: bool, cx: &mut Context<Self>) {
        if self.panels.set_attention(GroupKey::Assistant, attention) {
            cx.notify();
        }
    }
    /// The icons and tooltips the footer draws for its toggles.
    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn footer_toggle_looks(&self, cx: &App) -> super::chrome::FooterLooks {
        super::chrome::FooterLooks::of(&self.panels, cx)
    }
    pub fn toggle_right_panel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(transition) = self.panels.toggle(GroupKey::Assistant, window, cx) {
            self.apply_panel_transition(transition, window, cx);
        }
    }
    pub fn set_right_panel_maximized(&mut self, maximized: bool, cx: &mut Context<Self>) {
        if self.panels.set_expanded(GroupKey::Assistant, maximized, cx) {
            cx.notify();
        }
    }
    pub(super) fn close_right_panel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let transition = self.panels.close(GroupKey::Assistant, window, cx);
        self.apply_panel_transition(transition, window, cx);
    }
    fn apply_panel_transition(
        &mut self,
        transition: PanelTransition,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if transition.visibility_changed() == Some(GroupKey::Assistant) {
            cx.emit(WorkspaceEvent::RightPanelVisibilityChanged);
        }
        match transition.into_focus() {
            PanelFocus::Preserve => {}
            PanelFocus::Panel(focus) => window.focus(&focus, cx),
            PanelFocus::Central => {
                let focus = self
                    .active_item(cx)
                    .map(|item| item.focus_handle(cx))
                    .unwrap_or_else(|| self.focus_handle.clone());
                window.focus(&focus, cx);
            }
            PanelFocus::ActiveItem => {
                if let Some(item) = self.active_item(cx) {
                    window.focus(&item.focus_handle(cx), cx);
                }
            }
        }
        cx.notify();
    }
    pub fn swap_sides(&mut self, cx: &mut Context<Self>) {
        let swapped = self.body.swap(cx);
        self.panels.set_docked_left(swapped, cx);
        cx.notify();
    }
    pub(super) fn widen_right_panel(
        &mut self,
        delta: Pixels,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.panels.is_expanded(GroupKey::Assistant)
            || !self.panels.is_shown(GroupKey::Assistant)
        {
            return;
        }
        let arrangement = self.body.arrangement(&self.panels, cx);
        self.body
            .widen_group(&arrangement, GroupKey::Assistant, delta, window, cx);
        cx.notify();
    }
    pub fn dispatch_command(
        &mut self,
        action: Box<dyn Action>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(item) = self.command_item(window, cx) else {
            return;
        };
        let surface = self.command_surface(window, cx);
        let selection = surface.read(cx).selection();
        let surface = surface.downgrade();
        window.defer(cx, move |window, cx| {
            let target = surface.read_with(cx, |this, cx| {
                this.accepts_context_selection(selection, cx)
                    && this.tabs().contains(item.item_id())
                    && this.tabs().item_visible(item.item_id(), cx)
            });
            if target.unwrap_or(false) {
                let focus = item.focus_handle(cx);
                if window.is_action_available_in(action.as_ref(), &focus) {
                    focus.dispatch_action(action.as_ref(), window, cx);
                }
            }
        });
    }
    pub(super) fn forward_commands<E: InteractiveElement>(element: E, cx: &mut Context<Self>) -> E {
        fn forward<A: Action, E: InteractiveElement>(element: E, cx: &mut Context<Workspace>) -> E {
            element.on_action(cx.listener(|this, action: &A, window, cx| {
                this.dispatch_command(action.boxed_clone(), window, cx)
            }))
        }
        let element = forward::<crate::ClearSelection, _>(element, cx);
        let element = forward::<crate::Find, _>(element, cx);
        let element = forward::<crate::FindNext, _>(element, cx);
        let element = forward::<crate::FindPrevious, _>(element, cx);
        let element = forward::<crate::FindNextSelection, _>(element, cx);
        let element = forward::<crate::DisconnectSession, _>(element, cx);
        let element = forward::<crate::ReconnectSession, _>(element, cx);
        let element = forward::<crate::StartRecording, _>(element, cx);
        let element = forward::<crate::StopRecording, _>(element, cx);
        let element = forward::<crate::SessionSettings, _>(element, cx);
        let element = forward::<input::Copy, _>(element, cx);
        let element = forward::<input::Paste, _>(element, cx);
        forward::<input::SelectAll, _>(element, cx)
    }
    pub(super) fn surface_event(
        &mut self,
        event: &surface::SurfaceEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            surface::SurfaceEvent::Observed(source, event) => {
                if self.surfaces.window().resolve(*source, cx).is_err() {
                    return;
                }
                if *source != self.general_surface_source(cx) && *source != self.context_source(cx)
                {
                    cx.emit(WorkspaceEvent::SurfaceItemsChanged(*source));
                    return;
                }
                if *event == WorkspaceEvent::ActiveItemChanged
                    && *source == self.general_surface_source(cx)
                {
                    self.set_right_panel_maximized(false, cx);
                }
                if self.surfaces.observe_source(self.context_source(cx)) {
                    cx.emit(WorkspaceEvent::ContextChanged);
                }
                cx.emit(*event);
                cx.notify();
            }
            surface::SurfaceEvent::OpenLocal(selection, target) => {
                if self.accepts_context_selection(*selection, cx) {
                    self.factory().open_local(self, *target, window, cx);
                }
            }
            surface::SurfaceEvent::OpenTerminal(_, _) => {}
        }
    }
    pub fn rename_active_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(id) = self.command_item(window, cx).map(|item| item.item_id()) {
            self.command_surface(window, cx)
                .update(cx, |surface, cx| surface.rename_item(id, window, cx));
        }
    }
    pub fn move_active_tab(&mut self, offset: isize, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(id) = self.command_item(window, cx).map(|item| item.item_id()) {
            self.command_surface(window, cx).update(cx, |surface, cx| {
                surface.content_mut().move_item(id, offset, window, cx)
            });
        }
    }
    pub fn toggle_local_terminal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.set_right_panel_maximized(false, cx);
        let focused = self.command_item(window, cx).map(|item| item.item_id());
        let toggled = self
            .surfaces
            .outer()
            .update(cx, |surface, cx| surface.toggle_local(focused, window, cx));
        if !toggled {
            self.new_local_terminal(window, cx);
        }
        cx.emit(WorkspaceEvent::ItemsChanged);
        cx.notify();
    }
    pub fn change_local_directory(
        &mut self,
        path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), crate::LocalDirectoryError> {
        if self.selected_local_terminal(cx).is_none() {
            self.new_local_terminal(window, cx);
        }
        if self
            .selected_local_terminal(cx)
            .and_then(|id| self.tab_location(id, cx))
            .is_some_and(|(placement, _)| placement == DockPlacement::Bottom)
            && !self.dock_region_open(DockPlacement::Bottom, cx)
        {
            self.toggle_local_terminal(window, cx);
        }
        self.surfaces.outer().update(cx, |surface, cx| {
            surface.content_mut().change_directory(path, window, cx)
        })
    }
}
