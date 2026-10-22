//! Immutable shell observations; no native or owner mutation handle escapes.
use super::*;
use gpui_kit::component::dock::{DockAreaState, NodeId};
impl Workspace {
    pub fn explorer_source(&self, cx: &App) -> Option<crate::ExplorerSource> {
        let source = self.context_source(cx);
        if let nocterm_project_model::ContextId::Project(id) = source.context() {
            let surface = self.surfaces.window().resolve(source, cx).ok()?;
            let inner = surface.read(cx);
            let local = inner
                .content()
                .terminals(cx)
                .into_iter()
                .find(|entry| entry.active)
                .and_then(|entry| entry.capabilities.state().info(cx))
                .is_some_and(|info| info.local);
            return Some(match (inner.root(), inner.tabs().active_session()) {
                (Some(root), _) if local => crate::ExplorerSource::local_folder(id, root),
                (Some(root), Some(session)) => crate::ExplorerSource::folder(id, root, session),
                _ => crate::ExplorerSource::unavailable_project(id),
            });
        }
        self.active_session(cx).map(crate::ExplorerSource::session)
    }
    pub fn general_item(&self, id: EntityId, cx: &App) -> Option<Rc<dyn ItemHandle>> {
        self.surfaces.outer().read(cx).content().item_handle(id)
    }

    pub fn general_terminal_target(
        &self,
        window: &Window,
        cx: &App,
    ) -> Result<GeneralTerminalTarget, crate::ContextSelectionError> {
        GeneralTerminalTarget::new(
            self.surfaces.window(),
            self.surfaces.outer().read(cx).context_selection_receipt(),
            window,
            cx,
        )
    }
    pub fn project_surface_factory(&self) -> ProjectSurfaceFactory {
        ProjectSurfaceFactory::new(self.surfaces.window())
    }
    pub fn general_surface_source(&self, cx: &App) -> ProjectSurfaceReceipt {
        self.surfaces.outer().read(cx).context_source()
    }
    pub fn context_terminals(
        &self,
        source: ProjectSurfaceReceipt,
        cx: &App,
    ) -> Result<Vec<crate::TerminalEntry>, crate::ContextSelectionError> {
        Ok(self
            .surfaces
            .window()
            .resolve(source, cx)?
            .read(cx)
            .content()
            .terminals(cx))
    }
    pub fn selected_context(&self, cx: &App) -> nocterm_project_model::ContextId {
        self.context_source(cx).context()
    }
    pub fn context_window(&self) -> ContextWindowReceipt {
        self.surfaces.owner()
    }
    pub fn context_source(&self, cx: &App) -> ProjectSurfaceReceipt {
        if let Some(project) = self.surfaces.outer().read(cx).tabs().active_project() {
            return project.source(cx);
        }
        self.surfaces.outer().read(cx).context_source()
    }
    pub fn context_selection_receipt(&self, cx: &App) -> ContextSelectionReceipt {
        if let Some(project) = self.surfaces.outer().read(cx).tabs().active_project() {
            return project.selection_receipt(cx);
        }
        self.surfaces.outer().read(cx).context_selection_receipt()
    }
    pub fn accepts_context_selection(&self, receipt: ContextSelectionReceipt, cx: &App) -> bool {
        self.surfaces
            .window()
            .resolve(receipt.surface(), cx)
            .is_ok_and(|surface| surface.read(cx).accepts_context_selection(receipt, cx))
    }
    pub(super) fn general_context(&self, cx: &App) -> bool {
        self.selected_context(cx) == nocterm_project_model::ContextId::General
    }

