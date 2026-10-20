//! Composer editing owns future context separately from active grants and saved defaults.
use super::Attachment;
use nocterm_workspace_ports::ConnectionSummary;

impl Attachment {
    /// Whether this attachment gives the agent the saved server `summary`:
    /// the server itself, or the folder it is filed under.
    pub(crate) fn covers_server(&self, summary: &ConnectionSummary) -> bool {
        match self {
            Self::Connection(id) => summary.id.as_ref() == id,
            Self::Group(group) => summary
                .group
                .as_ref()
                .is_some_and(|name| name.as_ref() == group),
            Self::Terminal(_) | Self::UnavailableLocal(_) => false,
        }
    }
}
