use std::{cell::Cell, path::PathBuf, rc::Rc};

#[cfg(test)]
use gpui_kit::component::dock::InsertTarget;
use gpui_kit::{
    Action, AnyView, App, ClipboardItem, Context, Div, Entity, EntityId, EventEmitter, FocusHandle,
    Focusable, Menu, Pixels, SharedString, Subscription, Window,
    base::GlobalState,
    component::{
        ActiveTheme as _,
        dock::{
            DockArea, DockEvent, DockPlacement, DockSkin, PaneRef, PanelId, PanelStyle,
            panel_handle,
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
mod commands;
mod docking;
mod groups;
mod items;
mod layout;
mod local_dock;
mod tabs;
use local_dock::LocalOpener;
mod openers;
mod panels;
mod panes;
mod right_panel;
mod sessions;
mod terminal_target;

use crate::{
    CloseTab, Item, ItemEvent, ItemHandle, KEY_CONTEXT, NewTab, NextPanel, NextTab, PanelHandle,
    PreviousTab, SessionContext, ToggleSidebar,
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
    /// All registered tabs across panes in the clicked tab's dock placement.
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
    local: Option<Box<dyn crate::local_terminal::LocalTerminalHandle>>,
    _subscription: Subscription,
    _focus_subscription: Subscription,
}

pub struct Workspace {
    focus_handle: FocusHandle,
    dock: Entity<DockArea>,
    _dock_subscription: Subscription,
    _dock_observer: Subscription,
    selected_local_id: Option<EntityId>,
    local_terminal_height: Option<Pixels>,
    local_opener: Option<LocalOpener>,
    local_header_click_origin: Rc<Cell<Option<EntityId>>>,
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
        dock.update(cx, |dock, cx| {
            dock.set_closed_bottom_strip_visible(false, cx);
            dock.set_last_panel_drag_between_regions(true, window, cx);
        });
        let subscription = cx.subscribe_in(&dock, window, |this, _, _: &DockEvent, window, cx| {
            this.reconcile_empty_regions(window, cx);
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
            this.reconcile_bottom_visibility(window, cx);
        });
        let mut this = Self {
            dock,
            _dock_subscription: subscription,
            _dock_observer: observer,
            selected_local_id: None,
            local_terminal_height: None,
            local_header_click_origin: Rc::default(),
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
        if let Some(id) = self.command_item(window, cx).map(|item| item.item_id()) {
            self.close_item_by_id(id, window, cx);
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
    #[expect(clippy::too_many_lines, reason = "predates the limit")]
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

#[cfg(test)]
mod native_terminal_tab_tests;
