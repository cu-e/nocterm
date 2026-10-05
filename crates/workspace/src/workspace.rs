use std::{path::PathBuf, rc::Rc};

use gpui_kit::{
    Action, AnyView, App, ClipboardItem, Context, Div, Entity, EntityId, EventEmitter, FocusHandle,
    Focusable, Menu, Pixels, SharedString, Subscription, Window,
    base::GlobalState,
    component::{
        ActiveTheme as _,
        dock::{
            DockArea, DockEvent, DockPlacement, DockSkin, InsertTarget, PaneRef, PanelId,
            PanelStyle, panel_handle,
        },
        menu::AppMenuBar,
        v_flex,
    },
    div,
    prelude::*,
    rems,
};
use nocterm_session::{Auth, Target};
use nocterm_ui::ActiveDesign as _;

mod background;
mod chrome;
mod groups;
mod layout;
mod openers;
mod panels;
mod panes;
mod right_panel;
mod sessions;

use crate::{
    CloseTab, Item, ItemCommand, ItemEvent, ItemHandle, KEY_CONTEXT, NewTab, NextPanel, NextTab,
    PanelHandle, PreviousTab, SessionContext, ToggleSidebar,
};

/// A request to open a session in a new tab.
///
/// It names where to connect and how to sign in; the details of the
/// connection (terminal type, timeouts) come from the settings at the moment
/// it is made, so a reconnect picks up changed settings.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionSpec {
    /// Per-session overrides. Empty fields inherit user defaults.
    pub options: nocterm_session::SessionOptions,
    /// The tab's title.
    pub title: SharedString,
    /// Saved profile identity, independent of display aliases.
    pub profile: Option<SharedString>,
    pub target: Target,
    pub auth: Auth,
    pub launch: Option<nocterm_session::ShellLaunch>,
    pub credential: Option<nocterm_session::CredentialId>,
}

/// What the workspace tells its observers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkspaceEvent {
    /// Terminal membership or metadata changed.
    ItemsChanged,
    /// Another tab became active, or the last one closed.
    ActiveItemChanged,
    /// The session behind the active tab is a different one, or changed
    /// state.
    ActiveSessionChanged,
    /// Local shell reported a current-directory change.
    LocalDirectoryChanged,
    /// Saved server descriptors or discovered facts changed.
    ConnectionsChanged,
    /// The independent side panel was opened or closed.
    RightPanelVisibilityChanged,
}

/// Close commands use the clicked tab's pane and its current visual order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TabCloseScope {
    Current,
    Others,
    Left,
    Right,
    /// All central tabs; the independent local terminal is left running.
    All,
}

type SessionOpener = Rc<dyn Fn(&mut Workspace, SessionSpec, &mut Window, &mut Context<Workspace>)>;
type ActionRegistration = Box<dyn Fn(Div, &mut Context<Workspace>) -> Div>;
type MenuBuilder = Rc<dyn Fn(&Workspace, &Window, &App) -> Vec<Menu>>;

struct NewTabMenu {
    view: AnyView,
    focus: FocusHandle,
}

struct OpenItem {
    handle: Rc<dyn ItemHandle>,
    /// Updated on ItemEvent::Changed; safe to query while any Item renders.
    session: Option<SessionContext>,
    dock_item: Entity<crate::dock_item::DockItem>,
    _subscription: Subscription,
    _focus_subscription: Subscription,
}

/// The main window's content.
type LocalOpener = Rc<dyn Fn(&mut Workspace, &mut Window, &mut Context<Workspace>)>;

struct BottomTerminal {
    item: Entity<crate::dock_item::DockItem>,
    handle: Rc<dyn ItemHandle>,
    local: Box<dyn crate::local_terminal::LocalTerminalHandle>,
    attached: bool,
    height: Pixels,
    _subscription: Subscription,
    _focus_subscription: Subscription,
}

pub struct Workspace {
    focus_handle: FocusHandle,
    dock: Entity<DockArea>,
    _dock_subscription: Subscription,
    _dock_observer: Subscription,
    dock_hide_queued: bool,
    local_terminal: Option<BottomTerminal>,
    local_opener: Option<LocalOpener>,
    items: Vec<OpenItem>,
    tab_groups: crate::tab_groups::TabGroups<PanelId, gpui_kit::component::dock::NodeId>,
    active_item: Option<usize>,
    panels: Vec<Box<dyn PanelHandle>>,
    active_panel: usize,
    sidebar_open: bool,
    body: layout::Body,
    right_panel: Option<Box<dyn crate::right_panel::RightPanelHandle>>,
    right_panel_open: bool,
    right_panel_attention: bool,
    right_panel_available: bool,
    right_panel_maximized: bool,
    right_panel_subscription: Option<Subscription>,
    new_tab_menu: Option<NewTabMenu>,
    new_tab_menu_open: bool,
    session_opener: Option<SessionOpener>,
    program_opener: Option<openers::ProgramOpener>,
    actions: Vec<ActionRegistration>,
    status_views: Vec<(chrome::StatusSide, AnyView)>,
    /// The active session as last announced, to announce only changes.
    announced_session: Option<SessionContext>,
    menu_builder: Option<MenuBuilder>,
    app_menu_bar: Option<Entity<AppMenuBar>>,
    menu_focus: FocusHandle,
    menu_item: Option<EntityId>,
    last_command_item: Option<EntityId>,
    connection_directory: Option<Rc<dyn crate::ConnectionDirectory>>,
    /// Sessions running without a tab, opened for agents.
    background: Vec<background::BackgroundItem>,
    background_opener: Option<background::BackgroundOpener>,
}

