//! Explicit installations and typed handoffs to the original tab owner.
use super::*;
use crate::ContextSelectionError;
use crate::plugin::LOCAL_TERMINAL;
use crate::{LocalTerminalTarget, Panel, ProgramSpec};
use nocterm_session::{HostAddress, ShellLaunch};
use registration::{PanelRegistration, PanelSource, RegistrationError, WidgetRegistration};
use shell_layout::LayoutChange;
use surface::admission::SurfaceItemAdmission;

impl Workspace {
    pub fn add_general_background_from<T: Item>(
        &mut self,
        target: ContextSelectionReceipt,
        item: Entity<T>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<EntityId, crate::ContextSelectionError> {
        let id = item.entity_id();
        self.surfaces.outer().update(cx, |surface, cx| {
            surface
                .admit_background(target.surface(), item, window, cx)
                .map(|_| id)
        })
    }
    pub fn validate_context_source(
        &self,
        source: ProjectSurfaceReceipt,
        window: &Window,
        cx: &App,
    ) -> Result<(), ContextSelectionError> {
        self.surfaces.window().check_window(window)?;
        self.surfaces.window().resolve(source, cx)?;
        if !self.surfaces.window().registered(source, cx) {
            return Err(ContextSelectionError::StaleSurface);
        }
        Ok(())
    }
    pub fn add_context_background_from<T: Item>(
        &mut self,
        source: ProjectSurfaceReceipt,
        item: Entity<T>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<EntityId, ContextSelectionError> {
        if self
            .surfaces
            .window()
            .foreign_item(source, item.entity_id(), cx)
        {
            return Err(ContextSelectionError::StaleSurface);
        }
        if let Err(error) = self.validate_context_source(source, window, cx) {
            if self
                .surfaces
                .window()
                .resolve(source, cx)
                .ok()
                .is_none_or(|surface| !surface.read(cx).contains_item(item.entity_id()))
            {
                ItemHandle::close(&item, window, cx);
            }
            return Err(error);
        }
        let id = item.entity_id();
        let surface = self.surfaces.window().resolve(source, cx)?;
        surface.update(cx, |surface, cx| {
            surface
                .admit_background(source, item, window, cx)
                .map(|_| id)
        })
    }
    pub fn set_chat_connection_opener(&mut self, opener: Rc<dyn crate::ChatConnectionOpen>) {
        self.ports.set_chat_connections(opener);
    }
    pub fn chat_connection_opener(&self) -> Option<Rc<dyn crate::ChatConnectionOpen>> {
        self.ports.chat_connections()
    }
    pub fn show_context_background(
        &mut self,
        source: ProjectSurfaceReceipt,
        item: EntityId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<bool, crate::ContextSelectionError> {
        self.surfaces.window().check_window(window)?;
        let surface = self.surfaces.window().resolve(source, cx)?;
        let focus = self.context_source(cx) == source;
        Ok(surface.update(cx, |surface, cx| {
            surface.promote_background(item, focus, window, cx)
        }))
    }
    pub fn close_context_background(
        &mut self,
        source: ProjectSurfaceReceipt,
        item: EntityId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), crate::ContextSelectionError> {
        self.surfaces.window().check_window(window)?;
        let surface = self.surfaces.window().resolve(source, cx)?;
        surface.update(cx, |surface, cx| {
            surface.close_background_at(source, item, window, cx)
        });
        Ok(())
    }
    pub fn add_project_workspace<T: Item>(
        &mut self,
        item: Entity<T>,
        project: ProjectSurface,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<bool, crate::ContextSelectionError> {
        let project = project.bind(window, cx);
        let selection = self.surfaces.outer().read(cx).selection();
        let admission = SurfaceItemAdmission::workspace(selection, project);
        self.surfaces.outer().update(cx, |surface, cx| {
            surface.admit_item(admission, item, None, window, cx)
        })
    }
    pub fn set_session_factory(&mut self, factory: impl SessionFactory) {
        self.ports.install(factory);
    }
    pub fn set_session_opener(
        &mut self,
        opener: impl Fn(&mut Workspace, SessionSpec, &mut Window, &mut Context<Workspace>) + 'static,
    ) {
        self.ports.override_session(opener);
    }
    pub fn set_program_opener(
        &mut self,
        opener: impl Fn(&mut Workspace, ProgramSpec, &mut Window, &mut Context<Workspace>) + 'static,
    ) {
        self.ports.override_program(opener);
    }
    pub fn set_local_terminal_opener(
        &mut self,
        opener: impl Fn(&mut Workspace, LocalTerminalTarget, &mut Window, &mut Context<Workspace>)
        + 'static,
    ) {
        self.ports.override_local(opener);
    }
    pub fn add_general_item_from<T: Item>(
        &mut self,
        target: ContextSelectionReceipt,
        item: Entity<T>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<bool, ContextSelectionError> {
        let admission = SurfaceItemAdmission::general(target);
        self.surfaces.outer().update(cx, |surface, cx| {
            surface.admit_item(admission, item, None, window, cx)
        })
    }
    pub fn add_general_local_terminal_from<T: crate::LocalTerminal>(
        &mut self,
        target: ContextSelectionReceipt,
        item: Entity<T>,
        destination: LocalTerminalTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<bool, ContextSelectionError> {
        let admission = SurfaceItemAdmission::general_local(target, destination);
        let local =
            Some(Box::new(item.clone()) as Box<dyn crate::local_terminal::LocalTerminalHandle>);
        self.surfaces.outer().update(cx, |surface, cx| {
            surface.admit_item(admission, item, local, window, cx)
        })
    }
    pub fn open_session(&mut self, spec: SessionSpec, window: &mut Window, cx: &mut Context<Self>) {
        self.show_new_tab_menu(false, cx);
        self.factory().open_session(self, spec, window, cx);
    }
    pub fn open_program(&mut self, spec: ProgramSpec, window: &mut Window, cx: &mut Context<Self>) {
        let parent = self
            .surfaces
            .outer()
            .read(cx)
            .tabs()
            .program_parent(&spec.host.key);
        let opened_from = self.surfaces.outer().read(cx).tabs().len();
        self.factory().open_program(self, spec, window, cx);
        self.group_program(parent, opened_from, window, cx);
    }
    pub fn open_background_session(
        &mut self,
        spec: SessionSpec,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<EntityId> {
        self.factory().open_background(self, spec, window, cx)
    }
    pub(crate) fn new_local_terminal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.factory()
            .open_local(self, LocalTerminalTarget::Bottom, window, cx);
    }
    pub fn set_connection_directory(&mut self, directory: Rc<dyn crate::ConnectionDirectory>) {
        self.ports.set_directory(directory);
    }
    pub fn open_on_host(
        &mut self,
        host: &HostAddress,
        program: String,
        args: Vec<String>,
        directory: Option<String>,
        title: SharedString,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), crate::OpenOnHostError> {
        // Published snapshots choose the tab, the active one first; only the
        // chosen tabs are asked how they were opened.
        if !self.general_context(cx) {
            return Err(crate::OpenOnHostError::ProtectedContext);
        }
        let spec = self
            .surfaces
            .outer()
            .read(cx)
            .tabs()
            .session_spec_for(host, cx)
            .ok_or_else(|| crate::OpenOnHostError::NoConnectedSession {
                target: host.target.clone(),
            })?;
        let env = spec
            .launch
            .as_ref()
            .map(|launch| launch.env.clone())
            .unwrap_or_default();
        let launch = ShellLaunch {
            program: Some(program),
            args,
            cwd: directory,
            env,
            integration: false,
        };
        launch.validate()?;
        let spec = super::SessionSpec {
            title,
            launch: Some(launch),
            ..spec
        };
        self.open_session(spec, window, cx);
        Ok(())
    }
    /// Registers the panel of a contribution of the plugin catalog and creates it now, unless
    /// the user disabled its plugin: then the panel is created when the plugin is enabled.
    ///
    /// The panel joins the group and the position the window's layout gives it, and starts
    /// available.
    pub fn register_panel<T: Panel>(
        &mut self,
        registration: PanelRegistration<T>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), RegistrationError> {
        self.layout_settings_changed(window, cx);
        let set = self.shell.set().clone();
        let traits = self.registrations.admit_panel(
            &set,
            registration.id(),
            registration.expandable_requested(),
            registration.is_eager(),
        )?;
        let id = registration.id().clone();
        let opened = id.clone();
        match registration.into_source(traits.chrome, window, cx) {
            PanelSource::Hosted(panel) => {
                self.panels.register(id, Some(panel), self.shell.current());
            }
            PanelSource::Factory(factory) => {
                // The panel's slot exists while the factory reads the workspace.
                self.panels.register(id.clone(), None, self.shell.current());
                if self.shell.is_enabled(&id) {
                    let panel = factory(&*self, window, cx);
                    self.panels.fill(&id, panel);
                }
                self.registrations.keep_factory(id, factory);
            }
        }
        if self.shell.claim_open(&opened)
            && let Some(transition) = self.panels.reveal_panel(&opened)
            && transition.visibility_changed().is_some()
        {
            cx.emit(WorkspaceEvent::PanelVisibilityChanged);
        }
        self.sync_panel_sides(cx);
        cx.notify();
        Ok(())
    }
    /// Registers the status bar widget of a contribution of the plugin catalog. Its view is
    /// created now if the window's arrangement places the widget, and when it first does
    /// otherwise.
    pub fn register_widget(
        &mut self,
        registration: WidgetRegistration,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), RegistrationError> {
        self.layout_settings_changed(window, cx);
        let set = self.shell.set().clone();
        self.registrations.admit_widget(&set, registration.id())?;
        let id = registration.id().clone();
        let (factory, zone_hook) = registration.into_parts();
        self.chrome.status_bar_mut().add(id, factory, zone_hook);
        self.sync_status_widgets(window, cx);
        cx.notify();
        Ok(())
    }
    /// Creates the views of the widgets the arrangement places and drops those it does not.
    pub(super) fn sync_status_widgets(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let pending = self.chrome.status_bar().pending(self.shell.current());
        for (id, factory) in pending {
            let view = factory(&*self, window, cx);
            self.chrome.status_bar_mut().fill(&id, view);
        }
        self.chrome
            .status_bar_mut()
            .release_unplaced(self.shell.current());
        self.chrome
            .status_bar_mut()
            .follow_zones(self.shell.current(), cx);
    }
    /// Tells the expandable panels which side of the content their groups are on.
    pub(super) fn sync_panel_sides(&self, cx: &mut Context<Self>) {
        self.panels.set_docked_left(cx);
    }
    pub fn set_menu_builder(
        &mut self,
        builder: impl Fn(&Workspace, &Window, &App) -> Vec<Menu> + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.chrome.install_menu_builder(Rc::new(builder));
        self.reload_menu_bar(window, cx);
    }
    fn reload_menu_bar(&mut self, window: &Window, cx: &mut Context<Self>) {
        let Some(builder) = self.chrome.menu_builder() else {
            return;
        };
        let target = self.command_item(window, cx).map(|item| item.item_id());
        let menus = builder(self, window, cx)
            .into_iter()
            .map(Menu::owned)
            .collect();
        self.chrome
            .refresh_menus(menus, target, self.command_selection(window, cx), cx);
    }
    pub(super) fn prepare_menu(&mut self, window: &Window, cx: &mut Context<Self>) {
        if self.chrome.prepare_closed_menu(cx) {
            self.reload_menu_bar(window, cx);
        }
    }
    pub fn add_background_item<T: crate::Item>(
        &mut self,
        item: gpui_kit::Entity<T>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> EntityId {
        let id = item.entity_id();
        let selection = self.surfaces.outer().read(cx).selection();
        self.add_general_background_from(selection, item, window, cx)
            .unwrap_or(id)
    }
    pub fn show_background_session(
        &mut self,
        id: EntityId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        self.surfaces.outer().update(cx, |surface, cx| {
            surface.promote_background(id, true, window, cx)
        })
    }
    pub fn close_background_session(
        &mut self,
        id: EntityId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.surfaces.outer().update(cx, |surface, cx| {
            surface.close_background_at(surface.source(), id, window, cx)
        })
    }
    pub fn set_local_terminal<T: crate::LocalTerminal>(
        &mut self,
        item: Entity<T>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let source = self.general_surface_source(cx);
        if self.general_item(item.entity_id(), cx).is_some()
            || self
                .surfaces
                .window()
                .foreign_item(source, item.entity_id(), cx)
        {
            return;
        }
        if self.validate_context_source(source, window, cx).is_err() {
            ItemHandle::close(&item, window, cx);
            return;
        }
        let ids = self.surfaces.outer().read(cx).tabs().local_ids();
        for id in ids {
            self.close_tabs(id, TabCloseScope::Current, window, cx);
        }
        self.add_local_terminal(item, window, cx);
    }
    pub fn add_local_terminal<T: crate::LocalTerminal>(
        &mut self,
        item: Entity<T>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.add_local_terminal_at(item, LocalTerminalTarget::Bottom, window, cx);
    }
    pub fn add_local_terminal_at<T: crate::LocalTerminal>(
        &mut self,
        item: Entity<T>,
        target: LocalTerminalTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let selection = self.surfaces.outer().read(cx).selection();
        self.add_general_local_terminal_from(selection, item, target, window, cx)
            .unwrap_or(false)
    }
    pub fn add_item<T: Item>(
        &mut self,
        item: Entity<T>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let selection = self.surfaces.outer().read(cx).selection();
        let _ = self.add_general_item_from(selection, item, window, cx);
    }
    // ── Following the layout settings ────────────────────────────────────────

    /// Creates the panels of the plugins that are enabled and have none, and releases those of
    /// the plugins that are disabled. A disabled panel that shows closes first, and its entity
    /// drops once the change is applied; a panel that is enabled again starts closed.
    fn apply_enablement(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let ids: Vec<_> = self.panels.registered_ids().cloned().collect();
        for id in ids {
            if self.shell.is_enabled(&id) {
                self.create_panel(&id, window, cx);
                continue;
            }
            let Some((transition, panel)) = self.panels.release(&id, window, cx) else {
                continue;
            };
            if let Some(transition) = transition {
                self.apply_panel_transition(transition, window, cx);
            }
            cx.defer(move |_| drop(panel));
        }
    }

    /// Runs the factory of a registered panel that has none.
    fn create_panel(
        &mut self,
        id: &nocterm_plugin_model::ContributionId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.panels.lacks_panel(id) {
            return;
        }
        let Some(factory) = self.registrations.factory(id) else {
            return;
        };
        let panel = factory(&*self, window, cx);
        self.panels.fill(id, panel);
    }

    /// Applies the layout the settings select now: the groups take the new membership, a switch
    /// to a saved layout shows the panels it names, and panels the layout no longer places
    /// close.
    pub(super) fn layout_settings_changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let change = self.shell.refresh(cx);
        if change == LayoutChange::Unchanged {
            return;
        }
        let open = match &change {
            LayoutChange::Opened(open) => Some(open.as_slice()),
            _ => None,
        };
        self.apply_enablement(window, cx);
        for transition in self.panels.follow(self.shell.current(), open, window, cx) {
            self.apply_panel_transition(transition, window, cx);
        }
        if let Some(open) = open
            && open.contains(&LOCAL_TERMINAL.id) != self.local_terminal_is_visible(cx)
        {
            self.toggle_local_terminal(window, cx);
        }
        self.sync_panel_sides(cx);
        self.sync_status_widgets(window, cx);
        cx.notify();
    }
}