    pub fn items(&self, cx: &App) -> impl Iterator<Item = Rc<dyn ItemHandle>> {
        self.surfaces
            .outer()
            .read(cx)
            .tabs()
            .central_handles()
            .into_iter()
    }
    pub fn active_item(&self, cx: &App) -> Option<Rc<dyn ItemHandle>> {
        self.surfaces.outer().read(cx).tabs().active_handle()
    }
    pub fn find_item<T: Item>(&self, cx: &App) -> Option<Entity<T>> {
        self.surfaces.outer().read(cx).tabs().find()
    }
    pub fn connection_directory(&self) -> Option<Rc<dyn crate::ConnectionDirectory>> {
        self.ports.directory()
    }
    pub fn active_session(&self, cx: &App) -> Option<SessionContext> {
        self.surfaces.outer().read(cx).tabs().active_session()
    }
    pub fn connected_session_for_host(
        &self,
        host: &HostAddress,
        cx: &App,
    ) -> Option<SessionContext> {
        self.surfaces
            .outer()
            .read(cx)
            .tabs()
            .connected_session(host)
    }
    pub fn tabs(&self, cx: &App) -> Vec<TabSnapshot> {
        self.surfaces.outer().read(cx).tabs().snapshots(cx)
    }
    pub fn tab_panes(&self, placement: DockPlacement, cx: &App) -> Vec<TabPane> {
        self.surfaces.outer().read(cx).tabs().panes(placement, cx)
    }
    pub fn dock_region_present(&self, placement: DockPlacement, cx: &App) -> bool {
        self.surfaces
            .outer()
            .read(cx)
            .tabs()
            .region_present(placement, cx)
    }
    pub fn dock_region_open(&self, placement: DockPlacement, cx: &App) -> bool {
        self.surfaces
            .outer()
            .read(cx)
            .tabs()
            .region_open(placement, cx)
    }
    pub fn dock_region_size(&self, placement: DockPlacement, cx: &App) -> Option<Pixels> {
        self.surfaces
            .outer()
            .read(cx)
            .tabs()
            .region_size(placement, cx)
    }
    pub fn zoomed_pane(&self, cx: &App) -> Option<NodeId> {
        self.surfaces.outer().read(cx).tabs().zoomed(cx)
    }
    pub fn dock_layout(&self, cx: &App) -> DockAreaState {
        self.surfaces.outer().read(cx).tabs().dock_layout(cx)
    }
    pub fn selected_local_terminal(&self, cx: &App) -> Option<EntityId> {
        self.surfaces.outer().read(cx).tabs().selected_local_id(cx)
    }
    pub fn local_terminal_cwd(&self, cx: &App) -> Option<PathBuf> {
        self.surfaces.outer().read(cx).tabs().local_cwd(cx)
    }
    pub fn local_terminal_is_visible(&self, cx: &App) -> bool {
        self.surfaces.outer().read(cx).tabs().local_visible(cx)
    }
    pub fn is_background(&self, id: EntityId, cx: &App) -> bool {
        self.surfaces
            .outer()
            .read(cx)
            .content()
            .background_contains(id)
    }
    pub fn right_panel_is_available(&self) -> bool {
        self.right_panel.available()
    }
    pub fn right_panel_is_open(&self) -> bool {
        self.right_panel.open()
    }
    pub fn right_panel_is_maximized(&self) -> bool {
        self.right_panel.maximized()
    }
    pub fn new_tab_menu_is_open(&self) -> bool {
        self.chrome.new_tab_open()
    }
    pub fn sidebar_is_open(&self) -> bool {
        self.sidebar.visible()
    }
    pub fn sides_swapped(&self, cx: &App) -> bool {
        self.body.swapped(cx)
    }
    pub fn tab_group_items(&self, id: EntityId, cx: &App) -> Vec<EntityId> {
        self.surfaces.outer().read(cx).tabs().tab_group_items(id)
    }
    pub fn tab_group_count(&self, cx: &App) -> usize {
        self.surfaces.outer().read(cx).tabs().tab_group_count()
    }
    /// Previews the current visual close scope without exposing the dock's mutable state.
    pub fn tabs_to_close(&self, id: EntityId, scope: TabCloseScope, cx: &App) -> Vec<EntityId> {
        self.surfaces
            .outer()
            .read(cx)
            .tabs()
            .tabs_to_close(id, scope, cx)
    }
    pub fn can_split_active(&self, window: &Window, cx: &App) -> bool {
        self.command_item(window, cx).is_some_and(|item| {
            self.command_surface(window, cx)
                .read(cx)
                .tabs()
                .can_split(item.item_id(), cx)
        })
    }
    pub fn tab_location(&self, id: EntityId, cx: &App) -> Option<(DockPlacement, NodeId)> {
        self.surfaces.outer().read(cx).tabs().item_location(id, cx)
    }
    pub fn tab_is_visible(&self, id: EntityId, cx: &App) -> bool {
        self.surfaces.outer().read(cx).tabs().item_visible(id, cx)
    }
    pub(super) fn factory(&self) -> openers::Factory {
        self.ports.factory()
    }
    /// Focused visible Item wins; a captured menu target never falls back when stale.
    pub fn command_item(&self, window: &Window, cx: &App) -> Option<Rc<dyn ItemHandle>> {
        let surface = self.command_surface(window, cx);
        if self.chrome.stale_menu_selection(
            surface.read(cx).context_selection_receipt(),
            window,
            cx,
        ) {
            return None;
        }
        if let Some(item) = surface.read(cx).tabs().focused_item(window, cx) {
            return Some(item);
        }
        let menu = self
            .chrome
            .command_target(window, surface.read(cx).tabs().last_command(), cx);
        surface.read(cx).tabs().command_item(menu, cx)
    }
    pub(super) fn command_surface(&self, window: &Window, cx: &App) -> Entity<surface::TabSurface> {
        let outer = self.surfaces.outer();
        if outer
            .read(cx)
            .tabs()
            .focused_item(window, cx)
            .is_some_and(|item| outer.read(cx).tabs().project(item.item_id()).is_none())
        {
            return outer;
        }
        outer
            .read(cx)
            .tabs()
            .active_project()
            .map_or_else(|| outer.clone(), |project| project.engine())
    }
    pub(super) fn command_selection(&self, window: &Window, cx: &App) -> ContextSelectionReceipt {
        self.command_surface(window, cx)
            .read(cx)
            .context_selection_receipt()
    }
    pub fn command_available(&self, action: &dyn Action, window: &Window, cx: &App) -> bool {
        !self.focus_handle.is_focused(window)
            && self
                .command_item(window, cx)
                .is_some_and(|item| window.is_action_available_in(action, &item.focus_handle(cx)))
    }
    pub fn command_title(&self, window: &Window, cx: &App) -> Option<SharedString> {
        self.command_surface(window, cx)
            .read(cx)
            .tabs()
            .title(self.command_item(window, cx)?.item_id(), cx)
    }
    pub fn active_terminal(&self, cx: &App) -> Option<EntityId> {
        self.terminals(cx)
            .into_iter()
            .find(|entry| entry.active)
            .map(|entry| entry.item)
    }
    pub fn terminals(&self, cx: &App) -> Vec<crate::TerminalEntry> {
        self.context_terminals(self.context_source(cx), cx)
            .unwrap_or_default()
    }
}
