//! Explicit installations and typed handoffs to the original tab owner.
use super::*;
use crate::ContextSelectionError;
use crate::{LocalTerminalTarget, Panel, ProgramSpec};
use chrome::StatusSide;
use nocterm_session::{HostAddress, ShellLaunch};
use panel_groups::{HostedPanel, Presentation};
use surface::admission::SurfaceItemAdmission;
/// A panel's title, icon and badge sit in the workspace's chrome; the rest of the panel
/// redraws itself, so only a change to them counts.
fn presentation_of<T: Panel>(panel: &Entity<T>, cx: &App) -> Presentation {
    let panel = panel.read(cx);
    (panel.title(cx), panel.icon(cx), panel.badge(cx))
}

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
    pub fn add_panel<T: Panel>(&mut self, panel: Entity<T>, cx: &mut Context<Self>) {
        let metadata = presentation_of(&panel, cx);
        let subscription = cx.observe(&panel, move |this, panel, cx| {
            if this
                .panels
                .presentation_changed(panel.entity_id(), presentation_of(&panel, cx))
            {
                cx.notify();
            }
        });
        self.panels.add(
            GroupKey::Navigation,
            HostedPanel::plain(Box::new(panel), metadata, subscription),
        );
        cx.notify();
    }
    pub fn set_right_panel<T: crate::RightPanel>(
        &mut self,
        panel: Entity<T>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let subscription =
            cx.subscribe_in(&panel, window, |this, _, event, window, cx| match event {
                crate::RightPanelEvent::ToggleMaximized => this
                    .set_right_panel_maximized(!this.panels.is_expanded(GroupKey::Assistant), cx),
                crate::RightPanelEvent::Close => this.close_right_panel(window, cx),
                crate::RightPanelEvent::Widen(delta) => this.widen_right_panel(*delta, window, cx),
            });
        let presentation = presentation_of(&panel, cx);
        let hosted = HostedPanel::expandable(Box::new(panel), presentation, subscription);
        self.panels.install(GroupKey::Assistant, hosted);
        self.panels.set_docked_left(self.sides_swapped(cx), cx);
        cx.notify();
    }
    pub fn add_status_view<V: Render>(&mut self, view: Entity<V>, cx: &mut Context<Self>) {
        self.chrome.add_status(StatusSide::Trailing, view.into());
        cx.notify();
    }
    pub fn add_leading_status_view<V: Render>(&mut self, view: Entity<V>, cx: &mut Context<Self>) {
        self.chrome.add_status(StatusSide::Leading, view.into());
        cx.notify();
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
}
