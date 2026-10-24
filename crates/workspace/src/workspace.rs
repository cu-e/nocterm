use std::{cell::Cell, path::PathBuf, rc::Rc, sync::Arc};

use gpui_kit::{
    Action, AnyView, App, ClipboardItem, Context, Entity, EntityId, EventEmitter, FocusHandle,
    Focusable, Menu, Pixels, SharedString, Subscription, Window,
    component::dock::{
        DockArea, DockEvent, DockPlacement, DockSkin, PaneRef, PanelId, PanelStyle, panel_handle,
    },
    div,
    prelude::*,
    rems,
};
use nocterm_plugin_model::PluginCatalog;
use nocterm_session::{Auth, HostAddress, Target};

mod background;
mod bindings;
mod chrome;
mod composition;
mod contexts;
pub use contexts::{ContextSelectionReceipt, ContextWindowReceipt, ProjectSurfaceReceipt};
mod docking;
mod general_target;
mod groups;
pub use general_target::GeneralTerminalTarget;
mod interaction;
mod layout;
mod local_dock;
mod openers;
mod project_surface;
mod project_terminal;
#[cfg(any(test, feature = "test-support"))]
pub(crate) use project_terminal::project_terminal_selection;
pub use project_terminal::{admit_project_terminal, validate_project_terminal_target};
mod queries;
pub use project_surface::{ProjectSurface, ProjectSurfaceEvent, ProjectSurfaceFactory};
pub(crate) mod surface;
mod surfaces;
mod tabs;
pub use openers::SessionFactory;
mod actions;
mod panel_groups;
use panel_groups::GroupKey;
mod ports;
mod registration;
pub use registration::{
    ExpandablePanel, PanelRegistration, PanelRequest, RegistrationError, WidgetRegistration,
};
mod layout_switch;
mod sessions;
mod shell_layout;
pub use layout_switch::{LayoutChoice, LayoutSwitchError, MalformedLayouts};

use crate::{
    CloseTab, Item, ItemEvent, ItemHandle, NewTab, NextPanel, NextTab, PreviousTab, SessionContext,
    ToggleSidebar,
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
    /// Jump hosts, resolved before opening; the session's proxy reaches the first.
    pub jumps: Vec<nocterm_session::ResolvedJump>,
}

impl SessionSpec {
    /// The machine this spec reaches and the jump hosts in front of it.
    pub fn address(&self) -> HostAddress {
        HostAddress::new(
            self.target.clone(),
            self.jumps
                .iter()
                .map(|jump| jump.host.target.clone())
                .collect(),
        )
    }
}

/// What the workspace tells its observers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkspaceEvent {
    /// Terminal membership or metadata changed.
    ItemsChanged,
    /// One retained surface became visible; active projections now belong to it.
    ContextChanged,
    /// A retained surface changed while hidden; active projections stay unchanged.
    SurfaceItemsChanged(ProjectSurfaceReceipt),
    /// Another tab became active, or the last one closed.
    ActiveItemChanged,
    /// The session behind the active tab is a different one, or changed
    /// state.
    ActiveSessionChanged,
    /// Local shell reported a current-directory change.
    LocalDirectoryChanged,
    /// Saved server descriptors or discovered facts changed.
    ConnectionsChanged,
    /// A panel group was shown or hidden.
    PanelVisibilityChanged,
}

/// Close commands use the clicked tab's pane and its current visual order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TabCloseScope {
    Current,
    Others,
    Left,
    Right,
    /// All registered tabs across panes in the clicked tab's dock placement.
    All,
}

type MenuBuilder = Rc<dyn Fn(&Workspace, &Window, &App) -> Vec<Menu>>;

struct NewTabMenu {
    view: AnyView,
    focus: FocusHandle,
}

