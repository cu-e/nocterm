use std::{rc::Rc, sync::Arc};

use gpui_kit::{
    AnyView, App, Context, Entity, EntityId, EventEmitter, FocusHandle, Focusable, Render,
    SharedString, Window,
};
use nocterm_session::{HostExec, RemoteFs, Target};
use nocterm_ui::IconName;

/// Feature commands shared by menus, shortcuts and future command palettes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ItemCommand {
    Copy,
    Paste,
    SelectAll,
    ClearSelection,
    Find,
    FindNext,
    FindPrevious,
    FindNextSelection,
    Disconnect,
    Reconnect,
    StartRecording,
    StopRecording,
    SessionSettings,
}

/// The content of a tab.
pub trait Item: Render + Focusable + EventEmitter<ItemEvent> {
    /// The tab's label.
    fn tab_title(&self, cx: &App) -> SharedString;

    fn tab_icon(&self, _cx: &App) -> IconName {
        IconName::SquareTerminal
    }

    fn tab_state(&self, _cx: &App) -> TabState {
        TabState::Idle
    }

    /// The remote session this tab shows, if it shows one.
    ///
    /// The sidebar's file browser, and any future integration that acts on
    /// "the current host", reads the active tab's session through this.
    fn session(&self, _cx: &App) -> Option<SessionContext> {
        None
    }

    fn terminal_access(&self) -> Option<Rc<dyn crate::TerminalAccess>> {
        None
    }

    fn command_enabled(&self, _command: ItemCommand, _cx: &App) -> bool {
        false
    }

    fn execute(&mut self, _command: ItemCommand, _window: &mut Window, _cx: &mut Context<Self>) {}

    /// The tab is closing: release whatever it holds.
    fn on_close(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {}
}

/// What an [`Item`] tells the workspace.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ItemEvent {
    /// Its title, state or session changed; the tab strip and anything
    /// following the active session should look again.
    Changed,
    /// It wants its tab closed.
    CloseRequested,
}

/// How a tab's activity is shown in the tab strip.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TabState {
    /// Nothing to report.
    #[default]
    Idle,
    /// Work is under way: connecting, signing in.
    Busy,
    /// It needs the user: a question is waiting.
    Attention,
    /// It is over, or failed.
    Ended,
}

/// The remote session behind a tab.
#[derive(Clone)]
pub struct SessionContext {
    pub target: Target,
    /// The host's file system, when the transport offers one.
    pub fs: Option<Arc<dyn RemoteFs>>,
    /// Runs programs on the host, when the transport can.
    pub exec: Option<Arc<dyn HostExec>>,
    /// Whether the session is up. A disconnected session keeps its context,
    /// so views can show what they showed before, greyed out.
    pub connected: bool,
}

impl SessionContext {
    pub fn new(target: Target, fs: Option<Arc<dyn RemoteFs>>, connected: bool) -> Self {
        Self {
            target,
            fs,
            exec: None,
            connected,
        }
    }

    /// The same context, with the host's program runner.
    pub fn with_exec(mut self, exec: Option<Arc<dyn HostExec>>) -> Self {
        self.exec = exec;
        self
    }

    /// Whether two contexts are the same session in the same state.
    pub fn same_as(&self, other: &Self) -> bool {
        self.target == other.target
            && self.connected == other.connected
            && same_handle(&self.fs, &other.fs)
            && same_handle(&self.exec, &other.exec)
    }
}

fn same_handle<T: ?Sized>(left: &Option<Arc<T>>, right: &Option<Arc<T>>) -> bool {
    match (left, right) {
        (Some(left), Some(right)) => Arc::ptr_eq(left, right),
        (None, None) => true,
        _ => false,
    }
}

/// An [`Item`] of any type, as the workspace holds it.
pub trait ItemHandle: 'static {
    fn item_id(&self) -> EntityId;
    fn terminal_access(&self, cx: &App) -> Option<Rc<dyn crate::TerminalAccess>>;
    fn view(&self) -> AnyView;
    fn tab_title(&self, cx: &App) -> SharedString;
    fn tab_icon(&self, cx: &App) -> IconName;
    fn tab_state(&self, cx: &App) -> TabState;
    fn session(&self, cx: &App) -> Option<SessionContext>;
    fn focus_handle(&self, cx: &App) -> FocusHandle;
    fn close(&self, window: &mut Window, cx: &mut App);
    fn command_enabled(&self, command: ItemCommand, cx: &App) -> bool;
    fn execute(&self, command: ItemCommand, window: &mut Window, cx: &mut App);
}

impl<T: Item> ItemHandle for Entity<T> {
    fn terminal_access(&self, cx: &App) -> Option<Rc<dyn crate::TerminalAccess>> {
        self.read(cx).terminal_access()
    }
    fn item_id(&self) -> EntityId {
        self.entity_id()
    }

    fn view(&self) -> AnyView {
        self.clone().into()
    }

    fn tab_title(&self, cx: &App) -> SharedString {
        self.read(cx).tab_title(cx)
    }

    fn tab_icon(&self, cx: &App) -> IconName {
        self.read(cx).tab_icon(cx)
    }

    fn tab_state(&self, cx: &App) -> TabState {
        self.read(cx).tab_state(cx)
    }

    fn session(&self, cx: &App) -> Option<SessionContext> {
        self.read(cx).session(cx)
    }

    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.read(cx).focus_handle(cx)
    }

    fn close(&self, window: &mut Window, cx: &mut App) {
        self.update(cx, |item, cx| item.on_close(window, cx));
    }

    fn command_enabled(&self, command: ItemCommand, cx: &App) -> bool {
        self.read(cx).command_enabled(command, cx)
    }

    fn execute(&self, command: ItemCommand, window: &mut Window, cx: &mut App) {
        self.update(cx, |item, cx| item.execute(command, window, cx));
    }
}
