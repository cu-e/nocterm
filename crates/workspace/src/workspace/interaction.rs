//! Cross-owner commands apply typed effects only after their owner commits.
use super::*;
use crate::Panel;
use gpui_kit::{
    InteractiveElement,
    component::{Placement, input},
};
use right_panel::{PanelFocus, PanelTransition};
use tabs::{TabFocus, TabTransition};

impl Workspace {
    fn apply_tab_transition(
        &mut self,
        transition: TabTransition,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (before, focus, after, notify) = transition.into_parts();
        for event in before {
            cx.emit(event);
        }
        self.apply_tab_focus(focus, window, cx);
        for event in after {
            cx.emit(event);
        }
        if notify {
            cx.notify();
        }
    }
    fn apply_tab_focus(&mut self, focus: TabFocus, window: &mut Window, cx: &mut Context<Self>) {
        match focus {
            TabFocus::Preserve => {}
            TabFocus::Direct(focus) => window.focus(&focus, cx),
            TabFocus::Activate(id) => {
                self.activate_item_by_id(id, window, cx);
            }
            TabFocus::Central => self.focus_central(window, cx),
        }
    }
    fn emit_tab_events(&self, events: Vec<WorkspaceEvent>, cx: &mut Context<Self>) {
        let changed = !events.is_empty();
        for event in events {
            cx.emit(event);
        }
        if changed {
            cx.notify();
        }
    }
    pub fn activate_item(&mut self, ordinal: usize, window: &mut Window, cx: &mut Context<Self>) {
        let id = self.items().nth(ordinal).map(|item| item.item_id());
        if let Some(id) = id {
            self.activate_item_by_id(id, window, cx);
        }
    }
    pub fn activate_item_by_id(
        &mut self,
        id: EntityId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.tabs.item_visible(id, cx) {
            return false;
        }
        self.set_right_panel_maximized(false, cx);
        if let Some(transition) = self.tabs.activate(id, window, cx) {
            self.apply_tab_transition(transition, window, cx);
            true
        } else {
            false
        }
    }
    pub fn close_item(&mut self, ordinal: usize, window: &mut Window, cx: &mut Context<Self>) {
        let id = self.items().nth(ordinal).map(|item| item.item_id());
        if let Some(id) = id {
            self.close_item_by_id(id, window, cx);
        }
    }
    pub(crate) fn close_item_by_id(
        &mut self,
        id: EntityId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let focused = self
            .command_item(window, cx)
            .is_some_and(|item| item.item_id() == id);
        if let Some(transition) = self.tabs.close(id, focused, window, cx) {
            self.apply_tab_transition(transition, window, cx);
        }
    }
    pub(crate) fn item_removed(
        &mut self,
        id: EntityId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_item_by_id(id, window, cx);
    }
    pub(super) fn item_changed(
        &mut self,
        id: EntityId,
        session: Option<SessionContext>,
        cx: &mut Context<Self>,
    ) {
        if let Some(transition) = self.tabs.changed(id, session, cx) {
            let (before, _, after, notify) = transition.into_parts();
            for event in before.into_iter().chain(after) {
                cx.emit(event);
            }
            if notify {
                cx.notify();
            }
        }
    }
    pub(super) fn item_focused(&mut self, id: EntityId, cx: &mut Context<Self>) {
        if let Some(transition) = self.tabs.focused(id) {
            let (before, _, after, notify) = transition.into_parts();
            for event in before.into_iter().chain(after) {
                cx.emit(event);
            }
            if notify {
                cx.notify();
            }
        }
    }
    pub(super) fn native_dock_changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let events = self.tabs.native_changed(window, cx);
        self.emit_tab_events(events, cx);
        cx.notify();
    }
    pub(super) fn native_dock_visibility_changed(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.tabs.hidden_bottom_has_focus(window, cx) {
            self.focus_central(window, cx);
        }
    }
    pub fn rename_active_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(id) = self.command_item(window, cx).map(|item| item.item_id()) {
            self.tabs.rename(id, window, cx);
        }
    }
    pub(super) fn focus_central(&self, window: &mut Window, cx: &mut Context<Self>) {
        let focus = self
            .active_item()
            .filter(|item| self.tabs.item_visible(item.item_id(), cx))
            .map_or_else(|| self.focus_handle.clone(), |item| item.focus_handle(cx));
        window.focus(&focus, cx);
    }
    pub fn focus_terminal(
        &mut self,
        id: EntityId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        self.tabs.terminal(id, cx) && self.activate_item_by_id(id, window, cx)
    }
    pub fn close_tabs(
        &mut self,
        id: EntityId,
        scope: TabCloseScope,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        for id in self.tabs.tabs_to_close(id, scope, cx) {
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
            self.close_tabs(id, scope, window, cx);
        }
    }
    pub fn split_active(
        &mut self,
        placement: Placement,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(id) = self.command_item(window, cx).map(|item| item.item_id()) {
            self.split_item(id, placement, window, cx);
        }
    }
    pub fn split_item(
        &mut self,
        id: EntityId,
        placement: Placement,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.tabs.split(id, placement, window, cx) {
            cx.notify();
        }
    }
    pub fn focus_pane(&mut self, offset: isize, window: &mut Window, cx: &mut Context<Self>) {
        let target = self
            .command_item(window, cx)
            .and_then(|item| self.tabs.next_pane(item.item_id(), offset, cx));
        if let Some(id) = target {
            self.activate_item_by_id(id, window, cx);
        }
    }
    pub(super) fn cycle_tab(&mut self, offset: isize, window: &mut Window, cx: &mut Context<Self>) {
        let target = self
            .command_item(window, cx)
            .and_then(|item| self.tabs.next_tab(item.item_id(), offset, cx));
        if let Some(id) = target {
            self.activate_item_by_id(id, window, cx);
        }
    }
    pub fn move_active_tab(&mut self, offset: isize, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(id) = self.command_item(window, cx).map(|item| item.item_id()) {
            self.tabs.move_tab(id, offset, window, cx);
        }
    }
    pub fn new_tab_group(&mut self, id: EntityId, window: &mut Window, cx: &mut Context<Self>) {
        self.tabs.new_tab_group(id, window, cx);
        cx.notify();
    }
    pub fn group_tab_with(
        &mut self,
        id: EntityId,
        other: EntityId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.tabs.group_tab_with(id, other, window, cx);
        cx.notify();
    }
    pub fn ungroup_tab(&mut self, id: EntityId, window: &mut Window, cx: &mut Context<Self>) {
        self.tabs.ungroup_tab(id, window, cx);
        cx.notify();
    }
    pub fn dissolve_tab_group(
        &mut self,
        id: EntityId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.tabs.dissolve_tab_group(id, window, cx);
        cx.notify();
    }
    pub fn close_tab_group(&mut self, id: EntityId, window: &mut Window, cx: &mut Context<Self>) {
        for id in self.tabs.tab_group_items(id) {
            self.close_item_by_id(id, window, cx);
        }
    }
    pub(super) fn group_program(
        &mut self,
        parent: Option<EntityId>,
        opened_from: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.tabs.group_program(parent, opened_from, window, cx);
        cx.notify();
    }
    pub fn toggle_local_terminal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.set_right_panel_maximized(false, cx);
        let focused = self.command_item(window, cx).map(|item| item.item_id());
        if let Some(focus) = self.tabs.toggle_bottom(focused, window, cx) {
            self.apply_tab_focus(focus, window, cx);
        } else {
            self.new_local_terminal(window, cx);
        }
        cx.emit(WorkspaceEvent::ItemsChanged);
        cx.notify();
    }
    pub fn close_local_terminal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(id) = self.tabs.selected_local_id(cx) {
            self.close_item_by_id(id, window, cx);
        }
    }
    pub fn change_local_directory(
        &mut self,
        path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), crate::LocalDirectoryError> {
        if self.tabs.selected_local_id(cx).is_none() {
            self.new_local_terminal(window, cx);
        }
        if self
            .tabs
            .selected_local_id(cx)
            .and_then(|id| self.tab_location(id, cx))
            .is_some_and(|(placement, _)| placement == DockPlacement::Bottom)
            && !self.tabs.region_open(DockPlacement::Bottom, cx)
        {
            self.toggle_local_terminal(window, cx);
        }
        self.tabs
            .selected_local_handle(cx)
            .ok_or(crate::LocalDirectoryError::Unavailable)?
            .change_directory(path, window, cx)
    }
}
impl Workspace {
    pub(super) fn toggle_sidebar_panel(
        &mut self,
        ix: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.sidebar.showing(ix) {
            self.toggle_sidebar(window, cx);
        } else {
            self.activate_panel(ix, window, cx);
        }
    }
    pub(super) fn body_resized(
        &mut self,
        arrangement: layout::Arrangement,
        sizes: Vec<Pixels>,
        cx: &mut Context<Self>,
    ) {
        if !self.right_panel.maximized() {
            self.body.resized(arrangement, sizes, cx);
        }
    }
    /// Shows the panel at `ix`, opening the sidebar if it is closed.
    pub fn activate_panel(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.set_right_panel_maximized(false, cx);
        let Some(focus) = self.sidebar.activate(ix, cx) else {
            return;
        };
        window.focus(&focus, cx);
        cx.notify();
    }

