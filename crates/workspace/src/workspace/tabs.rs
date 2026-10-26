//! Registered tabs, cached session metadata and concrete lifecycle transitions.
use super::docking::{DockPane, TAB_PLACEMENTS};
use super::*;
use gpui_kit::component::dock::{DockAreaState, NodeId};
mod groups;
mod lifecycle;
mod local;
pub(super) mod project_skin;
mod selection;
pub(super) mod topology;

pub(super) struct TabEntry {
    handle: Rc<dyn ItemHandle>,
    session: Option<SessionContext>,
    dock_item: Entity<crate::dock_item::DockItem>,
    local: Option<Box<dyn crate::local_terminal::LocalTerminalHandle>>,
    cwd: Option<PathBuf>,
    subscription: Subscription,
    focus_subscription: Subscription,
    project: Option<project_surface::ProjectPresentation>,
}
impl TabEntry {
    pub(super) fn new(
        handle: Rc<dyn ItemHandle>,
        session: Option<SessionContext>,
        dock_item: Entity<crate::dock_item::DockItem>,
        local: Option<Box<dyn crate::local_terminal::LocalTerminalHandle>>,
        cwd: Option<PathBuf>,
        subscription: Subscription,
        focus_subscription: Subscription,
    ) -> Self {
        Self {
            handle,
            session,
            dock_item,
            local,
            cwd,
            subscription,
            focus_subscription,
            project: None,
        }
    }
    pub(super) fn with_project(
        mut self,
        project: Option<project_surface::ProjectPresentation>,
    ) -> Self {
        self.project = project;
        self
    }
    pub(super) fn id(&self) -> EntityId {
        self.handle.item_id()
    }
    pub(super) fn handle(&self) -> Rc<dyn ItemHandle> {
        self.handle.clone()
    }
    pub(super) fn refresh(&mut self, session: Option<SessionContext>, cx: &mut App) {
        self.session = session;
        self.dock_item.update(cx, |_, cx| cx.notify());
    }
    pub(super) fn close(self, window: &mut Window, cx: &mut App) {
        self.detach().handle.close(window, cx);
    }
    fn panel(&self) -> PanelId {
        PanelId::from(self.dock_item.entity_id())
    }
    fn detach(self) -> ClosedTab {
        let Self {
            handle,
            dock_item,
            subscription,
            focus_subscription,
            ..
        } = self;
        drop(subscription);
        drop(focus_subscription);
        ClosedTab { handle, dock_item }
    }
}
struct ClosedTab {
    handle: Rc<dyn ItemHandle>,
    dock_item: Entity<crate::dock_item::DockItem>,
}

pub(super) enum TabFocus {
    Preserve,
    Direct(FocusHandle),
    Activate(EntityId),
    Central,
}
pub(super) struct TabTransition {
    before: Vec<WorkspaceEvent>,
    focus: TabFocus,
    after: Vec<WorkspaceEvent>,
    notify: bool,
}
impl TabTransition {
    fn new(
        before: Vec<WorkspaceEvent>,
        focus: TabFocus,
        after: Vec<WorkspaceEvent>,
        notify: bool,
    ) -> Self {
        Self {
            before,
            focus,
            after,
            notify,
        }
    }
    pub(super) fn into_parts(self) -> (Vec<WorkspaceEvent>, TabFocus, Vec<WorkspaceEvent>, bool) {
        (self.before, self.focus, self.after, self.notify)
    }
}

