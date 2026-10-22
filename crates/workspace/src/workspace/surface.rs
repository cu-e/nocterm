//! One retained native tab engine with its own event and focus authority.
use super::*;
use crate::ContextSelectionError;
use crate::LocalTerminalTarget;
use commands::SurfaceState;
use gpui_kit::component::Placement;
use membership::SurfaceContent;
pub(super) use membership::SurfaceFrame;
use nocterm_project_model::{ContextId, RootPath};
use tabs::{TabFocus, TabTransition};

mod actions;
mod admission;
mod commands;
mod membership;
pub(super) mod window;

pub(crate) struct TabSurface {
    state: SurfaceState,
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
            state: SurfaceState::new(source),
            content: SurfaceContent::new(tabs::Tabs::new(bindings::prepare_topology(
                source, window, cx,
            ))),
            focus_handle: cx.focus_handle(),
            window: owner,
        }
    }
    pub(crate) fn context_source(&self) -> ProjectSurfaceReceipt {
        self.state.source()
    }
    fn source(&self) -> ProjectSurfaceReceipt {
        self.context_source()
    }
    pub(crate) fn context_selection_receipt(&self) -> ContextSelectionReceipt {
        self.state.selection(self.window.generation())
    }
    fn selection(&self) -> ContextSelectionReceipt {
        self.context_selection_receipt()
    }
    pub(crate) fn accepts_context_selection(
        &self,
        target: ContextSelectionReceipt,
        cx: &App,
    ) -> bool {
        self.state.accepts(
            target,
            self.window.generation(),
            self.window.presented(self.source(), cx),
        )
    }
    fn selected(&self, target: ContextSelectionReceipt, cx: &App) -> bool {
        self.accepts_context_selection(target, cx)
    }
    fn is_active(&self, source: ProjectSurfaceReceipt, cx: &App) -> bool {
        self.state.matches(source) && self.window.presented(source, cx)
    }
    fn general_context(&self) -> bool {
        self.state.general()
    }
    fn validate_general(
        &self,
        source: ProjectSurfaceReceipt,
        window: &Window,
    ) -> Result<(), ContextSelectionError> {
        self.validate_source(source, window)?;
        if !self.general_context() {
            return Err(ContextSelectionError::ProtectedContext);
        }
        Ok(())
    }
    fn validate_source(
        &self,
        source: ProjectSurfaceReceipt,
        window: &Window,
    ) -> Result<(), ContextSelectionError> {
        self.window.check_window(window)?;
        self.state.validate(source)
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
        let selected = self
            .tabs()
            .last_command()
            .or_else(|| self.tabs().active_item().map(ItemHandle::item_id));
        if self.state.select(selected) && self.general_context() {
            self.window.select_outer(self.source(), cx);
        }
    }
    fn emit_observation(&self, event: WorkspaceEvent, cx: &mut Context<Self>) {
        cx.emit(SurfaceEvent::Observed(self.source(), event));
    }
    fn membership_changed(&self, source: ProjectSurfaceReceipt, cx: &mut Context<Self>) {
        if self.is_active(source, cx) {
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
        if self.general_context() {
            cx.emit(SurfaceEvent::OpenLocal(
                self.selection(),
                crate::LocalTerminalTarget::Beside(id),
            ));
        } else {
            cx.emit(SurfaceEvent::OpenTerminal(
                self.selection(),
                Some(crate::LocalTerminalTarget::Beside(id)),
            ));
        }
    }

    pub(crate) fn terminal_creation_allowed(&self) -> bool {
        !self.state.closed()
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
        self.validate_source(self.source(), window)?;
        self.state.initial_terminal(
            self.window.registered(self.source(), cx),
            self.window.generation(),
        )
    }
    pub(in crate::workspace) fn preferred_focus(&self, cx: &App) -> FocusHandle {
        self.content
            .active_item()
            .map_or_else(|| self.focus_handle.clone(), |item| item.focus_handle(cx))
    }

    pub(in crate::workspace) fn command_item(
        &self,
        window: &Window,
        cx: &App,
    ) -> Option<Rc<dyn ItemHandle>> {
        self.tabs()
            .focused_item(window, cx)
            .or_else(|| self.tabs().command_item(self.tabs().last_command(), cx))
    }

    pub(in crate::workspace) fn set_presented(&mut self, presented: bool, cx: &mut Context<Self>) {
        if self.state.set_presented(presented) {
            cx.notify();
        }
    }
    pub(in crate::workspace) fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.window.check_window(window).is_err() || !self.state.close() {
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
    pub(in crate::workspace) fn close_item_at(
        &mut self,
        source: ProjectSurfaceReceipt,
        id: EntityId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.state.matches(source) {
            return;
        }
        if self.content.background_contains(id) {
            self.close_background_at(source, id, window, cx);
            return;
        }
        let focused = self.is_active(source, cx)
            && self
                .command_item(window, cx)
                .is_some_and(|item| item.item_id() == id);
        if let Some(transition) = self.content.close(id, focused, window, cx) {
            self.apply_surface_transition(source, transition, window, cx);
        }
    }
    pub(crate) fn item_removed(
        &mut self,
        source: ProjectSurfaceReceipt,
        id: EntityId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_item_at(source, id, window, cx);
    }
    pub(in crate::workspace) fn item_changed_at(
        &mut self,
        source: ProjectSurfaceReceipt,
        id: EntityId,
        session: Option<SessionContext>,
        cx: &mut Context<Self>,
    ) {
        if !self.state.matches(source) {
            return;
        }
        if self.content.background_changed(id, session.clone(), cx) {
            self.membership_changed(source, cx);
            return;
        }
        if let Some(transition) = self.content.changed(id, session, cx) {
            let (before, _, after, notify) = transition.into_parts();
            self.refresh_selection(cx);
            if self.is_active(source, cx) {
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
        if !self.is_active(source, cx) {
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
        if self.is_active(source, cx) {
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
        if !self.state.matches(source) {
            return;
        }
        let presentation = (
            self.tabs().zoomed(cx),
            self.tabs().region_open(DockPlacement::Bottom, cx),
        );
        if self.state.change_presentation(presentation) && self.general_context() {
            self.window.select_outer(source, cx);
        }
        let events = self.content.native_changed(window, cx);
        if self.is_active(source, cx) {
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
        if self.is_active(source, cx) && self.tabs().hidden_bottom_has_focus(window, cx) {
            self.focus_central(window, cx);
        }
    }
    pub(in crate::workspace) fn focus_central(&self, window: &mut Window, cx: &mut Context<Self>) {
        let focus = self
            .content
            .active_item()
            .filter(|item| self.tabs().item_visible(item.item_id(), cx))
            .map_or_else(|| self.focus_handle.clone(), |item| item.focus_handle(cx));
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

    pub(in crate::workspace) fn add_general_background_from<T: Item>(
        &mut self,
        target: ContextSelectionReceipt,
        item: Entity<T>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<EntityId, ContextSelectionError> {
        let id = item.entity_id();
        if self.contains_item(id) || self.window.foreign_item(self.source(), id, cx) {
            return Ok(id);
        }
        if let Err(error) = self.validate_general(target.surface(), window) {
            ItemHandle::close(&item, window, cx);
            return Err(error);
        }
        Ok(self.register_background_item(target.surface(), item, window, cx))
    }
    pub(in crate::workspace) fn add_project_item<T: Item>(
        &mut self,
        target: ContextSelectionReceipt,
        item: Entity<T>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<bool, ContextSelectionError> {
        self.register_context_item(
            admission::SurfaceItemAdmission::central(target),
            item,
            None,
            window,
            cx,
        )
    }
    pub(in crate::workspace) fn add_project_local<T: crate::LocalTerminal>(
        &mut self,
        target: ContextSelectionReceipt,
        item: Entity<T>,
        destination: LocalTerminalTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<bool, ContextSelectionError> {
        let local = Box::new(item.clone());
        self.register_context_item(
            admission::SurfaceItemAdmission::local(target, destination),
            item,
            Some(local),
            window,
            cx,
        )
    }
    pub(in crate::workspace) fn add_project_background<T: Item>(
        &mut self,
        item: Entity<T>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<bool, ContextSelectionError> {
        let id = item.entity_id();
        if self.contains_item(id) || self.window.foreign_item(self.source(), id, cx) {
            return Ok(false);
        }
        if let Err(error) = self.validate_source(self.source(), window) {
            ItemHandle::close(&item, window, cx);
            return Err(error);
        }
        self.register_background_item(self.source(), item, window, cx);
        Ok(true)
    }
    pub(in crate::workspace) fn add_workspace<T: Item>(
        &mut self,
        item: Entity<T>,
        project: project_surface::ProjectPresentation,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<bool, ContextSelectionError> {
        project
            .surface()
            .validate_outer(self.source().window(), item.entity_id(), window, cx)?;
        if self.contains_item(item.entity_id())
            || self
                .window
                .foreign_item(self.source(), item.entity_id(), cx)
            || self
                .tabs()
                .project_item(project.surface().project())
                .is_some()
        {
            return Ok(false);
        }
        self.validate_general(self.source(), window)?;
        let target = self
            .tabs()
            .active_location(cx)
            .filter(|(placement, _)| *placement == DockPlacement::Center)
            .map(|(_, node)| node);
        let prepared = bindings::prepare_tab(
            item,
            None,
            self.tabs().header_origin(),
            Some(project),
            self.source(),
            window,
            cx,
        );
        let focus = self.is_active(self.source(), cx);
        if let Some(id) = self.content.register(
            prepared.into_entry(),
            DockPlacement::Center,
            target,
            focus,
            window,
            cx,
        ) {
            if focus {
                self.activate_item_by_id(id, window, cx);
            }
            self.membership_changed(self.source(), cx);
        }
        Ok(true)
    }
    pub(in crate::workspace) fn add_general_item_from<T: Item>(
        &mut self,
        target: ContextSelectionReceipt,
        item: Entity<T>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<bool, ContextSelectionError> {
        self.register_context_item(
            admission::SurfaceItemAdmission::general(target),
            item,
            None,
            window,
            cx,
        )
    }
    pub(in crate::workspace) fn add_general_local_terminal_from<T: crate::LocalTerminal>(
        &mut self,
        target: ContextSelectionReceipt,
        item: Entity<T>,
        destination: LocalTerminalTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<bool, ContextSelectionError> {
        let local = Box::new(item.clone());
        self.register_context_item(
            admission::SurfaceItemAdmission::general_local(target, destination),
            item,
            Some(local),
            window,
            cx,
        )
    }
    fn register_context_item<T: Item>(
        &mut self,
        admission: admission::SurfaceItemAdmission,
        item: Entity<T>,
        local: Option<Box<dyn crate::local_terminal::LocalTerminalHandle>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<bool, ContextSelectionError> {
        if self.contains_item(item.entity_id())
            || self
                .window
                .foreign_item(self.source(), item.entity_id(), cx)
        {
            return Ok(false);
        }
        let source = admission.target().surface();
        let validation = if admission.project() {
            self.validate_source(source, window)
        } else {
            self.validate_general(source, window)
        };
        if let Err(error) =
            validation.and_then(|_| admission.destination(self.tabs(), cx).map(|_| ()))
        {
            ItemHandle::close(&item, window, cx);
            return Err(error);
        }
        let focus = self.selected(admission.target(), cx);
        let entry = bindings::prepare_tab(
            item,
            local,
            self.tabs().header_origin(),
            None,
            source,
            window,
            cx,
        )
        .into_entry();
        let Some(id) = self.content.admit(entry, &admission, focus, window, cx)? else {
            return Ok(false);
        };
        if focus {
            self.activate_item_by_id(id, window, cx);
        }
        self.emit_observation(WorkspaceEvent::ItemsChanged, cx);
        cx.notify();
        Ok(true)
    }

    fn register_background_item<T: Item>(
        &mut self,
        source: ProjectSurfaceReceipt,
        item: Entity<T>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> EntityId {
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
        let id = self.content.insert_background(entry);
        self.membership_changed(source, cx);
        id
    }

    pub(in crate::workspace) fn promote_background(
        &mut self,
        id: EntityId,
        focus: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let focused = focus && self.is_active(self.source(), cx);
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
        if self.state.matches(source) && self.content.close_background(id, window, cx) {
            self.membership_changed(source, cx);
        }
    }
    pub(crate) fn set_local_terminal<T: crate::LocalTerminal>(
        &mut self,
        item: Entity<T>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.contains_item(item.entity_id())
            || self
                .window
                .foreign_item(self.source(), item.entity_id(), cx)
        {
            return;
        }
        if !self.general_context() {
            if !self.contains_item(item.entity_id()) {
                ItemHandle::close(&item, window, cx);
            }
            return;
        }
        let ids: Vec<_> = self.tabs().local_ids();
        for id in ids {
            self.close_item_by_id(id, window, cx);
        }
        self.add_local_terminal_at(item, LocalTerminalTarget::Bottom, window, cx);
    }

    pub(crate) fn add_local_terminal_at<T: crate::LocalTerminal>(
        &mut self,
        item: Entity<T>,
        target: LocalTerminalTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        self.add_general_local_terminal_from(self.selection(), item, target, window, cx)
            .unwrap_or(false)
    }
    pub(crate) fn add_item<T: Item>(
        &mut self,
        item: Entity<T>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let _ = self.add_general_item_from(self.selection(), item, window, cx);
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
        if self.general_context() {
            root
        } else {
            actions::bind(root, cx)
        }
    }
}
