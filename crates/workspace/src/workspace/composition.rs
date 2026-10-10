//! Installation and explicit erased handoffs; state transitions belong to owners.
use super::*;
use crate::{LocalTerminalTarget, Panel, ProgramSpec};
use chrome::StatusSide;
use nocterm_session::ShellLaunch;
impl Workspace {
    /// Installs what turns requests into tabs.
    pub fn set_session_factory(&mut self, factory: impl SessionFactory) {
        self.ports.install(factory);
    }

    /// Installs only what turns a [`SessionSpec`] into a tab, keeping the
    /// rest of the current factory.
    pub fn set_session_opener(
        &mut self,
        opener: impl Fn(&mut Workspace, SessionSpec, &mut Window, &mut Context<Workspace>) + 'static,
    ) {
        self.ports.override_session(opener);
    }

    /// Installs only what turns a [`ProgramSpec`] into a tab.
    pub fn set_program_opener(
        &mut self,
        opener: impl Fn(&mut Workspace, ProgramSpec, &mut Window, &mut Context<Workspace>) + 'static,
    ) {
        self.ports.override_program(opener);
    }

    /// Installs only what opens a local shell.
    pub fn set_local_terminal_opener(
        &mut self,
        opener: impl Fn(&mut Workspace, LocalTerminalTarget, &mut Window, &mut Context<Workspace>)
        + 'static,
    ) {
        self.ports.override_local(opener);
    }

    /// Opens a session in a new tab.
    pub fn open_session(&mut self, spec: SessionSpec, window: &mut Window, cx: &mut Context<Self>) {
        self.show_new_tab_menu(false, cx);
        self.factory().open_session(self, spec, window, cx);
    }

    /// Opens a program in a new tab, grouped with the active tab when that
    /// is a session on the same host, as a container's log is with the
    /// session it was opened from.
    pub fn open_program(&mut self, spec: ProgramSpec, window: &mut Window, cx: &mut Context<Self>) {
        let parent = self.tabs.program_parent(&spec.host.key);
        let opened_from = self.tabs.len();
        self.factory().open_program(self, spec, window, cx);
        self.group_program(parent, opened_from, window, cx);
    }
}