pub(super) struct Tabs {
    entries: Vec<TabEntry>,
    groups: crate::tab_groups::TabGroups<PanelId, NodeId>,
    active_central_id: Option<EntityId>,
    announced_session: Option<SessionContext>,
    last_command_id: Option<EntityId>,
    topology: topology::DockTopology,
    local: local::LocalDockState,
}
impl Tabs {
    pub(super) fn new(topology: topology::DockTopology) -> Self {
        Self {
            entries: Vec::new(),
            groups: Default::default(),
            active_central_id: None,
            announced_session: None,
            last_command_id: None,
            topology,
            local: Default::default(),
        }
    }
    fn entry(&self, id: EntityId) -> Option<&TabEntry> {
        self.entries.iter().find(|entry| entry.id() == id)
    }
    fn active_entry(&self) -> Option<&TabEntry> {
        self.entry(self.active_central_id?)
    }
    fn local_entry(&self, id: EntityId) -> Option<&TabEntry> {
        self.entry(id).filter(|entry| entry.local.is_some())
    }
    fn selected_local(&self, cx: &App) -> Option<&TabEntry> {
        let saved = self.local_entry(self.local.selected()?)?;
        let pane = self.pane_for_item(saved.id(), cx)?;
        self.item_for_panel(pane.active_panel()?)
            .filter(|entry| entry.local.is_some())
            .or(Some(saved))
    }
    fn item_for_panel(&self, panel: PanelId) -> Option<&TabEntry> {
        self.entries.iter().find(|entry| entry.panel() == panel)
    }
    pub(super) fn panel_of(&self, id: EntityId) -> Option<PanelId> {
        self.entry(id).map(TabEntry::panel)
    }
    pub(super) fn contains(&self, id: EntityId) -> bool {
        self.entry(id).is_some()
    }
    pub(super) fn active_item(&self) -> Option<&dyn ItemHandle> {
        self.active_entry().map(|entry| entry.handle.as_ref())
    }
    pub(super) fn item_handle(&self, id: EntityId) -> Option<Rc<dyn ItemHandle>> {
        self.entries
            .iter()
            .find(|entry| entry.handle.item_id() == id)
            .map(|entry| entry.handle.clone())
    }
    pub(super) fn central_handles(&self) -> Vec<Rc<dyn ItemHandle>> {
        self.entries
            .iter()
            .filter(|entry| entry.local.is_none())
            .map(|entry| entry.handle.clone())
            .collect()
    }
    pub(super) fn active_handle(&self) -> Option<Rc<dyn ItemHandle>> {
        self.active_entry().map(|entry| entry.handle.clone())
    }
    pub(super) fn active_project(&self) -> Option<crate::ProjectSurface> {
        self.active_entry()?
            .project
            .as_ref()
            .map(project_surface::ProjectPresentation::surface)
    }
    pub(super) fn project(&self, item: EntityId) -> Option<crate::ProjectSurface> {
        self.entry(item)?
            .project
            .as_ref()
            .map(project_surface::ProjectPresentation::surface)
    }
    pub(super) fn project_item(
        &self,
        project: nocterm_project_model::ProjectId,
    ) -> Option<EntityId> {
        self.entries
            .iter()
            .find(|entry| {
                entry
                    .project
                    .as_ref()
                    .is_some_and(|view| view.surface().project() == project)
            })
            .map(TabEntry::id)
    }
    pub(super) fn find<T: Item>(&self) -> Option<Entity<T>> {
        self.entries
            .iter()
            .find_map(|entry| entry.handle.view().downcast::<T>().ok())
    }
    pub(super) fn len(&self) -> usize {
        self.entries.len()
    }
    pub(super) fn active_location(&self, cx: &App) -> Option<(DockPlacement, NodeId)> {
        self.item_location(self.active_central_id?, cx)
    }
    pub(super) fn header_origin(&self) -> Rc<Cell<Option<EntityId>>> {
        self.local.header_origin()
    }
    pub(super) fn dock_view(&self) -> AnyView {
        self.topology.view()
    }
    pub(super) fn has_visible_tabs(&self, cx: &App) -> bool {
        self.entries
            .iter()
            .any(|entry| self.item_visible(entry.id(), cx))
    }
    pub(super) fn item_location(&self, id: EntityId, cx: &App) -> Option<(DockPlacement, NodeId)> {
        self.topology.location(self.panel_of(id)?, cx)
    }
    pub(super) fn item_visible(&self, id: EntityId, cx: &App) -> bool {
        self.item_location(id, cx).is_some_and(|(placement, _)| {
            placement == DockPlacement::Center || self.topology.region_open(placement, cx)
        })
    }
    pub(super) fn item_displayed(&self, id: EntityId, cx: &App) -> bool {
        self.item_visible(id, cx)
            && self
                .item_location(id, cx)
                .is_some_and(|(_, node)| self.zoomed(cx).is_none_or(|zoomed| zoomed == node))
            && self
                .pane_for_item(id, cx)
                .is_some_and(|pane| pane.active_panel() == self.panel_of(id))
    }
    fn pane_for_item(&self, id: EntityId, cx: &App) -> Option<DockPane> {
        let (placement, node) = self.item_location(id, cx)?;
        self.topology.pane(placement, node, cx)
    }
    pub(super) fn snapshots(&self, cx: &App) -> Vec<TabSnapshot> {
        self.entries
            .iter()
            .map(|entry| TabSnapshot {
                id: entry.id(),
                alias: entry.dock_item.read(cx).alias.clone(),
                local: entry.local.is_some(),
                location: self.item_location(entry.id(), cx),
                group_color: entry.dock_item.read(cx).group_color(),
                visible: self.item_visible(entry.id(), cx),
            })
            .collect()
    }
    pub(super) fn panes(&self, placement: DockPlacement, cx: &App) -> Vec<TabPane> {
        self.topology
            .panes(placement, cx)
            .into_iter()
            .map(|pane| TabPane {
                placement: pane.placement,
                node: pane.node,
                items: pane
                    .panels
                    .into_iter()
                    .filter_map(|panel| self.item_for_panel(panel).map(TabEntry::id))
                    .collect(),
            })
            .collect()
    }
    pub(super) fn region_present(&self, placement: DockPlacement, cx: &App) -> bool {
        self.topology.region_present(placement, cx)
    }
    pub(super) fn region_open(&self, placement: DockPlacement, cx: &App) -> bool {
        self.topology.region_open(placement, cx)
    }
    pub(super) fn region_size(&self, placement: DockPlacement, cx: &App) -> Option<Pixels> {
        self.topology.region_size(placement, cx)
    }
    pub(super) fn zoomed(&self, cx: &App) -> Option<NodeId> {
        self.topology.zoomed(cx)
    }
    pub(super) fn dock_layout(&self, cx: &App) -> DockAreaState {
        self.topology.dump(cx)
    }
    pub(super) fn selected_local_id(&self, cx: &App) -> Option<EntityId> {
        self.selected_local(cx).map(TabEntry::id)
    }
    pub(super) fn local_cwd(&self, cx: &App) -> Option<PathBuf> {
        self.selected_local(cx)?.local.as_deref()?.cwd(cx)
    }
    pub(super) fn local_ids(&self) -> Vec<EntityId> {
        self.entries
            .iter()
            .filter(|entry| entry.local.is_some())
            .map(TabEntry::id)
            .collect()
    }
    pub(super) fn selected_local_handle(
        &self,
        cx: &App,
    ) -> Option<&dyn crate::local_terminal::LocalTerminalHandle> {
        self.selected_local(cx)?.local.as_deref()
    }
    pub(super) fn local_destination(&self, cx: &App) -> Option<(DockPlacement, NodeId)> {
        self.item_location(self.selected_local_id(cx)?, cx)
            .filter(|(placement, _)| *placement == DockPlacement::Bottom)
    }
    pub(super) fn active_session(&self) -> Option<SessionContext> {
        self.announced_session.clone()
    }
    pub(super) fn connected_session(&self, host: &HostAddress) -> Option<SessionContext> {
        let matches = |session: &&SessionContext| session.connected && session.reaches(host);
        self.active_entry()
            .and_then(|entry| entry.session.as_ref())
            .filter(matches)
            .or_else(|| {
                self.entries
                    .iter()
                    .filter_map(|entry| entry.session.as_ref())
                    .find(matches)
            })
            .cloned()
    }
    pub(super) fn session_spec_for(&self, host: &HostAddress, cx: &App) -> Option<SessionSpec> {
        self.active_entry()
            .into_iter()
            .chain(self.entries.iter())
            .filter(|entry| {
                entry
                    .session
                    .as_ref()
                    .is_some_and(|session| session.connected && session.reaches(host))
            })
            .find_map(|entry| entry.handle.session_spec(cx))
    }
    pub(super) fn program_parent(&self, key: &crate::HostKey) -> Option<EntityId> {
        self.active_entry()
            .filter(|entry| {
                entry
                    .session
                    .as_ref()
                    .is_some_and(|session| crate::HostKey::of(session) == *key)
            })
            .map(TabEntry::id)
    }
    pub(super) fn focused_item(&self, window: &Window, cx: &App) -> Option<Rc<dyn ItemHandle>> {
        self.entries
            .iter()
            .find(|entry| {
                entry.dock_item.read(cx).contains_focus(window, cx)
                    && self.item_visible(entry.id(), cx)
            })
            .map(|entry| entry.handle.clone())
    }
    pub(super) fn command_item(
        &self,
        menu: Option<EntityId>,
        cx: &App,
    ) -> Option<Rc<dyn ItemHandle>> {
        if let Some(id) = menu {
            return self
                .entry(id)
                .filter(|_| self.item_visible(id, cx))
                .map(|entry| entry.handle.clone());
        }
        self.active_entry()
            .filter(|entry| self.item_visible(entry.id(), cx))
            .map(|entry| entry.handle.clone())
    }
    pub(super) fn last_command(&self) -> Option<EntityId> {
        self.last_command_id
    }
    pub(super) fn title(&self, id: EntityId, cx: &App) -> Option<SharedString> {
        let entry = self.entry(id)?;
        Some(
            entry
                .dock_item
                .read(cx)
                .alias
                .clone()
                .unwrap_or_else(|| entry.handle.tab_title(cx)),
        )
    }
    pub(super) fn terminal(&self, id: EntityId, cx: &App) -> bool {
        self.entry(id)
            .is_some_and(|entry| entry.handle.terminal_capabilities(cx).is_some())
    }
    pub(super) fn foreground_terminals(&self, cx: &App) -> Vec<crate::TerminalEntry> {
        self.entries
            .iter()
            // Registered project Items own inner terminals and advertise no
            // outer terminal capability. Skip their cached membership before
            // touching the view, including while that ProjectView is updated.
            .filter(|entry| entry.project.is_none())
            .filter_map(|entry| {
                Some(crate::TerminalEntry {
                    item: entry.id(),
                    capabilities: entry.handle.terminal_capabilities(cx)?,
                    title: self.title(entry.id(), cx)?,
                    active: false,
                    bottom: self
                        .item_location(entry.id(), cx)
                        .is_some_and(|(placement, _)| placement == DockPlacement::Bottom),
                    background: false,
                })
            })
            .collect()
    }
}
