//! What a chat may do with the window it is attached to, without the shell.
//!
//! Chats read terminals and saved connections and open, show and close the
//! background sessions they own only through [`ChatWorkspace`]. The panel
//! supplies [`WorkspaceSeat`], the one implementation over a workspace.
use gpui_kit::{App, EntityId, Subscription, WeakEntity, Window};
use nocterm_workspace::{ConnectionSummary, ExpectedSession, TerminalEntry, Workspace};
use std::rc::Rc;

pub(crate) trait ChatWorkspace: 'static {
    /// The identity of the window's workspace, stable while it lives.
    fn id(&self) -> EntityId;
    fn is_open(&self) -> bool;
    /// Foreground and background terminals; none once the workspace closed.
    fn terminals(&self, cx: &App) -> Option<Vec<TerminalEntry>>;
    fn connections(&self, cx: &App) -> Vec<ConnectionSummary>;
    /// Whether saved connections can be opened in this window.
    fn opens_connections(&self, cx: &App) -> bool;
    /// Calls `changed` whenever the workspace's membership or placement
    /// changes, while the workspace lives.
    fn observe(&self, changed: Box<dyn Fn(&mut App)>, cx: &mut App) -> Option<Subscription>;
    /// Opens saved connection `profile` without a tab and returns its item.
    /// Called outside workspace updates: opening updates the workspace.
    fn open_background(&self, profile: &str, window: &mut Window, cx: &mut App)
    -> Option<EntityId>;
    fn show_background(&self, item: EntityId, window: &mut Window, cx: &mut App);
    fn close_background(&self, item: EntityId, window: &mut Window, cx: &mut App);
    /// Closes background `item` only while it still runs connection
    /// `expected`, checked in the same transition that closes it.
    fn close_if_original(
        &self,
        item: EntityId,
        expected: Option<ExpectedSession>,
        window: &mut Window,
        cx: &mut App,
    ) -> bool;
}

pub(crate) type Seat = Rc<dyn ChatWorkspace>;

/// The workspace a panel is attached to, held weakly.
pub(crate) struct WorkspaceSeat(WeakEntity<Workspace>);

impl WorkspaceSeat {
    pub(crate) fn new(workspace: WeakEntity<Workspace>) -> Self {
        Self(workspace)
    }
}

/// The connection `item` currently runs, as the workspace places it.
fn current(workspace: &Workspace, item: EntityId, cx: &App) -> Option<ExpectedSession> {
    workspace
        .terminals(cx)
        .into_iter()
        .find(|entry| entry.item == item)
        .and_then(|entry| entry.capabilities.state().info(cx))
        .map(|info| info.expected)
}

impl ChatWorkspace for WorkspaceSeat {
    fn id(&self) -> EntityId {
        self.0.entity_id()
    }
    fn is_open(&self) -> bool {
        self.0.upgrade().is_some()
    }
    fn terminals(&self, cx: &App) -> Option<Vec<TerminalEntry>> {
        Some(self.0.upgrade()?.read(cx).terminals(cx))
    }
    fn connections(&self, cx: &App) -> Vec<ConnectionSummary> {
        self.0
            .upgrade()
            .and_then(|workspace| workspace.read(cx).connection_directory())
            .map(|directory| directory.connections(cx))
            .unwrap_or_default()
    }
    fn opens_connections(&self, cx: &App) -> bool {
        self.0
            .upgrade()
            .is_some_and(|workspace| workspace.read(cx).connection_directory().is_some())
    }
    fn observe(&self, changed: Box<dyn Fn(&mut App)>, cx: &mut App) -> Option<Subscription> {
        let workspace = self.0.upgrade()?;
        Some(cx.subscribe(&workspace, move |_, _, cx| changed(cx)))
    }
    fn open_background(
        &self,
        profile: &str,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<EntityId> {
        let directory = self.0.upgrade()?.read(cx).connection_directory()?;
        directory.open_background(profile, &self.0, window, cx)
    }
    fn show_background(&self, item: EntityId, window: &mut Window, cx: &mut App) {
        let _ = self.0.update(cx, |workspace, cx| {
            workspace.show_background_session(item, window, cx)
        });
    }
    fn close_background(&self, item: EntityId, window: &mut Window, cx: &mut App) {
        let _ = self.0.update(cx, |workspace, cx| {
            workspace.close_background_session(item, window, cx)
        });
    }
    fn close_if_original(
        &self,
        item: EntityId,
        expected: Option<ExpectedSession>,
        window: &mut Window,
        cx: &mut App,
    ) -> bool {
        self.0
            .update(cx, |workspace, cx| {
                if !workspace.is_background(item) || current(workspace, item, cx) != expected {
                    return false;
                }
                workspace.close_background_session(item, window, cx);
                true
            })
            .unwrap_or(false)
    }
}

/// The connection `item` runs now, read through the seat.
pub(crate) fn current_session(seat: &Seat, item: EntityId, cx: &App) -> Option<ExpectedSession> {
    seat.terminals(cx)?
        .into_iter()
        .find(|entry| entry.item == item)
        .and_then(|entry| entry.capabilities.state().info(cx))
        .map(|info| info.expected)
}
