//! Owns hydration generations, queued intentions, draft overlays and delivery permits.
pub(in crate::thread) mod load_operation;
use gpui::App;
use nocterm_ai::{
    history::{ChatSummary, HistoryPermit, SavedChat},
    session::Ticket,
    storage::AgentStorageError,
};

/// Runs once the saved chat is hydrated, outside the thread's update.
pub type AfterHydration = Box<dyn FnOnce(&mut App)>;

pub enum ArchiveIntent {
    /// Hands the hydrated chat back to whoever asked for it (open or fork).
    Deliver(AfterHydration),
    Rename {
        name: Option<String>,
    },
    Pin {
        pinned: bool,
    },
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct LoadId(u64);
enum ArchivePhase {
    Resident,
    Archived {
        summary: ChatSummary,
        loading: Option<LoadId>,
    },
}
pub(super) struct DraftOverlay {
    text: String,
    updated: u64,
}
impl DraftOverlay {
    pub(super) fn new(text: String, updated: u64) -> Self {
        Self { text, updated }
    }
    pub(super) fn text(&self) -> &str {
        &self.text
    }
    pub(super) fn updated(&self) -> u64 {
        self.updated
    }
}
pub(super) enum RequestOutcome {
    RunNow(ArchiveIntent),
    AlreadyLoading,
    Start {
        load_id: LoadId,
        chat_id: String,
        ticket: Ticket,
    },
}
pub(super) enum LoadOutcome {
    Stale,
    Failed(AgentStorageError),
    Hydrate {
        chat: Box<SavedChat>,
        overlay: Option<DraftOverlay>,
        intents: Vec<ArchiveIntent>,
    },
}
pub(super) struct EvictionGate {
    detached: bool,
    idle: bool,
    durable: bool,
    clean_draft: bool,
    no_error: bool,
    saved: bool,
}
impl EvictionGate {
    pub(super) fn new(
        detached: bool,
        idle: bool,
        durable: bool,
        clean_draft: bool,
        no_error: bool,
        saved: bool,
    ) -> Self {
        Self {
            detached,
            idle,
            durable,
            clean_draft,
            no_error,
            saved,
        }
    }
    pub(super) fn is_permitted(&self) -> bool {
        self.detached
            && self.idle
            && self.durable
            && self.clean_draft
            && self.no_error
            && self.saved
    }
}
pub(super) struct Archive {
    phase: ArchivePhase,
    pending: Vec<ArchiveIntent>,
    draft_overlay: Option<DraftOverlay>,
    resident_bytes: usize,
    permit: Option<HistoryPermit>,
    next_load: u64,
}
impl Default for Archive {
    fn default() -> Self {
        Self {
            phase: ArchivePhase::Resident,
            pending: Vec::new(),
            draft_overlay: None,
            resident_bytes: 0,
            permit: None,
            next_load: 0,
        }
    }
}
impl Archive {
    pub(super) fn archived(summary: ChatSummary) -> Self {
        Self {
            phase: ArchivePhase::Archived {
                summary,
                loading: None,
            },
            ..Self::default()
        }
    }
    pub(super) fn summary(&self) -> Option<&ChatSummary> {
        match &self.phase {
            ArchivePhase::Resident => None,
            ArchivePhase::Archived { summary, .. } => Some(summary),
        }
    }
    pub(super) fn is_archived(&self) -> bool {
        self.summary().is_some()
    }
    pub(super) fn request(&mut self, intent: ArchiveIntent, ticket: Ticket) -> RequestOutcome {
        let ArchivePhase::Archived { summary, loading } = &mut self.phase else {
            return RequestOutcome::RunNow(intent);
        };
        self.pending.push(intent);
        if loading.is_some() {
            return RequestOutcome::AlreadyLoading;
        }
        self.next_load += 1;
        let load_id = LoadId(self.next_load);
        *loading = Some(load_id);
        RequestOutcome::Start {
            load_id,
            chat_id: summary.id.clone(),
            ticket,
        }
    }
    pub(super) fn edit_draft(&mut self, text: String, updated: u64) -> bool {
        if self
            .draft_overlay
            .as_ref()
            .is_some_and(|overlay| overlay.text() == text)
        {
            return false;
        }
        self.draft_overlay = Some(DraftOverlay::new(text, updated));
        true
    }
    pub(super) fn draft_patch(&self, id: &str, agent: &str) -> Option<SavedChat> {
        self.is_archived().then_some(())?;
        self.draft_overlay.as_ref().map(|overlay| {
            SavedChat::archive_draft(
                id.to_owned(),
                agent.to_owned(),
                overlay.text.clone(),
                overlay.updated,
            )
        })
    }
    pub(super) fn accept(
        &mut self,
        load_id: LoadId,
        current: bool,
        result: Result<SavedChat, AgentStorageError>,
        permit: HistoryPermit,
    ) -> LoadOutcome {
        let ArchivePhase::Archived { loading, .. } = &mut self.phase else {
            return LoadOutcome::Stale;
        };
        if !current || *loading != Some(load_id) {
            return LoadOutcome::Stale;
        }
        *loading = None;
        match result {
            Ok(chat) => {
                self.permit = Some(permit);
                LoadOutcome::Hydrate {
                    chat: Box::new(chat),
                    overlay: self.draft_overlay.take(),
                    intents: std::mem::take(&mut self.pending),
                }
            }
            Err(error) => {
                self.pending.clear();
                LoadOutcome::Failed(error)
            }
        }
    }
    pub(super) fn hydration_committed(&mut self, resident_bytes: usize) {
        self.phase = ArchivePhase::Resident;
        self.resident_bytes = resident_bytes;
    }
    pub(super) fn release_delivery_permit(&mut self) {
        self.permit = None;
    }
    pub(super) fn resident_bytes(&self) -> usize {
        self.resident_bytes
    }
    pub(super) fn request_evict(&mut self, gate: EvictionGate, summary: ChatSummary) -> bool {
        if self.is_archived() || !gate.is_permitted() {
            return false;
        }
        self.phase = ArchivePhase::Archived {
            summary,
            loading: None,
        };
        self.resident_bytes = 0;
        self.draft_overlay = None;
        true
    }
}
#[cfg(test)]
mod tests;