/// An immutable observation of a registered tab, independent of native view handles.
#[derive(Clone, Debug)]
pub struct TabSnapshot {
    id: EntityId,
    alias: Option<SharedString>,
    local: bool,
    location: Option<(DockPlacement, gpui_kit::component::dock::NodeId)>,
    group_color: Option<usize>,
    visible: bool,
}
impl TabSnapshot {
    pub fn id(&self) -> EntityId {
        self.id
    }
    pub fn alias(&self) -> Option<&str> {
        self.alias.as_deref()
    }
    pub fn is_local(&self) -> bool {
        self.local
    }
    pub fn location(&self) -> Option<(DockPlacement, gpui_kit::component::dock::NodeId)> {
        self.location
    }
    pub fn group_color(&self) -> Option<usize> {
        self.group_color
    }
    pub fn is_visible(&self) -> bool {
        self.visible
    }
}

/// One current native pane's registered tab identities in visual order.
#[derive(Clone, Debug)]
pub struct TabPane {
    placement: DockPlacement,
    node: gpui_kit::component::dock::NodeId,
    items: Vec<EntityId>,
}
impl TabPane {
    pub fn placement(&self) -> DockPlacement {
        self.placement
    }
    pub fn node(&self) -> gpui_kit::component::dock::NodeId {
        self.node
    }
    pub fn items(&self) -> &[EntityId] {
        &self.items
    }
}

/// Collaborators shared by the windows composed by one application installation.
#[derive(Clone)]
pub struct Deps {
    layout: nocterm_ui::LayoutHandle,
    catalog: Arc<PluginCatalog>,
    settings: Option<nocterm_store::SettingsHandle>,
}

impl Deps {
    /// Dependencies whose catalog lists the shell's own contributions and groups only, so a
    /// panel or widget of any other plugin is refused until the application supplies its
    /// catalog with [`Deps::with_catalog`].
    pub fn new(layout: nocterm_ui::LayoutHandle) -> Self {
        let catalog = crate::plugin::catalog(&[], &[])
            .expect("the shell's own contributions form a valid catalog");
        Self {
            layout,
            catalog: Arc::new(catalog),
            settings: None,
        }
    }

    /// Registers panels and widgets against `catalog`, which holds the shell's own
    /// contributions and groups beside those of the plugins.
    pub fn with_catalog(mut self, catalog: impl Into<Arc<PluginCatalog>>) -> Self {
        self.catalog = catalog.into();
        self
    }

    /// Reads and writes the selected layout and the saved layouts through `settings`, which the
    /// windows of the application share. Without it each window keeps its own in-memory
    /// settings, which is how tests compose a window.
    pub fn with_settings(mut self, settings: nocterm_store::SettingsHandle) -> Self {
        self.settings = Some(settings);
        self
    }
}

pub struct Workspace {
    focus_handle: FocusHandle,
    surfaces: surfaces::WindowSurfaces,
    chrome: chrome::Chrome,
    panels: panel_groups::PanelGroups,
    body: layout::Body,
    shell: shell_layout::ShellLayout,
    ports: ports::WorkspacePorts,
    actions: actions::ActionRegistry,
    registrations: registration::Registrations,
}

impl EventEmitter<WorkspaceEvent> for Workspace {}