impl Workspace {
    pub fn open_background_session(
        &mut self,
        spec: SessionSpec,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<EntityId> {
        self.factory().open_background(self, spec, window, cx)
    }

    pub fn add_background_item<T: crate::Item>(
        &mut self,
        item: gpui_kit::Entity<T>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> EntityId {
        let id = item.entity_id();
        if self.background.contains(id) || self.tabs.contains(id) {
            return id;
        }
        let subscription = cx.subscribe_in(&item, window, move |this, _, event, window, cx| {
            if !this.background.contains(id) {
                return;
            }
            match event {
                crate::ItemEvent::Changed => cx.emit(super::WorkspaceEvent::ItemsChanged),
                crate::ItemEvent::CloseRequested => this.close_background_session(id, window, cx),
            }
        });
        if self.background.insert(item, subscription) {
            cx.emit(super::WorkspaceEvent::ItemsChanged);
            cx.notify();
        }
        id
    }

    pub fn show_background_session(
        &mut self,
        id: EntityId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(record) = self.background.take(id) else {
            return false;
        };
        let target = self.tabs.active_location(cx).filter(|(placement, _)| {
            *placement == gpui_kit::component::dock::DockPlacement::Center
        });
        let bindings = super::bindings::WorkspaceBindings::new(cx.weak_entity());
        let prepared = record.into_tab(&bindings, self.tabs.header_origin(), window, cx);
        self.register_prepared(
            prepared,
            gpui_kit::component::dock::DockPlacement::Center,
            target.map(|(_, node)| node),
            window,
            cx,
        );
        true
    }

    pub fn close_background_session(
        &mut self,
        id: EntityId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(record) = self.background.take(id) {
            record.close(window, cx);
            cx.emit(super::WorkspaceEvent::ItemsChanged);
            cx.notify();
        }
    }
}
impl Workspace {
    pub(crate) fn new_local_terminal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.factory()
            .open_local(self, LocalTerminalTarget::Bottom, window, cx);
    }
    pub(crate) fn new_local_terminal_from(
        &mut self,
        id: EntityId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.tab_location(id, cx).is_none() {
            return;
        }
        self.factory()
            .open_local(self, LocalTerminalTarget::Beside(id), window, cx);
    }
    pub fn set_local_terminal<T: crate::LocalTerminal>(
        &mut self,
        item: Entity<T>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let ids: Vec<_> = self.tabs.local_ids();
        for id in ids {
            self.close_item_by_id(id, window, cx);
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
    /// Registers an owned new shell in the requested live pane.
    /// A stale anchor closes the new shell once and returns false. An already
    /// registered Item returns false without changing or closing that Item.
    pub fn add_local_terminal_at<T: crate::LocalTerminal>(
        &mut self,
        item: Entity<T>,
        target: LocalTerminalTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.is_background(item.entity_id()) || self.tabs.contains(item.entity_id()) {
            return false;
        }
        let destination = match target {
            LocalTerminalTarget::Bottom => self.tabs.local_destination(cx),
            LocalTerminalTarget::Beside(anchor) => {
                let Some(destination) = self.tab_location(anchor, cx) else {
                    ItemHandle::close(&item, window, cx);
                    return false;
                };
                Some(destination)
            }
        };
        let placement = destination.map_or(DockPlacement::Bottom, |(placement, _)| placement);
        self.tabs.ensure_region_open(placement, window, cx);
        let local: Box<dyn crate::local_terminal::LocalTerminalHandle> = Box::new(item.clone());
        self.register_item(
            item,
            Some(local),
            placement,
            destination.map(|(_, node)| node),
            window,
            cx,
        );
        true
    }
}
impl Workspace {
    pub fn add_item<T: Item>(
        &mut self,
        item: Entity<T>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let target = self
            .tabs
            .active_location(cx)
            .filter(|(placement, _)| *placement == DockPlacement::Center);
        self.register_item(
            item,
            None,
            DockPlacement::Center,
            target.map(|(_, node)| node),
            window,
            cx,
        );
    }
    fn register_item<T: Item>(
        &mut self,
        item: Entity<T>,
        local: Option<Box<dyn crate::local_terminal::LocalTerminalHandle>>,
        placement: DockPlacement,
        target: Option<gpui_kit::component::dock::NodeId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.is_background(item.entity_id()) || self.tabs.contains(item.entity_id()) {
            return;
        }
        let bindings = bindings::WorkspaceBindings::new(cx.weak_entity());
        let prepared = bindings.prepare(item, local, self.tabs.header_origin(), window, cx);
        self.register_prepared(prepared, placement, target, window, cx);
    }
    fn register_prepared(
        &mut self,
        prepared: bindings::PreparedTab,
        placement: DockPlacement,
        target: Option<gpui_kit::component::dock::NodeId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(id) = self.tabs.register(prepared, placement, target, window, cx) {
            self.activate_item_by_id(id, window, cx);
            cx.emit(WorkspaceEvent::ItemsChanged);
        }
    }
    pub fn set_connection_directory(&mut self, directory: Rc<dyn crate::ConnectionDirectory>) {
        self.ports.set_directory(directory);
    }
    /// Opens a new tab running `program` with `args` in `directory` on the
    /// host of a connected session to `target`. It signs in as that session
    /// did; a saved profile's credentials are reused, otherwise the user is
    /// asked again.
    pub fn open_on_host(
        &mut self,
        target: &Target,
        program: String,
        args: Vec<String>,
        directory: Option<String>,
        title: SharedString,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        // Published snapshots choose the tab, the active one first; only the
        // chosen tabs are asked how they were opened.
        let spec = self
            .tabs
            .session_spec_for(target, cx)
            .ok_or_else(|| format!("No connected session to {target} is open."))?;
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
}
impl Workspace {
    /// Adds a sidebar panel, after the ones already there.
    pub fn add_panel<T: Panel>(&mut self, panel: Entity<T>, cx: &mut Context<Self>) {
        // A panel's title, icon and badge sit in the workspace's chrome; the
        // rest of the panel redraws itself, so only a change to them counts.
        let chrome = |panel: &Entity<T>, cx: &gpui_kit::App| {
            let panel = panel.read(cx);
            (panel.title(cx), panel.icon(cx), panel.badge(cx))
        };
        let metadata = chrome(&panel, cx);
        let subscription = cx.observe(&panel, move |this, panel, cx| {
            if this
                .sidebar
                .metadata_changed(panel.entity_id(), chrome(&panel, cx))
            {
                cx.notify();
            }
        });
        self.sidebar
            .register(Box::new(panel), metadata, subscription);
        cx.notify();
    }
}
impl Workspace {
    pub fn set_right_panel<T: crate::RightPanel>(
        &mut self,
        panel: Entity<T>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let subscription =
            cx.subscribe_in(&panel, window, |this, _, event, window, cx| match event {
                crate::RightPanelEvent::ToggleMaximized => {
                    this.set_right_panel_maximized(!this.right_panel.maximized(), cx)
                }
                crate::RightPanelEvent::Close => this.close_right_panel(window, cx),
                crate::RightPanelEvent::Widen(delta) => this.widen_right_panel(*delta, window, cx),
            });
        let left = self.sides_swapped(cx);
        self.right_panel
            .install(Box::new(panel), subscription, left, cx);
        cx.notify();
    }
}
impl Workspace {
    /// A feature-owned compact status view, visible even while the sidebar is hidden.
    pub fn add_status_view<V: Render>(&mut self, view: Entity<V>, cx: &mut Context<Self>) {
        self.chrome.add_status(StatusSide::Trailing, view.into());
        cx.notify();
    }

    /// A feature-owned status view at the start of the footer, after the
    /// sidebar's panel buttons: room for what describes the active host.
    pub fn add_leading_status_view<V: Render>(&mut self, view: Entity<V>, cx: &mut Context<Self>) {
        self.chrome.add_status(StatusSide::Leading, view.into());
        cx.notify();
    }
}
impl Workspace {
    /// Supplies window-specific menus to the toolkit's standard menu bar.
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
        self.chrome.refresh_menus(menus, target, cx);
    }

    pub(super) fn prepare_menu(&mut self, window: &Window, cx: &mut Context<Self>) {
        if self.chrome.prepare_closed_menu(cx) {
            self.reload_menu_bar(window, cx);
        }
    }
}
