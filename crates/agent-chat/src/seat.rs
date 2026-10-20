//! What a chat may do with the window it is attached to, without the shell.
//!
//! Chats read terminals and saved connections and open, show and close the
//! background sessions they own only through [`ChatWorkspace`]. The window
//! shell supplies the implementation.
use gpui::{App, EntityId, Subscription, Window};
use nocterm_workspace_ports::{ConnectionSummary, ExpectedSession, TerminalEntry};
use std::rc::Rc;

pub trait ChatWorkspace: 'static {
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

pub type Seat = Rc<dyn ChatWorkspace>;

/// The connection `item` runs now, read through the seat.
pub fn current_session(seat: &Seat, item: EntityId, cx: &App) -> Option<ExpectedSession> {
    seat.terminals(cx)?
        .into_iter()
        .find(|entry| entry.item == item)
        .and_then(|entry| entry.capabilities.state().info(cx))
        .map(|info| info.expected)
}