impl Workspace {
    pub fn new(deps: Deps, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let settings = deps.settings.unwrap_or_else(|| {
            nocterm_store::SettingsConfig::in_memory(Default::default()).install(cx)
        });
        let mut shell = shell_layout::ShellLayout::new(
            &deps.layout,
            settings.clone(),
            deps.catalog.clone(),
            cx,
        );
        let workspace = cx.weak_entity();
        let window_handle = window.window_handle();
        shell.follow(cx, move |cx| {
            let workspace = workspace.clone();
            let _ = window_handle.update(cx, move |_, window, cx| {
                let _ = workspace.update(cx, |this, cx| this.layout_settings_changed(window, cx));
            });
        });
        let mut this = Self {
            focus_handle: cx.focus_handle(),
            surfaces: surfaces::WindowSurfaces::new(window, cx),
            chrome: chrome::Chrome::new(cx.focus_handle()),
            panels: panel_groups::PanelGroups::new(),
            body: layout::Body::new(deps.layout.clone()),
            shell,
            ports: ports::WorkspacePorts::default(),
            actions: actions::ActionRegistry::default(),
            registrations: registration::Registrations::new(deps.catalog),
        };
        this.register_action::<crate::EditCopy>(|_, _, window, cx| {
            window.dispatch_action(Box::new(gpui_kit::component::input::Copy), cx)
        });
        this.register_action::<crate::EditPaste>(|_, _, window, cx| {
            window.dispatch_action(Box::new(gpui_kit::component::input::Paste), cx)
        });
        this.register_action::<crate::SelectAll>(|_, _, window, cx| {
            window.dispatch_action(Box::new(gpui_kit::component::input::SelectAll), cx)
        });
        this.register_action::<crate::CopyConnectionName>(|this, _, window, cx| {
            if let Some(title) = this.command_title(window, cx) {
                cx.write_to_clipboard(ClipboardItem::new_string(title.to_string()));
            }
        });
        this
    }

    // ── New tabs and sessions ────────────────────────────────────────────────

    /// The view shown by the tab strip's "+" button. It receives focus
    /// whenever the menu opens.
    pub fn set_new_tab_menu<V: Render + Focusable>(
        &mut self,
        menu: Entity<V>,
        cx: &mut Context<Self>,
    ) {
        let focus = menu.read(cx).focus_handle(cx);
        self.chrome.install_new_tab(menu.into(), focus);
        cx.notify();
    }

    pub fn show_new_tab_menu(&mut self, open: bool, cx: &mut Context<Self>) {
        if self.chrome.show_new_tab(open) {
            cx.notify();
        }
    }

    // ── Actions ──────────────────────────────────────────────────────────────

    /// Handles action `A` whenever focus is inside the workspace.
    ///
    /// This is how a feature adds a command without the workspace knowing
    /// the feature; which keys trigger it is up to the keymap.
    pub fn register_action<A: Action>(
        &mut self,
        handler: impl Fn(&mut Workspace, &A, &mut Window, &mut Context<Workspace>) + 'static,
    ) {
        self.actions.register(handler);
    }

    fn on_new_tab(&mut self, _: &NewTab, window: &mut Window, cx: &mut Context<Self>) {
        let selection = self.context_selection_receipt(cx);
        let workspace = cx.weak_entity();
        window.defer(cx, move |_, cx| {
            let _ = workspace.update(cx, |this, cx| {
                if this.accepts_context_selection(selection, cx) {
                    this.show_new_tab_menu(!this.chrome.new_tab_open(), cx);
                }
            });
        });
    }

    fn on_close_tab(&mut self, _: &CloseTab, window: &mut Window, cx: &mut Context<Self>) {
        self.close_active_tabs(TabCloseScope::Current, window, cx);
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
        if let Some(next) = self.panels.next(GroupKey::Navigation) {
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
        let root = self.actions.bind(&self.focus_handle, cx);

        let regions = chrome::Regions::new(
            self.surfaces.frame(cx),
            self.context_selection_receipt(cx),
            &self.panels,
            self.ports.directory(),
            (&self.shell, self.registrations.catalog()),
        );
        let body = self
            .chrome
            .render_body(&mut self.body, &regions, window, cx);
        root.child(
            self.chrome
                .render_title_bar(self.context_selection_receipt(cx), cx),
        )
        .child(div().flex_1().min_h_0().child(body))
        .child(self.chrome.render_footer(&regions, cx))
    }
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;

#[cfg(test)]
mod focus_transition_tests;
#[cfg(test)]
mod layout_switch_tests;
#[cfg(test)]
mod native_terminal_tab_tests;
#[cfg(test)]
mod panel_availability_tests;
#[cfg(test)]
mod panel_chrome_tests;