impl EventEmitter<WorkspaceEvent> for Workspace {}

impl Workspace {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let (dock, skin) = DockSkin::dock_area("workspace", None, window, cx);
        skin.set_panel_style(PanelStyle::TabBar, cx);
        // Session tabs may close even when they are the dock's last panel.
        // Adapter-owned buttons provide that lifecycle policy using native controls.
        skin.set_close_button_visible(false, cx);
        skin.set_toggle_button_visible(false, cx);
        let subscription = cx.subscribe_in(&dock, window, |this, _, _: &DockEvent, window, cx| {
            this.schedule_hidden_dock_detach(window, cx);
            this.regroup(window, cx);
            let focused = this
                .items
                .iter()
                .find(|open| open.dock_item.read(cx).contains_focus(window, cx))
                .map(|open| open.handle.item_id());
            if let Some(id) = focused {
                this.mark_active(id, cx);
            }
            cx.notify();
        });
        // Dock resizing changes its open state with `notify`, without emitting
        // DockEvent. Observe both paths so a closed bottom strip disappears.
        let observer = cx.observe_in(&dock, window, |this, _, window, cx| {
            this.schedule_hidden_dock_detach(window, cx);
        });
        let mut this = Self {
            dock,
            _dock_subscription: subscription,
            _dock_observer: observer,
            dock_hide_queued: false,
            local_terminal: None,
            local_opener: None,
            focus_handle: cx.focus_handle(),
            items: Vec::new(),
            tab_groups: Default::default(),
            active_item: None,
            panels: Vec::new(),
            active_panel: 0,
            sidebar_open: true,
            body: layout::Body::default(),
            right_panel: None,
            right_panel_open: false,
            right_panel_attention: false,
            right_panel_available: false,
            right_panel_maximized: false,
            right_panel_subscription: None,
            new_tab_menu: None,
            new_tab_menu_open: false,
            session_opener: None,
            program_opener: None,
            actions: Vec::new(),
            status_views: Vec::new(),
            announced_session: None,
            menu_builder: None,
            app_menu_bar: None,
            menu_focus: cx.focus_handle(),
            menu_item: None,
            last_command_item: None,
            connection_directory: None,
            background: Vec::new(),
            background_opener: None,
        };
        macro_rules! item_action {
            ($action:ty, $command:ident) => {
                this.register_action::<$action>(|this, _, window, cx| {
                    this.dispatch_item_command(ItemCommand::$command, window, cx);
                });
            };
        }
        this.register_action::<crate::EditCopy>(|_, _, window, cx| {
            window.dispatch_action(Box::new(gpui_kit::component::input::Copy), cx)
        });
        this.register_action::<crate::EditPaste>(|_, _, window, cx| {
            window.dispatch_action(Box::new(gpui_kit::component::input::Paste), cx)
        });
        this.register_action::<crate::SelectAll>(|_, _, window, cx| {
            window.dispatch_action(Box::new(gpui_kit::component::input::SelectAll), cx)
        });
        item_action!(crate::ClearSelection, ClearSelection);
        item_action!(crate::Find, Find);
        item_action!(crate::FindNext, FindNext);
        item_action!(crate::FindPrevious, FindPrevious);
        item_action!(crate::FindNextSelection, FindNextSelection);
        item_action!(crate::DisconnectSession, Disconnect);
        item_action!(crate::ReconnectSession, Reconnect);
        item_action!(crate::StartRecording, StartRecording);
        item_action!(crate::StopRecording, StopRecording);
        item_action!(crate::SessionSettings, SessionSettings);
        item_action!(gpui_kit::component::input::Copy, Copy);
        item_action!(gpui_kit::component::input::Paste, Paste);
        item_action!(gpui_kit::component::input::SelectAll, SelectAll);
        this.register_action::<crate::CopyConnectionName>(|this, _, window, cx| {
            if let Some(title) = this.command_title(window, cx) {
                cx.write_to_clipboard(ClipboardItem::new_string(title.to_string()));
            }
        });
        this
    }

    // ── Items ────────────────────────────────────────────────────────────────

    /// Opens `item` in a new tab and switches to it.
    pub fn add_item<T: Item>(
        &mut self,
        item: Entity<T>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let id = item.entity_id();
        let subscription =
            cx.subscribe_in(
                &item,
                window,
                move |this, item, event, window, cx| match event {
                    ItemEvent::Changed => {
                        if let Some(open) = this
                            .items
                            .iter_mut()
                            .find(|open| open.handle.item_id() == id)
                        {
                            open.session = item.read(cx).session(cx);
                            open.dock_item.update(cx, |_, cx| cx.notify());
                        }
                        this.announce_session(cx);
                        cx.emit(WorkspaceEvent::ItemsChanged);
                        cx.notify();
                    }
                    ItemEvent::CloseRequested => {
                        if let Some(ix) = this
                            .items
                            .iter()
                            .position(|open| open.handle.item_id() == id)
                        {
                            this.close_item(ix, window, cx);
                        }
                    }
                },
            );

        let destination = self
            .active_item
            .and_then(|ix| self.items.get(ix))
            .and_then(|open| {
                self.dock
                    .read(cx)
                    .layout(DockPlacement::Center)?
                    .find_panel_node(PanelId::from(open.dock_item.entity_id()))
            });
        let session = item.read(cx).session(cx);
        let handle: Rc<dyn ItemHandle> = Rc::new(item);
        let workspace = cx.weak_entity();
        let dock_item = cx.new(|item_cx| {
            crate::dock_item::DockItem::new(handle.clone(), workspace, false, window, item_cx)
        });
        let focus = dock_item.read(cx).container_focus_handle();
        let focus_subscription = cx.on_focus_in(&focus, window, move |this, _, cx| {
            this.last_command_item = Some(id);
            this.mark_active(id, cx);
        });
        self.dock.update(cx, |dock, cx| {
            dock.add_panel_view(
                panel_handle(dock_item.clone()),
                DockPlacement::Center,
                None,
                window,
                cx,
            )
        });
        if let Some(node) = destination {
            self.dock.update(cx, |dock, cx| {
                dock.move_panel(
                    PanelId::from(dock_item.entity_id()),
                    InsertTarget::Tabs {
                        node,
                        ix: None,
                        activate: true,
                    },
                    window,
                    cx,
                )
            });
        }
        self.items.push(OpenItem {
            handle,
            session,
            dock_item,
            _subscription: subscription,
            _focus_subscription: focus_subscription,
        });
        self.activate_item(self.items.len() - 1, window, cx);
        cx.emit(WorkspaceEvent::ItemsChanged);
    }

    /// Open items in stable registration order. The native dock owns their display order.
    pub fn items(&self) -> impl Iterator<Item = &dyn ItemHandle> {
        self.items.iter().map(|open| open.handle.as_ref())
    }

    pub fn set_connection_directory(&mut self, directory: Rc<dyn crate::ConnectionDirectory>) {
        self.connection_directory = Some(directory);
    }
    pub fn connection_directory(&self) -> Option<Rc<dyn crate::ConnectionDirectory>> {
        self.connection_directory.clone()
    }

    pub fn active_terminal(&self, cx: &App) -> Option<EntityId> {
        let terminals = self.terminals(cx);
        self.last_command_item
            .filter(|id| terminals.iter().any(|entry| entry.item == *id))
            .or_else(|| {
                self.active_item()
                    .filter(|item| item.terminal_access(cx).is_some())
                    .map(|item| item.item_id())
            })
    }

    pub fn terminals(&self, cx: &App) -> Vec<crate::TerminalEntry> {
        let mut entries: Vec<_> = self
            .items
            .iter()
            .filter_map(|open| {
                Some(crate::TerminalEntry {
                    item: open.handle.item_id(),
                    access: open.handle.terminal_access(cx)?,
                    title: open
                        .dock_item
                        .read(cx)
                        .alias
                        .clone()
                        .unwrap_or_else(|| open.handle.tab_title(cx)),
                    active: false,
                    bottom: false,
                    background: false,
                })
            })
            .collect();
        if let Some(local) = &self.local_terminal
            && let Some(access) = local.handle.terminal_access(cx)
        {
            entries.push(crate::TerminalEntry {
                item: local.handle.item_id(),
                access,
                title: local.handle.tab_title(cx),
                active: false,
                bottom: true,
                background: false,
            });
        }
        entries.extend(self.background_entries(cx));
        let active = self
            .last_command_item
            .filter(|id| entries.iter().any(|entry| entry.item == *id))
            .or_else(|| self.active_item().map(|item| item.item_id()));
        for entry in &mut entries {
            entry.active = active == Some(entry.item);
        }
        entries
    }

    pub fn active_item(&self) -> Option<&dyn ItemHandle> {
        self.active_item
            .and_then(|ix| self.items.get(ix))
            .map(|open| open.handle.as_ref())
    }

    /// The first open tab showing a `T`.
    pub fn find_item<T: Item>(&self) -> Option<Entity<T>> {
        self.items
            .iter()
            .find_map(|open| open.handle.view().downcast::<T>().ok())
    }

    pub fn activate_item(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.set_right_panel_maximized(false, cx);
        let Some(open) = self.items.get(ix) else {
            return;
        };
        let panel = PanelId::from(open.dock_item.entity_id());
        self.dock
            .update(cx, |dock, cx| dock.select_panel(panel, window, cx));
        let focus = open.handle.focus_handle(cx);
        let changed = self.active_item != Some(ix);
        self.active_item = Some(ix);
        window.focus(&focus, cx);
        if changed {
            cx.emit(WorkspaceEvent::ActiveItemChanged);
            self.announce_session(cx);
        }
        cx.notify();
    }

    /// Switches to the tab showing `item`, if it is open.
    pub fn activate_item_by_id(
        &mut self,
        item: gpui_kit::EntityId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        match self
            .items
            .iter()
            .position(|open| open.handle.item_id() == item)
        {
            Some(ix) => {
                self.activate_item(ix, window, cx);
                true
            }
            None => false,
        }
    }

    pub(crate) fn tabs_to_close(
        &self,
        id: gpui_kit::EntityId,
        scope: TabCloseScope,
        cx: &App,
    ) -> Vec<gpui_kit::EntityId> {
        let Some(clicked) = self.items.iter().find(|item| item.handle.item_id() == id) else {
            return Vec::new();
        };
        if scope == TabCloseScope::All {
            return self
                .items
                .iter()
                .map(|item| item.handle.item_id())
                .collect();
        }
        let panel = PanelId::from(clicked.dock_item.entity_id());
        let dock = self.dock.read(cx);
        let Some(tree) = dock.layout(DockPlacement::Center) else {
            return Vec::new();
        };
        let Some(node) = tree
            .find_panel_node(panel)
            .and_then(|node| tree.find_node(node))
        else {
            return Vec::new();
        };
        let PaneRef::Tabs { panels, .. } = node.kind() else {
            return Vec::new();
        };
        let Some(clicked_ix) = panels.iter().position(|candidate| *candidate == panel) else {
            return Vec::new();
        };
        panels
            .iter()
            .enumerate()
            .filter(|(ix, _)| match scope {
                TabCloseScope::Current => *ix == clicked_ix,
                TabCloseScope::Others => *ix != clicked_ix,
                TabCloseScope::Left => *ix < clicked_ix,
                TabCloseScope::Right => *ix > clicked_ix,
                TabCloseScope::All => true,
            })
            .filter_map(|(_, panel)| {
                self.items
                    .iter()
                    .find(|item| PanelId::from(item.dock_item.entity_id()) == *panel)
                    .map(|item| item.handle.item_id())
            })
            .collect()
    }

    pub fn close_tabs(
        &mut self,
        id: gpui_kit::EntityId,
        scope: TabCloseScope,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Resolve visual order before mutations collapse or renumber native panes.
        let ids = self.tabs_to_close(id, scope, cx);
        for id in ids {
            if let Some(ix) = self
                .items
                .iter()
                .position(|item| item.handle.item_id() == id)
            {
                self.close_item(ix, window, cx);
            }
        }
    }

    fn close_active_tabs(
        &mut self,
        scope: TabCloseScope,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(id) = self.active_item().map(|item| item.item_id()) {
            self.close_tabs(id, scope, window, cx);
        }
    }

    pub fn close_item(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        if ix >= self.items.len() {
            return;
        }
        let previous_active = self.active_item().map(|item| item.item_id());
        let closed = self.items.remove(ix);
        cx.emit(WorkspaceEvent::ItemsChanged);
        let closed_id = closed.handle.item_id();
        let node = self
            .dock
            .read(cx)
            .layout(DockPlacement::Center)
            .and_then(|tree| tree.find_panel_node(PanelId::from(closed.dock_item.entity_id())));
        self.dock.update(cx, |dock, cx| {
            dock.remove_panel(closed.dock_item.clone(), window, cx)
        });
        closed.handle.close(window, cx);
        if previous_active == Some(closed_id) {
            let replacement = node
                .and_then(|node| {
                    let dock = self.dock.read(cx);
                    let PaneRef::Tabs { panels, active_ix } =
                        dock.layout(DockPlacement::Center)?.find_node(node)?.kind()
                    else {
                        return None;
                    };
                    panels.get(active_ix).copied()
                })
                .and_then(|panel| {
                    self.items
                        .iter()
                        .position(|open| PanelId::from(open.dock_item.entity_id()) == panel)
                });
            self.active_item = replacement
                .or_else(|| (!self.items.is_empty()).then(|| ix.min(self.items.len() - 1)));
            match self.active_item {
                Some(active) => {
                    let focus = self.items[active].handle.focus_handle(cx);
                    window.focus(&focus, cx);
                }
                None => window.focus(&self.focus_handle, cx),
            }
            cx.emit(WorkspaceEvent::ActiveItemChanged);
            self.announce_session(cx);
        } else {
            self.active_item = previous_active.and_then(|id| {
                self.items
                    .iter()
                    .position(|open| open.handle.item_id() == id)
            });
        }
        cx.notify();
    }

    /// Record focus without changing the toolkit's layout or stealing focus.
    pub(crate) fn mark_active(&mut self, id: gpui_kit::EntityId, cx: &mut Context<Self>) {
        let ix = self
            .items
            .iter()
            .position(|open| open.handle.item_id() == id);
        if ix.is_some() && ix != self.active_item {
            self.active_item = ix;
            cx.emit(WorkspaceEvent::ActiveItemChanged);
            self.announce_session(cx);
            cx.notify();
        }
    }

    /// The toolkit calls this only for a true removal, never for a tab move.
    pub(crate) fn item_removed(
        &mut self,
        id: gpui_kit::EntityId,
        bottom: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if bottom {
            if self
                .local_terminal
                .as_ref()
                .is_some_and(|local| local.handle.item_id() == id)
            {
                self.close_local_terminal(window, cx);
            }
        } else if let Some(ix) = self
            .items
            .iter()
            .position(|open| open.handle.item_id() == id)
        {
            self.close_item(ix, window, cx);
        }
    }

    pub fn set_local_terminal_opener(
        &mut self,
        opener: impl Fn(&mut Workspace, &mut Window, &mut Context<Workspace>) + 'static,
    ) {
        self.local_opener = Some(Rc::new(opener));
    }

    pub fn set_local_terminal<T: crate::LocalTerminal>(
        &mut self,
        item: Entity<T>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_local_terminal(window, cx);
        let mut cwd = item.read(cx).cwd(cx);
        let subscription =
            cx.subscribe_in(
                &item,
                window,
                move |this, item, event, window, cx| match event {
                    ItemEvent::Changed => {
                        if let Some(local) = &this.local_terminal {
                            local.item.update(cx, |_, cx| cx.notify());
                        }
                        let now = item.read(cx).cwd(cx);
                        if now != cwd {
                            cwd = now;
                            cx.emit(WorkspaceEvent::LocalDirectoryChanged);
                        }
                        cx.emit(WorkspaceEvent::ItemsChanged);
                        cx.notify();
                    }
                    ItemEvent::CloseRequested => this.close_local_terminal(window, cx),
                },
            );
        let handle: Rc<dyn ItemHandle> = Rc::new(item.clone());
        let id = item.entity_id();
        let workspace = cx.weak_entity();
        let dock_item = cx.new(|item_cx| {
            crate::dock_item::DockItem::new(handle.clone(), workspace, true, window, item_cx)
        });
        let focus = dock_item.read(cx).container_focus_handle();
        let focus_subscription = cx.on_focus_in(&focus, window, move |this, _, _| {
            this.last_command_item = Some(id);
        });
        let height = rems(cx.design().layout.local_terminal_height).to_pixels(window.rem_size());
        self.dock.update(cx, |dock, cx| {
            dock.add_panel_view(
                panel_handle(dock_item.clone()),
                DockPlacement::Bottom,
                Some(height),
                window,
                cx,
            )
        });
        window.focus(&handle.focus_handle(cx), cx);
        self.local_terminal = Some(BottomTerminal {
            item: dock_item,
            handle,
            local: Box::new(item),
            attached: true,
            height,
            _subscription: subscription,
            _focus_subscription: focus_subscription,
        });
        cx.emit(WorkspaceEvent::ItemsChanged);
        cx.notify();
    }

    pub fn toggle_local_terminal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.set_right_panel_maximized(false, cx);
        if self
            .local_terminal
            .as_ref()
            .is_some_and(|local| local.attached)
        {
            self.hide_local_terminal(window, cx);
        } else if self.local_terminal.is_some() {
            self.show_local_terminal(window, cx);
        } else if let Some(opener) = self.local_opener.clone() {
            opener(self, window, cx);
        }
        cx.notify();
    }

    fn schedule_hidden_dock_detach(&mut self, window: &Window, cx: &mut Context<Self>) {
        if self.dock_hide_queued
            || !self
                .local_terminal
                .as_ref()
                .is_some_and(|local| local.attached)
            || self.dock.read(cx).is_dock_open(DockPlacement::Bottom)
        {
            return;
        }
        self.dock_hide_queued = true;
        let workspace = cx.weak_entity();
        cx.defer_in(window, move |_, window, cx| {
            let _ = workspace.update(cx, |workspace, cx| {
                workspace.dock_hide_queued = false;
                if workspace
                    .local_terminal
                    .as_ref()
                    .is_some_and(|local| local.attached)
                    && !workspace.dock.read(cx).is_dock_open(DockPlacement::Bottom)
                {
                    workspace.hide_local_terminal(window, cx);
                }
            });
        });
    }

    fn hide_local_terminal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(local) = self.local_terminal.as_mut().filter(|local| local.attached) else {
            return;
        };
        local.attached = false;
        local.height = self
            .dock
            .read(cx)
            .dock_size(DockPlacement::Bottom)
            .unwrap_or(local.height);
        local
            .item
            .update(cx, |item, _| item.detach_without_closing());
        let item = local.item.clone();
        self.dock.update(cx, |dock, cx| {
            dock.remove_panel(item, window, cx);
            if dock.is_empty(DockPlacement::Bottom, cx) {
                dock.remove_dock(DockPlacement::Bottom, window, cx);
            }
        });
        if let Some(item) = self.active_item() {
            window.focus(&item.focus_handle(cx), cx);
        } else {
            window.focus(&self.focus_handle, cx);
        }
        cx.notify();
    }

    fn show_local_terminal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(local) = self.local_terminal.as_mut().filter(|local| !local.attached) else {
            return;
        };
        local.attached = true;
        let item = local.item.clone();
        let height = local.height;
        let focus = local.handle.focus_handle(cx);
        self.dock.update(cx, |dock, cx| {
            dock.add_panel_view(
                panel_handle(item),
                DockPlacement::Bottom,
                Some(height),
                window,
                cx,
            )
        });
        window.focus(&focus, cx);
        cx.notify();
    }

    pub fn close_local_terminal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(local) = self.local_terminal.take() {
            self.dock.update(cx, |dock, cx| {
                dock.remove_panel(local.item, window, cx);
                if dock.is_empty(DockPlacement::Bottom, cx) {
                    dock.remove_dock(DockPlacement::Bottom, window, cx);
                }
            });
            local.handle.close(window, cx);
            if let Some(item) = self.active_item() {
                window.focus(&item.focus_handle(cx), cx);
            } else {
                window.focus(&self.focus_handle, cx);
            }
            cx.emit(WorkspaceEvent::LocalDirectoryChanged);
            cx.emit(WorkspaceEvent::ItemsChanged);
            cx.notify();
        }
    }

    pub fn local_terminal_cwd(&self, cx: &App) -> Option<PathBuf> {
        self.local_terminal
            .as_ref()
            .and_then(|local| local.local.cwd(cx))
    }

    pub fn change_local_directory(
        &mut self,
        path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        if self.local_terminal.is_none() {
            self.toggle_local_terminal(window, cx);
        }
        if self
            .local_terminal
            .as_ref()
            .is_some_and(|local| !local.attached)
        {
            self.show_local_terminal(window, cx);
        }
        self.local_terminal
            .as_ref()
            .ok_or("Local terminal is unavailable")?
            .local
            .change_directory(path, window, cx)
    }

    pub fn rename_active_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(item) = self.active_item.and_then(|ix| self.items.get(ix)) {
            item.dock_item
                .update(cx, |item, cx| item.start_alias(window, cx));
        }
    }

    // ── New tabs and sessions ────────────────────────────────────────────────

    /// The view shown by the tab strip's "+" button. It receives focus
    /// whenever the menu opens.
    pub fn set_new_tab_menu<V: Render + Focusable>(
        &mut self,
        menu: Entity<V>,
        cx: &mut Context<Self>,
    ) {
        self.new_tab_menu = Some(NewTabMenu {
            focus: menu.read(cx).focus_handle(cx),
            view: menu.into(),
        });
        cx.notify();
    }

    pub fn show_new_tab_menu(&mut self, open: bool, cx: &mut Context<Self>) {
        if self.new_tab_menu_open != open {
            self.new_tab_menu_open = open;
            cx.notify();
        }
    }

    // ── Actions ──────────────────────────────────────────────────────────────

    /// Focused bottom terminal, focused central item, then the active central tab.
    /// An open menu retains its opening context while native popup focus moves.
    pub fn command_item(&self, window: &Window, cx: &App) -> Option<Rc<dyn ItemHandle>> {
        if let Some(local) = &self.local_terminal
            && self.dock.read(cx).is_dock_open(DockPlacement::Bottom)
            && local.item.read(cx).contains_focus(window, cx)
        {
            return Some(local.handle.clone());
        }
        if let Some(item) = self
            .items
            .iter()
            .find(|item| item.dock_item.read(cx).contains_focus(window, cx))
        {
            return Some(item.handle.clone());
        }
        if self.menu_focus.contains_focused(window, cx)
            && let Some(id) = self.menu_item.or(self.last_command_item)
        {
            if let Some(local) = &self.local_terminal
                && local.handle.item_id() == id
                && self.dock.read(cx).is_dock_open(DockPlacement::Bottom)
            {
                return Some(local.handle.clone());
            }
            if let Some(item) = self.items.iter().find(|item| item.handle.item_id() == id) {
                return Some(item.handle.clone());
            }
            return None;
        }
        self.active_item
            .and_then(|ix| self.items.get(ix))
            .map(|item| item.handle.clone())
    }

    pub fn item_command_enabled(&self, command: ItemCommand, window: &Window, cx: &App) -> bool {
        self.command_item(window, cx)
            .is_some_and(|item| item.command_enabled(command, cx))
    }

    pub fn execute_item_command(
        &mut self,
        command: ItemCommand,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(item) = self.command_item(window, cx)
            && item.command_enabled(command, cx)
        {
            item.execute(command, window, cx);
        }
    }

    fn dispatch_item_command(
        &mut self,
        command: ItemCommand,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(item) = self.command_item(window, cx)
            && item.command_enabled(command, cx)
        {
            if matches!(
                command,
                ItemCommand::Find | ItemCommand::FindNextSelection | ItemCommand::SessionSettings
            ) {
                let workspace = cx.weak_entity();
                window.defer(cx, move |window, cx| {
                    let _ = workspace.update(cx, |this, cx| {
                        let still_open = this
                            .items
                            .iter()
                            .any(|open| open.handle.item_id() == item.item_id())
                            || this
                                .local_terminal
                                .as_ref()
                                .is_some_and(|local| local.handle.item_id() == item.item_id());
                        if still_open && item.command_enabled(command, cx) {
                            item.execute(command, window, cx);
                        }
                    });
                });
            } else {
                item.execute(command, window, cx);
            }
        }
    }

    /// The displayed tab label, including its user-defined alias.
    pub fn command_title(&self, window: &Window, cx: &App) -> Option<SharedString> {
        let item = self.command_item(window, cx)?;
        let dock = self
            .items
            .iter()
            .find(|open| open.handle.item_id() == item.item_id())
            .map(|open| &open.dock_item)
            .or_else(|| {
                self.local_terminal
                    .as_ref()
                    .filter(|local| local.handle.item_id() == item.item_id())
                    .map(|local| &local.item)
            });
        Some(
            dock.and_then(|dock| dock.read(cx).alias.clone())
                .unwrap_or_else(|| item.tab_title(cx)),
        )
    }

    pub fn sidebar_is_open(&self) -> bool {
        self.sidebar_open
    }
    pub fn local_terminal_is_visible(&self, cx: &App) -> bool {
        self.local_terminal
            .as_ref()
            .is_some_and(|local| local.attached)
            && self.dock.read(cx).is_dock_open(DockPlacement::Bottom)
    }

    /// Supplies window-specific menus to the toolkit's standard menu bar.
    pub fn set_menu_builder(
        &mut self,
        builder: impl Fn(&Workspace, &Window, &App) -> Vec<Menu> + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.menu_builder = Some(Rc::new(builder));
        self.reload_menu_bar(window, cx);
    }

    fn reload_menu_bar(&mut self, window: &Window, cx: &mut Context<Self>) {
        let Some(builder) = self.menu_builder.clone() else {
            return;
        };
        self.menu_item = self.command_item(window, cx).map(|item| item.item_id());
        let menus = builder(self, window, cx)
            .into_iter()
            .map(Menu::owned)
            .collect();
        if let Some(bar) = &self.app_menu_bar {
            bar.update(cx, |bar, cx| bar.set_menus(menus, cx));
        } else {
            // Construction reads GlobalState once; restore it immediately so
            // other windows and OS menus never inherit this snapshot.
            let previous = GlobalState::global(cx).app_menus().to_vec();
            GlobalState::global_mut(cx).set_app_menus(menus);
            self.app_menu_bar = Some(AppMenuBar::new(cx));
            GlobalState::global_mut(cx).set_app_menus(previous);
        }
    }

    fn prepare_menu(&mut self, window: &Window, cx: &mut Context<Self>) {
        if !self
            .app_menu_bar
            .as_ref()
            .is_some_and(|bar| bar.read(cx).is_open())
        {
            self.menu_item = None;
            self.reload_menu_bar(window, cx);
        }
    }

    /// Handles action `A` whenever focus is inside the workspace.
    ///
    /// This is how a feature adds a command without the workspace knowing
    /// the feature; which keys trigger it is up to the keymap.
    pub fn register_action<A: Action>(
        &mut self,
        handler: impl Fn(&mut Workspace, &A, &mut Window, &mut Context<Workspace>) + 'static,
    ) {
        let handler = Rc::new(handler);
        self.actions.push(Box::new(move |element, cx| {
            let handler = handler.clone();
            element.on_action(cx.listener(move |workspace, action: &A, window, cx| {
                handler(workspace, action, window, cx);
            }))
        }));
    }

    fn on_new_tab(&mut self, _: &NewTab, window: &mut Window, cx: &mut Context<Self>) {
        let workspace = cx.weak_entity();
        window.defer(cx, move |_, cx| {
            let _ = workspace.update(cx, |this, cx| {
                this.show_new_tab_menu(!this.new_tab_menu_open, cx);
            });
        });
    }

    fn on_close_tab(&mut self, _: &CloseTab, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(ix) = self.active_item {
            self.close_item(ix, window, cx);
        }
    }

    fn on_next_tab(&mut self, _: &NextTab, window: &mut Window, cx: &mut Context<Self>) {
        self.cycle_tab(1, window, cx);
    }
    fn on_previous_tab(&mut self, _: &PreviousTab, window: &mut Window, cx: &mut Context<Self>) {
        self.cycle_tab(-1, window, cx);
    }

    fn on_toggle_sidebar(
        &mut self,
        _: &ToggleSidebar,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.toggle_sidebar(window, cx);
    }

    fn on_next_panel(&mut self, _: &NextPanel, window: &mut Window, cx: &mut Context<Self>) {
        if !self.panels.is_empty() {
            let next = if self.sidebar_open {
                (self.active_panel + 1) % self.panels.len()
            } else {
                self.active_panel
            };
            self.activate_panel(next, window, cx);
        }
    }

    // ── Rendering ────────────────────────────────────────────────────────────
}

