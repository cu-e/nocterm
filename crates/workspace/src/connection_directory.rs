//! Only user-facing connection metadata crosses this boundary.
use gpui_kit::{App, EntityId, SharedString, WeakEntity, Window};

use crate::{ConnectionSummary, Workspace};

/// Called with the application context, never from inside a workspace
/// update: opening a connection updates the workspace itself.
pub trait ConnectionDirectory: 'static {
    fn connections(&self, cx: &App) -> Vec<ConnectionSummary>;
    /// Recently opened saved servers, most recent first, using current metadata.
    /// Quick connections and deleted profiles are excluded.
    fn recent_connections(&self, _: &App) -> Vec<ConnectionSummary> {
        Vec::new()
    }
    /// All named groups, including groups without connections.
    fn groups(&self, cx: &App) -> Vec<SharedString> {
        let mut groups: Vec<_> = self
            .connections(cx)
            .into_iter()
            .filter_map(|connection| connection.group)
            .collect();
        groups.sort();
        groups.dedup();
        groups
    }
    /// Opens a saved connection in a new tab, through the normal
    /// authentication UI, as if chosen in the sidebar.
    fn open(
        &self,
        id: &str,
        workspace: &WeakEntity<Workspace>,
        window: &mut Window,
        cx: &mut App,
    ) -> bool;
    /// Opens a saved connection without a tab, for an agent, and returns the
    /// session's item. Not recorded among recent connections.
    fn open_background(
        &self,
        id: &str,
        workspace: &WeakEntity<Workspace>,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<EntityId>;
}
