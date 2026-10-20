//! Only user-facing connection metadata crosses this boundary.
use gpui::{Image, SharedString};
use nocterm_session::Target;
use std::sync::Arc;

#[derive(Clone, Debug)]
pub struct ConnectionSummary {
    pub id: SharedString,
    pub name: SharedString,
    pub group: Option<SharedString>,
    pub description: SharedString,
    pub target: Target,
    /// The server's system icon, in its colour; none shows a generic server.
    pub icon: Option<Arc<Image>>,
    /// The flag of the server's country, once loaded.
    pub flag: Option<Arc<Image>>,
}