    /// Shows the panel showing a `T`, if there is one.
    pub fn activate_panel_of<T: Panel>(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(ix) = self.panel_index_of::<T>() {
            self.activate_panel(ix, window, cx);
        }
    }

    /// Shows the panel showing a `T`, or hides the sidebar when it already
    /// shows it.
    pub fn toggle_panel_of<T: Panel>(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.panel_index_of::<T>() {
            Some(ix) if self.sidebar.showing(ix) => self.toggle_sidebar(window, cx),
            Some(ix) => self.activate_panel(ix, window, cx),
            None => {}
        }
    }

    fn panel_index_of<T: Panel>(&self) -> Option<usize> {
        self.sidebar.index_of::<T>()
    }

    pub fn toggle_sidebar(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.set_right_panel_maximized(false, cx);
        if !self.sidebar.toggle()
            && let Some(item) = self.active_item()
        {
            let focus = item.focus_handle(cx);
            window.focus(&focus, cx);
        }
        cx.notify();
    }
}
impl Workspace {
    pub fn set_right_panel_available(
        &mut self,
        available: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(transition) = self.right_panel.set_available(available, window, cx) {
            self.apply_panel_transition(transition, window, cx);
        }
    }

    pub fn set_right_panel_attention(&mut self, attention: bool, cx: &mut Context<Self>) {
        if self.right_panel.set_attention(attention) {
            cx.notify();
        }
    }
    pub fn toggle_right_panel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(transition) = self.right_panel.toggle(window, cx) {
            self.apply_panel_transition(transition, window, cx);
        }
    }

    pub fn set_right_panel_maximized(&mut self, maximized: bool, cx: &mut Context<Self>) {
        if self.right_panel.set_maximized(maximized, cx) {
            cx.notify();
        }
    }

    pub(super) fn close_right_panel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let transition = self.right_panel.close(window, cx);
        self.apply_panel_transition(transition, window, cx);
    }
    fn apply_panel_transition(
        &mut self,
        transition: PanelTransition,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if transition.visibility_changed() {
            cx.emit(WorkspaceEvent::RightPanelVisibilityChanged);
        }
        match transition.into_focus() {
            PanelFocus::Preserve => {}
            PanelFocus::Panel(focus) => window.focus(&focus, cx),
            PanelFocus::Central => {
                let focus = self
                    .active_item()
                    .map(|item| item.focus_handle(cx))
                    .unwrap_or_else(|| self.focus_handle.clone());
                window.focus(&focus, cx);
            }
        }
        cx.notify();
    }
}
impl Workspace {
    /// Puts the sidebar and the side panel on each other's side.
    pub fn swap_sides(&mut self, cx: &mut Context<Self>) {
        let swapped = self.body.swap(cx);
        self.right_panel.set_docked_left(swapped, cx);
        cx.notify();
    }

