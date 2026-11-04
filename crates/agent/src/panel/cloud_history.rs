//! The Cloud chats of the window's hub, as the history column lists them beside local chats
//! (ADR 0130: only a hub window has them).
//!
//! The listing is the hub's: rows open their chat through the entry the port listed, and they
//! offer only what the port supports (no rename, pin, fork or delete).
use super::AgentPanel;
use crate::cloud_access::CloudAgentsAccess;
use gpui_kit::{App, Context, Task};
use nocterm_cloud_agents_model::{ChatHistoryEntry, ChatSummary, CloudChatError};
use nocterm_hubs_model::HubId;
use std::sync::Arc;

/// Names one listing, so the answer of an older one is dropped.
#[derive(Clone)]
pub(super) struct CloudListingTicket(Arc<()>);

struct Listing {
    ticket: CloudListingTicket,
    hub: HubId,
    _task: Task<()>,
}

pub(super) struct CloudHistory {
    access: Option<CloudAgentsAccess>,
    /// The hub the entries belong to, and the entries.
    listed: Option<(HubId, Vec<Arc<dyn ChatHistoryEntry>>)>,
    listing: Option<Listing>,
    /// Changes whenever the listed rows do, for the history region's stamp.
    revision: u64,
}

impl CloudHistory {
    pub(super) fn new(access: Option<CloudAgentsAccess>) -> Self {
        Self {
            access,
            listed: None,
            listing: None,
            revision: 0,
        }
    }

    /// Lists the chats of the window's hub. Without `again`, a hub already listed (or being
    /// listed) is not asked twice; `again` asks for what changed since, such as a new chat.
    pub(super) fn refresh(&mut self, again: bool, cx: &mut Context<AgentPanel>) {
        let Some(access) = &self.access else {
            return;
        };
        let (Some(hub), Some(launcher)) = (access.shown_hub(cx), access.chats().cloned()) else {
            self.forget();
            return;
        };
        if self
            .listed
            .as_ref()
            .is_some_and(|(listed, _)| *listed != hub)
        {
            self.forget();
        }
        let asked = self
            .listing
            .as_ref()
            .is_some_and(|listing| listing.hub == hub);
        if !again && (asked || self.listed.is_some()) {
            return;
        }
        let ticket = CloudListingTicket(Arc::new(()));
        let completion = ticket.clone();
        let listed = hub.clone();
        let task = cx.spawn(async move |panel, cx| {
            let result = launcher.history(&listed).await;
            let _ = panel.update(cx, |panel, cx| {
                panel.on_cloud_history_listed(&completion, listed, result, cx);
            });
        });
        self.listing = Some(Listing {
            ticket,
            hub,
            _task: task,
        });
    }

    /// Takes the answer of the current listing; an older listing's answer, or a failed one,
    /// leaves the rows as they are. Returns whether the rows changed.
    pub(super) fn accept(
        &mut self,
        ticket: &CloudListingTicket,
        hub: HubId,
        result: Result<Vec<Arc<dyn ChatHistoryEntry>>, CloudChatError>,
    ) -> bool {
        if !self
            .listing
            .as_ref()
            .is_some_and(|listing| Arc::ptr_eq(&listing.ticket.0, &ticket.0))
        {
            return false;
        }
        self.listing = None;
        match result {
            Ok(entries) => {
                self.listed = Some((hub, entries));
                self.revision = self.revision.wrapping_add(1);
                true
            }
            // A hub without Cloud chats lists none; any other failure keeps the earlier rows.
            Err(CloudChatError::Unsupported) => {
                self.forget();
                true
            }
            Err(_) => false,
        }
    }

    fn forget(&mut self) {
        if self.listed.take().is_some() {
            self.revision = self.revision.wrapping_add(1);
        }
        self.listing = None;
    }

    /// The listed entries of the hub the window shows now.
    fn entries(&self, cx: &App) -> &[Arc<dyn ChatHistoryEntry>] {
        let shown = self.access.as_ref().and_then(|access| access.shown_hub(cx));
        match &self.listed {
            Some((hub, entries)) if Some(hub) == shown.as_ref() => entries,
            _ => &[],
        }
    }

    pub(super) fn is_empty(&self, cx: &App) -> bool {
        self.entries(cx).is_empty()
    }

    /// The rows matching `query` (lowercase), in the order the hub listed them. A Cloud chat is
    /// end-to-end encrypted and not loaded, so the search matches what its row shows.
    pub(super) fn matching(&self, query: &str, cx: &App) -> Vec<ChatSummary> {
        self.entries(cx)
            .iter()
            .map(|entry| entry.summary())
            .filter(|row| query.is_empty() || matches_row(row, query))
            .cloned()
            .collect()
    }

    /// The entry listed under `key`, which alone can open its chat.
    pub(super) fn entry(&self, key: &str, cx: &App) -> Option<Arc<dyn ChatHistoryEntry>> {
        self.entries(cx)
            .iter()
            .find(|entry| entry.summary().key == key)
            .cloned()
    }

    pub(super) fn revision(&self) -> u64 {
        self.revision
    }
}

fn matches_row(row: &ChatSummary, query: &str) -> bool {
    [Some(&row.title), Some(&row.host), row.model.as_ref()]
        .into_iter()
        .flatten()
        .any(|text| text.to_lowercase().contains(query))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row() -> ChatSummary {
        ChatSummary::plain(
            "k".into(),
            "Release notes".into(),
            "Scribe".into(),
            Some("gpt-x".into()),
        )
    }

    #[test]
    fn a_cloud_row_matches_its_title_host_or_model() {
        assert!(matches_row(&row(), "release"));
        assert!(matches_row(&row(), "scribe"));
        assert!(matches_row(&row(), "gpt"));
        assert!(!matches_row(&row(), "budget"));
    }
}