impl Focusable for Workspace {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();

        let mut root = v_flex()
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus_handle)
            .size_full()
            .bg(Self::window_background(cx))
            .text_color(theme.foreground)
            .on_action(cx.listener(Self::on_new_tab))
            .on_action(cx.listener(Self::on_close_tab))
            .on_action(cx.listener(|this, _: &crate::CloseOtherTabs, window, cx| {
                this.close_active_tabs(TabCloseScope::Others, window, cx)
            }))
            .on_action(cx.listener(|this, _: &crate::CloseTabsLeft, window, cx| {
                this.close_active_tabs(TabCloseScope::Left, window, cx)
            }))
            .on_action(cx.listener(|this, _: &crate::CloseTabsRight, window, cx| {
                this.close_active_tabs(TabCloseScope::Right, window, cx)
            }))
            .on_action(cx.listener(|this, _: &crate::CloseAllTabs, window, cx| {
                this.close_active_tabs(TabCloseScope::All, window, cx)
            }))
            .on_action(cx.listener(Self::on_next_tab))
            .on_action(cx.listener(Self::on_previous_tab))
            .on_action(cx.listener(Self::on_toggle_sidebar))
            .on_action(cx.listener(Self::on_next_panel))
            .on_action(cx.listener(|this, _: &crate::SwapSides, _, cx| this.swap_sides(cx)))
            .on_action(
                cx.listener(|this, _: &crate::ToggleRightPanel, window, cx| {
                    this.toggle_right_panel(window, cx)
                }),
            )
            .on_action(
                cx.listener(|this, _: &crate::ToggleRightPanelMaximized, _, cx| {
                    this.set_right_panel_maximized(!this.right_panel_maximized, cx)
                }),
            )
            .on_action(cx.listener(|this, _: &crate::MoveTabLeft, window, cx| {
                this.move_active_tab(-1, window, cx)
            }))
            .on_action(cx.listener(|this, _: &crate::MoveTabRight, window, cx| {
                this.move_active_tab(1, window, cx)
            }))
            .on_action(cx.listener(|this, _: &crate::SplitLeft, window, cx| {
                this.split_active(gpui_kit::component::Placement::Left, window, cx)
            }))
            .on_action(cx.listener(|this, _: &crate::SplitRight, window, cx| {
                this.split_active(gpui_kit::component::Placement::Right, window, cx)
            }))
            .on_action(cx.listener(|this, _: &crate::SplitUp, window, cx| {
                this.split_active(gpui_kit::component::Placement::Top, window, cx)
            }))
            .on_action(cx.listener(|this, _: &crate::SplitDown, window, cx| {
                this.split_active(gpui_kit::component::Placement::Bottom, window, cx)
            }))
            .on_action(
                cx.listener(|this, _: &crate::NextPane, window, cx| this.focus_pane(1, window, cx)),
            )
            .on_action(cx.listener(|this, _: &crate::PreviousPane, window, cx| {
                this.focus_pane(-1, window, cx)
            }))
            .on_action(cx.listener(|_, _: &crate::RenameTab, window, cx| {
                let workspace = cx.weak_entity();
                window.defer(cx, move |window, cx| {
                    let _ = workspace.update(cx, |this, cx| this.rename_active_tab(window, cx));
                });
            }))
            .on_action(
                cx.listener(|this, _: &crate::ToggleLocalTerminal, window, cx| {
                    this.toggle_local_terminal(window, cx)
                }),
            )
            .on_action(
                cx.listener(|this, _: &crate::CloseLocalTerminal, window, cx| {
                    this.close_local_terminal(window, cx)
                }),
            );
        for register in &self.actions {
            root = register(root, cx);
        }

        let body = self.render_body(window, cx);
        root.child(self.render_title_bar(cx))
            .child(div().flex_1().min_h_0().child(body))
            .child(self.render_footer(cx))
    }
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