    /// Widens (or, with a negative delta, narrows) the side panel, for a
    /// panel that grows a column of its own.
    pub(super) fn widen_right_panel(
        &mut self,
        delta: Pixels,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.right_panel.maximized() || !self.right_panel.open() {
            return;
        }
        let arrangement =
            self.body
                .arrangement(self.sidebar.visible(), self.right_panel.open(), cx);
        self.body.widen_side_panel(arrangement, delta, window, cx);
        cx.notify();
    }
}
impl Workspace {
    /// Runs `action` on the command item after the current dispatch, so its
    /// handler can move focus freely and update the workspace.
    pub fn dispatch_command(
        &mut self,
        action: Box<dyn Action>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(item) = self.command_item(window, cx) else {
            return;
        };
        let workspace = cx.weak_entity();
        window.defer(cx, move |window, cx| {
            let target = workspace.read_with(cx, |this, cx| {
                this.tabs.contains(item.item_id()) && this.tab_is_visible(item.item_id(), cx)
            });
            if target.unwrap_or(false) {
                let focus = item.focus_handle(cx);
                if window.is_action_available_in(action.as_ref(), &focus) {
                    focus.dispatch_action(action.as_ref(), window, cx);
                }
            }
        });
    }

    /// Hands the commands an Item answers from the workspace's chrome to the
    /// command item. Never put this on an ancestor of the Items: a handler
    /// there would make every command look available.
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
}
