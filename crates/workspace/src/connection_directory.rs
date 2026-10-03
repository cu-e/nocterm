//! Only user-facing connection metadata crosses this boundary.
use gpui_kit::{App, Context, SharedString, Window};
use nocterm_session::Target;

use crate::Workspace;

#[derive(Clone, Debug)]
pub struct ConnectionSummary {
    pub id: SharedString,
    pub name: SharedString,
    pub group: Option<SharedString>,
    pub description: SharedString,
    pub target: Target,
}

pub trait ConnectionDirectory: 'static {
    fn connections(&self, cx: &App) -> Vec<ConnectionSummary>;
    /// User-triggered opening through the normal authentication UI.
    fn open(&self, id: &str, window: &mut Window, cx: &mut Context<Workspace>) -> bool;
}
