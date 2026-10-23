//! One retained native tab engine with its own event and focus authority.
use super::*;
use crate::ContextSelectionError;
use commands::SurfaceState;
use gpui_kit::component::Placement;
use membership::SurfaceContent;
pub(super) use membership::SurfaceFrame;
use nocterm_project_model::{ContextId, ProjectTarget, RootPath};
use selection::SurfaceSelection;
use tabs::{TabFocus, TabTransition};

mod actions;
pub(in crate::workspace) mod admission;
mod commands;
mod membership;
mod selection;
pub(super) mod window;

pub(crate) struct TabSurface {
    state: SurfaceState,
    selection: SurfaceSelection,
    content: SurfaceContent,
    focus_handle: FocusHandle,
    window: Rc<window::SurfaceWindow>,
}

#[derive(Clone)]
pub(super) enum SurfaceEvent {
    Observed(ProjectSurfaceReceipt, WorkspaceEvent),
    OpenLocal(ContextSelectionReceipt, crate::LocalTerminalTarget),
    OpenTerminal(ContextSelectionReceipt, Option<crate::LocalTerminalTarget>),
}

impl EventEmitter<SurfaceEvent> for TabSurface {}
impl TabSurface {
    pub(super) fn new(
        owner: Rc<window::SurfaceWindow>,
        context: ContextId,
        outer: Option<EntityId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let source = ProjectSurfaceReceipt::new(owner.owner(), context, cx.entity_id(), outer);
        Self {
            state: SurfaceState::default(),
            selection: SurfaceSelection::new(source),
            content: SurfaceContent::new(tabs::Tabs::new(bindings::prepare_topology(
                source, window, cx,
            ))),
            focus_handle: cx.focus_handle(),
            window: owner,
        }
    }
    pub(crate) fn source(&self) -> ProjectSurfaceReceipt {
        self.selection.source()
    }
    pub(crate) fn selection(&self) -> ContextSelectionReceipt {
        self.selection.selection(self.window.generation())
    }
    pub(crate) fn accepts_context_selection(
        &self,
        target: ContextSelectionReceipt,
        cx: &App,
    ) -> bool {
        self.selection.selected(target, &self.window, cx)
    }
    pub(crate) fn content(&self) -> &SurfaceContent {
        &self.content
    }
    pub(crate) fn content_mut(&mut self) -> &mut SurfaceContent {
        &mut self.content
    }
    pub(super) fn tabs(&self) -> &tabs::Tabs {
        self.content.tabs()
    }

    pub(super) fn contains_item(&self, id: EntityId) -> bool {
        self.content.contains(id)
    }
    pub(super) fn frame(&self, view: AnyView, cx: &App) -> SurfaceFrame {
        self.content.frame(view, cx)
    }
    fn refresh_selection(&mut self, cx: &mut App) {
        if self.selection.refresh(self.content.tabs()) {
            self.window.select_outer(self.source(), cx);
        }
    }
    fn emit_observation(&self, event: WorkspaceEvent, cx: &mut Context<Self>) {
        cx.emit(SurfaceEvent::Observed(self.source(), event));
    }
    fn membership_changed(&self, source: ProjectSurfaceReceipt, cx: &mut Context<Self>) {
        if self.selection.is_active(source, &self.window, cx) {
            self.emit_observation(WorkspaceEvent::ItemsChanged, cx);
            cx.notify();
        } else {
            self.emit_observation(WorkspaceEvent::SurfaceItemsChanged(source), cx);
        }
    }
    pub(crate) fn new_local_terminal_from(
        &mut self,
        id: EntityId,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.content.tab_location(id, cx).is_none() || !self.terminal_creation_allowed() {
            return;
        }
        cx.emit(self.selection.terminal_request(id, &self.window));
    }

    pub(crate) fn terminal_creation_allowed(&self) -> bool {
        !self.selection.closed()
    }
    pub(in crate::workspace) fn monitor_target(&self) -> Option<ProjectTarget> {
        self.state.monitor_target()
    }
    pub(in crate::workspace) fn set_monitor_target(
        &mut self,
        target: ProjectTarget,
        cx: &mut Context<Self>,
    ) {
        if self.state.set_monitor_target(target) {
            self.emit_observation(WorkspaceEvent::ActiveSessionChanged, cx);
        }
    }
    pub(in crate::workspace) fn root(&self) -> Option<RootPath> {
        self.state.root()
    }
    pub(in crate::workspace) fn set_root(&mut self, root: RootPath, cx: &mut Context<Self>) {
        if self.state.set_root(root) {
            self.emit_observation(WorkspaceEvent::SurfaceItemsChanged(self.source()), cx);
            cx.notify();
        }
    }
    pub(in crate::workspace) fn initial_terminal_target(
        &mut self,
        window: &Window,
        cx: &App,
    ) -> Result<ContextSelectionReceipt, ContextSelectionError> {
        self.selection
            .validate_source(self.source(), &self.window, window)?;
        self.state.initial_terminal(
            !self.selection.general() && self.window.registered(self.source(), cx),
            self.selection(),
        )
    }
    pub(in crate::workspace) fn preferred_focus(&self, cx: &App) -> FocusHandle {
        self.content.preferred_focus(&self.focus_handle, false, cx)
    }

    pub(in crate::workspace) fn set_presented(&mut self, presented: bool, cx: &mut Context<Self>) {
        if self.selection.set_presented(presented) {
            cx.notify();
        }
    }
    pub(in crate::workspace) fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.window.check_window(window).is_err() || !self.selection.close() {
            return;
        }
        self.content.close_all(window, cx);
        self.emit_observation(WorkspaceEvent::SurfaceItemsChanged(self.source()), cx);
        cx.notify();
    }
    pub(in crate::workspace) fn rename_item(
        &mut self,
        id: EntityId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.content.rename(id, self.selection(), window, cx);
    }

    pub(in crate::workspace) fn toggle_local(
        &mut self,
        focused: Option<EntityId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(focus) = self.content.toggle_bottom(focused, window, cx) else {
            return false;
        };
        self.apply_tab_focus(focus, window, cx);
        true
    }

    fn apply_tab_transition(
        &mut self,
        transition: TabTransition,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (before, focus, after, notify) = transition.into_parts();
        self.refresh_selection(cx);
        for event in before {
            self.emit_observation(event, cx);
        }
        self.apply_tab_focus(focus, window, cx);
        for event in after {
            self.emit_observation(event, cx);
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
    fn emit_tab_events(&mut self, events: Vec<WorkspaceEvent>, cx: &mut Context<Self>) {
        self.refresh_selection(cx);
        let changed = !events.is_empty();
        for event in events {
            self.emit_observation(event, cx);
        }
        if changed {
            cx.notify();
        }
    }
    pub(crate) fn activate_item_by_id(
        &mut self,
        id: EntityId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if let Some(transition) = self.content.activate(id, window, cx) {
            self.apply_tab_transition(transition, window, cx);
            true
        } else {
            false
        }
    }
    pub(crate) fn close_item_by_id(
        &mut self,
        id: EntityId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_item_at(self.source(), id, window, cx);
    }
    pub(crate) fn close_item_at(
        &mut self,
        source: ProjectSurfaceReceipt,
        id: EntityId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.selection.matches(source) {
            return;
        }
        if self.content.background_contains(id) {
            self.close_background_at(source, id, window, cx);
            return;
        }
        let focused = self.selection.is_active(source, &self.window, cx)
            && self
                .content
                .command_item(window, cx)
                .is_some_and(|item| item.item_id() == id);
        if let Some(transition) = self.content.close(id, focused, window, cx) {
            self.apply_surface_transition(source, transition, window, cx);
        }
    }
    pub(in crate::workspace) fn item_changed_at(
        &mut self,
        source: ProjectSurfaceReceipt,
        id: EntityId,
        session: Option<SessionContext>,
        cx: &mut Context<Self>,
    ) {
        if !self.selection.matches(source) {
            return;
        }
        if self.content.background_changed(id, session.clone(), cx) {
            self.membership_changed(source, cx);
            return;
        }
        if let Some(transition) = self.content.changed(id, session, cx) {
            let (before, _, after, notify) = transition.into_parts();
            self.refresh_selection(cx);
            if self.selection.is_active(source, &self.window, cx) {
                for event in before.into_iter().chain(after) {
                    self.emit_observation(event, cx);
                }
                if notify {
                    cx.notify();
                }
            } else {
                self.emit_observation(WorkspaceEvent::SurfaceItemsChanged(source), cx);
            }
        }
    }
    pub(in crate::workspace) fn item_focused_at(
        &mut self,
        source: ProjectSurfaceReceipt,
        id: EntityId,
        cx: &mut Context<Self>,
    ) {
        if !self.selection.is_active(source, &self.window, cx) {
            return;
        }
        if let Some(transition) = self.content.focused(id) {
            let (before, _, after, notify) = transition.into_parts();
            self.refresh_selection(cx);
            for event in before.into_iter().chain(after) {
                self.emit_observation(event, cx);
            }
            if notify {
                cx.notify();
            }
        }
    }
    fn apply_surface_transition(
        &mut self,
        source: ProjectSurfaceReceipt,
        transition: TabTransition,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.selection.is_active(source, &self.window, cx) {
            self.apply_tab_transition(transition, window, cx);
        } else {
            self.emit_observation(WorkspaceEvent::SurfaceItemsChanged(source), cx);
        }
    }
    pub(in crate::workspace) fn native_dock_changed(
        &mut self,
        source: ProjectSurfaceReceipt,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.selection.matches(source) {
            return;
        }
        if self
            .selection
            .refresh_presentation(self.content.presentation(cx))
        {
            self.window.select_outer(source, cx);
        }
        let events = self.content.native_changed(window, cx);
        if self.selection.is_active(source, &self.window, cx) {
            self.emit_tab_events(events, cx);
            cx.notify();
        } else if !events.is_empty() {
            self.emit_observation(WorkspaceEvent::SurfaceItemsChanged(source), cx);
        }
    }
    pub(in crate::workspace) fn native_dock_visibility_changed(
        &mut self,
        source: ProjectSurfaceReceipt,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.selection.is_active(source, &self.window, cx)
            && self.tabs().hidden_bottom_has_focus(window, cx)
        {
            self.focus_central(window, cx);
        }
    }
    pub(in crate::workspace) fn focus_central(&self, window: &mut Window, cx: &mut Context<Self>) {
        let focus = self.content.preferred_focus(&self.focus_handle, true, cx);
        window.focus(&focus, cx);
    }
    pub(crate) fn focus_terminal(
        &mut self,
        id: EntityId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        self.tabs().terminal(id, cx) && self.activate_item_by_id(id, window, cx)
    }
    pub(crate) fn close_tabs(
        &mut self,
        id: EntityId,
        scope: TabCloseScope,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        for id in self.tabs().tabs_to_close(id, scope, cx) {
            self.close_item_by_id(id, window, cx);
        }
    }
    pub(crate) fn split_item(
        &mut self,
        id: EntityId,
        placement: Placement,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.content.split(id, placement, window, cx) {
            cx.notify();
        }
    }

    pub(crate) fn close_tab_group(
        &mut self,
        id: EntityId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        for id in self.tabs().tab_group_items(id) {
            self.close_item_by_id(id, window, cx);
        }
    }

    pub(crate) fn close_local_terminal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(id) = self.tabs().selected_local_id(cx) {
            self.close_item_by_id(id, window, cx);
        }
    }

    pub(in crate::workspace) fn admit_item<T: Item>(
        &mut self,
        mut admission: admission::SurfaceItemAdmission,
        item: Entity<T>,
        local: Option<Box<dyn crate::local_terminal::LocalTerminalHandle>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<bool, ContextSelectionError> {
        admission.validate_outer(self.source().window(), item.entity_id(), window, cx)?;
        if self.contains_item(item.entity_id())
            || self
                .window
                .foreign_item(self.source(), item.entity_id(), cx)
            || admission.duplicate_project(self.tabs())
        {
            return Ok(false);
        }
        let source = admission.target().surface();
        let destination = admission
            .resolve(&self.selection, &self.window, self.tabs(), window, cx)
            .inspect_err(|_| {
                if !admission.outer() {
                    ItemHandle::close(&item, window, cx);
                }
            })?;
        let outer = admission.outer();
        let focus = self.accepts_context_selection(admission.target(), cx);
        let entry = bindings::prepare_tab(
            item,
            local,
            self.tabs().header_origin(),
            admission.take_presentation(),
            source,
            window,
            cx,
        )
        .into_entry();
        let Some(id) = self.content.admit(entry, destination, focus, window, cx) else {
            return Ok(false);
        };
        if focus {
            self.activate_item_by_id(id, window, cx);
        }
        if outer {
            self.membership_changed(source, cx);
        } else {
            self.emit_observation(WorkspaceEvent::ItemsChanged, cx);
            cx.notify();
        }
        Ok(true)
    }

    pub(in crate::workspace) fn admit_background<T: Item>(
        &mut self,
        source: ProjectSurfaceReceipt,
        item: Entity<T>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<bool, ContextSelectionError> {
        let id = item.entity_id();
        if self.contains_item(id) || self.window.foreign_item(self.source(), id, cx) {
            return Ok(false);
        }
        if let Err(error) = self.selection.validate_source(source, &self.window, window) {
            ItemHandle::close(&item, window, cx);
            return Err(error);
        }
        let entry = bindings::prepare_tab(
            item,
            None,
            self.tabs().header_origin(),
            None,
            source,
            window,
            cx,
        )
        .into_entry();
        self.content.insert_background(entry);
        self.membership_changed(source, cx);
        Ok(true)
    }

    pub(in crate::workspace) fn promote_background(
        &mut self,
        id: EntityId,
        focus: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let focused = focus && self.selection.is_active(self.source(), &self.window, cx);
        if !self.content.promote(id, focused, window, cx) {
            return false;
        }
        if focused {
            self.activate_item_by_id(id, window, cx);
        }
        self.membership_changed(self.source(), cx);
        true
    }

    pub(in crate::workspace) fn close_background_at(
        &mut self,
        source: ProjectSurfaceReceipt,
        id: EntityId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.selection.matches(source) && self.content.close_background(id, window, cx) {
            self.membership_changed(source, cx);
        }
    }
}

impl Focusable for TabSurface {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}
impl Render for TabSurface {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let root = div()
            .track_focus(&self.focus_handle)
            .size_full()
            .min_h_0()
            .min_w_0()
            .child(self.tabs().dock_view());
        if self.selection.general() {
            root
        } else {
            actions::bind(root, cx)
        }
    }
}
