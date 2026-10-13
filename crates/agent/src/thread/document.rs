//! Durable chat transitions own semantic revisions; capture only hands off existing intent.
use gpui_kit::Task;
use nocterm_ai::{
    acp,
    history::{HistoryCodecError, SavedChat, SharedChat},
    thread::{Entry, ThreadChange, ThreadState},
};
use std::{collections::HashSet, sync::Arc};
mod budget;
mod metadata;
mod revision;
mod transcript;
use budget::BudgetState;
use metadata::{Identity, Metadata};
pub(crate) use metadata::{Inputs, Resume};
use revision::PersistenceState;
pub(crate) use transcript::ToolMetadataPatch;
use transcript::Transcript;

pub(crate) struct ChatDocument {
    identity: Identity,
    metadata: Metadata,
    transcript: Transcript,
    persistence: PersistenceState,
    budget: BudgetState,
    changed_rows: HashSet<usize>,
    draft_debounce: Option<Task<()>>,
}
pub(super) struct DocumentRestart {
    identity: Identity,
    metadata: Metadata,
    transcript: Transcript,
    persistence: PersistenceState,
}
impl DocumentRestart {
    pub(super) fn agent_id(&self) -> &str {
        self.identity.agent()
    }
}
impl ChatDocument {
    pub(super) fn new(agent: String, id: String, updated: u64) -> Self {
        Self {
            identity: Identity::new(agent, id),
            metadata: Metadata::new(updated),
            transcript: Transcript::default(),
            persistence: PersistenceState::default(),
            budget: BudgetState::default(),
            changed_rows: HashSet::new(),
            draft_debounce: None,
        }
    }
    pub(super) fn restarted(data: DocumentRestart) -> Self {
        let mut document = Self::new(
            data.identity.agent().into(),
            data.identity.id().into(),
            data.metadata.updated(),
        );
        document.hydrate_restart(data);
        document
    }
    pub(super) fn fork_created(&mut self) {
        self.persistence.fork_created();
    }
    pub(crate) fn agent_id(&self) -> &str {
        self.identity.agent()
    }
    pub(crate) fn chat_id(&self) -> &str {
        self.identity.id()
    }
    pub(crate) fn name(&self) -> Option<&str> {
        self.metadata.name()
    }
    pub(crate) fn pinned(&self) -> bool {
        self.metadata.pinned()
    }
    pub(crate) fn updated(&self) -> u64 {
        self.metadata.updated()
    }
    pub(crate) fn pending_history(&self) -> bool {
        self.metadata.pending_history()
    }
    pub(crate) fn transcript(&self) -> &ThreadState {
        self.transcript.state()
    }
    pub(crate) fn entries(&self) -> &[Entry] {
        &self.transcript().entries
    }
    pub(crate) fn desired_revision(&self) -> u64 {
        self.persistence.desired()
    }
    pub(crate) fn published_revision(&self) -> u64 {
        self.persistence.published_revision()
    }
    pub(crate) fn persistence_error(&self) -> Option<&crate::runtime::AgentPersistenceError> {
        self.persistence.failure()
    }
    pub(crate) fn history_present(&self) -> bool {
        self.persistence.history_present()
    }
    pub(crate) fn draft_changed(&self) -> bool {
        self.persistence.draft_changed()
    }
    pub(crate) fn clean(&self) -> bool {
        self.persistence.clean()
    }
    fn changed(&mut self, now: u64, draft: bool) {
        self.persistence.changed(draft);
        self.metadata.timestamp(now);
    }
    pub(super) fn imported_clean(&mut self, chat: &mut SavedChat, pending: bool) {
        self.identity = Identity::new(chat.agent_id.clone(), chat.id.clone());
        self.metadata = Metadata::restored(chat.name.take(), chat.pinned, chat.updated, pending);
        self.budget = BudgetState::restored(chat.prepared_budget());
        self.transcript = Transcript::restored(
            std::mem::take(&mut chat.entries),
            std::mem::take(&mut chat.times),
            chat.title.take(),
        );
        self.persistence.imported_clean();
        self.persistence.begin_hydration();
        self.draft_debounce = None;
        self.changed_rows.clear();
    }
    pub(super) fn imported_with_overlay(
        &mut self,
        chat: &mut SavedChat,
        pending: bool,
        updated: u64,
    ) {
        let persistence = std::mem::take(&mut self.persistence);
        self.imported_clean(chat, pending);
        self.persistence = persistence;
        self.persistence.begin_hydration();
        self.metadata.timestamp(updated);
    }
    pub(super) fn archived_clean(&mut self, summary: &nocterm_ai::history::ChatSummary) {
        self.identity = Identity::new(summary.agent_id.clone(), summary.id.clone());
        self.metadata =
            Metadata::restored(summary.name.clone(), summary.pinned, summary.updated, false);
        self.persistence.imported_clean();
    }
    pub(super) fn establish_clean_inputs(&mut self, input: &Inputs<'_>) {
        if self.clean() || !self.persistence.inputs_ready() {
            self.persistence.remember_inputs(input);
        }
    }
    pub(super) fn accept_inputs(&mut self, input: &Inputs<'_>, now: u64, draft: bool) -> bool {
        if self.persistence.inputs_match(input) {
            return false;
        }
        self.persistence.remember_inputs(input);
        if !self.history_present()
            && self.name().is_none()
            && !self.pinned()
            && self.entries().is_empty()
            && !input.has_content()
        {
            return false;
        }
        self.changed(now, draft);
        true
    }
    pub(super) fn rename(&mut self, name: Option<String>, now: u64) -> bool {
        if !self.metadata.rename(name) {
            return false;
        }
        self.changed(now, false);
        true
    }
    pub(super) fn pin(&mut self, pinned: bool, now: u64) -> bool {
        if !self.metadata.pin(pinned) {
            return false;
        }
        self.changed(now, false);
        true
    }
    pub(super) fn set_pending_history(&mut self, pending: bool, now: u64) -> bool {
        if !self.metadata.pending(pending) {
            return false;
        }
        self.changed(now, false);
        true
    }
    pub(super) fn edited_archived_draft(&mut self, now: u64) {
        self.changed(now, true);
    }
    pub(super) fn apply_transcript(
        &mut self,
        update: acp::SessionUpdate,
        now: u64,
    ) -> ThreadChange {
        let (change, semantic) = self.transcript.apply(update);
        if semantic {
            self.changed(now, false);
        }
        change
    }
    pub(super) fn append_user(
        &mut self,
        content: Vec<acp::ContentBlock>,
        now: u64,
        input: &Inputs<'_>,
    ) {
        self.transcript.append_user(content);
        self.persistence.remember_inputs(input);
        self.mark_row(self.entries().len() - 1);
        self.changed(now, false);
    }
    pub(super) fn tokens(&mut self, usage: Option<acp::Usage>) {
        self.transcript.tokens(usage);
    }
    pub(super) fn patch_tool_rows(&mut self, patches: Vec<ToolMetadataPatch>, now: u64) {
        let first = SavedChat::saved_entry_start(self.entries().len());
        let mut semantic = false;
        for patch in patches {
            let row = patch.row();
            if self.transcript.patch_tool(patch) {
                self.mark_row(row);
                semantic |= row >= first;
            }
        }
        if semantic {
            self.changed(now, false);
        }
    }
    pub(super) fn finish_pending_tools(&mut self, now: u64) {
        let first = SavedChat::saved_entry_start(self.entries().len());
        let rows = self.transcript.finish_pending_tools();
        let semantic = rows.iter().any(|row| *row >= first);
        for row in rows {
            self.mark_row(row);
        }
        if semantic {
            self.changed(now, false);
        }
    }
    pub(crate) fn take_changed_rows(&mut self) -> HashSet<usize> {
        std::mem::take(&mut self.changed_rows)
    }
    pub(super) fn mark_row(&mut self, row: usize) {
        self.changed_rows.insert(row);
        self.budget.mark(row);
    }
    pub(super) fn refresh_budget(&mut self) -> Result<(), HistoryCodecError> {
        self.budget.refresh(&self.transcript.state().entries)
    }
    pub(crate) fn resident_bytes(&self) -> usize {
        self.budget.resident_bytes()
    }
    pub(super) fn validate_candidate(
        &mut self,
        metadata: &SavedChat,
        encoded: impl Iterator<Item = usize>,
        memory: impl Iterator<Item = usize>,
    ) -> Result<(), HistoryCodecError> {
        self.refresh_budget()?;
        self.budget.validate(metadata, encoded, memory)
    }
    pub(super) fn metadata(&self, input: &Inputs<'_>) -> SavedChat {
        let mut chat = SavedChat::for_identity(
            self.agent_id().into(),
            self.chat_id().into(),
            self.updated(),
        );
        chat.name = self.name().map(str::to_owned);
        chat.pinned = self.pinned();
        chat.title = self.transcript().title.clone();
        chat.pending_history = self.pending_history();
        chat.times = SavedChat::bounded_times(self.entries().len(), &self.transcript().times);
        input.metadata(&mut chat);
        chat
    }
    pub(super) fn snapshot(&self, input: &Inputs<'_>) -> Option<Arc<SharedChat>> {
        if !self.pinned()
            && self.name().is_none()
            && self.entries().is_empty()
            && !input.has_content()
        {
            return None;
        }
        let mut metadata = self.metadata(input);
        metadata.entries = SavedChat::bounded_entries(self.entries());
        Some(Arc::new(SharedChat {
            metadata,
            queue: input.queue(),
        }))
    }
    pub(super) fn needs_candidate(&self) -> bool {
        !self.clean() && !self.persistence.has_current_candidate()
    }
    pub(super) fn current_snapshot(&self) -> Option<Arc<SharedChat>> {
        self.persistence.current_snapshot()
    }
    pub(super) fn existing_offer(&self) -> Option<(Arc<SharedChat>, u64)> {
        self.persistence.capture_pending()
    }
    pub(super) fn propose_candidate(
        &mut self,
        input: &Inputs<'_>,
    ) -> Option<(Arc<SharedChat>, u64)> {
        if self.clean() {
            return None;
        }
        if self.persistence.has_current_candidate() {
            return self.persistence.capture_pending();
        }
        let snapshot = self.snapshot(input).or_else(|| {
            self.history_present().then(|| {
                Arc::new(SharedChat {
                    metadata: self.metadata(input),
                    queue: Vec::new(),
                })
            })
        })?;
        self.persistence.remember_inputs(input);
        self.persistence.prepare(snapshot);
        self.persistence.capture_pending()
    }
    pub(super) fn capture_patch(
        &mut self,
        snapshot: Arc<SharedChat>,
    ) -> Option<(Arc<SharedChat>, u64)> {
        if self.clean() {
            return None;
        }
        self.persistence.prepare(snapshot);
        self.persistence.capture_pending()
    }
    pub(super) fn mark_normal_offer(&mut self, revision: u64) -> bool {
        self.persistence.mark_normal_offer(revision)
    }
    pub(super) fn acknowledge_admission(&mut self, revision: u64) {
        self.persistence.acknowledge_admission(revision);
        if revision == self.desired_revision() {
            self.draft_debounce = None;
        }
    }
    pub(super) fn published(&mut self, revision: u64) -> bool {
        self.persistence.published(revision)
    }
    pub(super) fn failed(
        &mut self,
        revision: u64,
        error: crate::runtime::AgentPersistenceError,
    ) -> bool {
        self.persistence.failed(revision, error)
    }
    pub(super) fn has_debounce(&self) -> bool {
        self.draft_debounce.is_some()
    }
    pub(super) fn replace_debounce(&mut self, task: Task<()>) {
        self.draft_debounce = Some(task);
    }
    pub(super) fn cancel_debounce(&mut self) {
        self.draft_debounce = None;
    }
    pub(super) fn evict_clean(&mut self) -> bool {
        if !self.clean() || self.draft_changed() {
            return false;
        }
        self.transcript = Transcript::default();
        self.budget = BudgetState::default();
        self.changed_rows.clear();
        true
    }
    pub(super) fn release_disabled(&mut self) -> bool {
        if !self.clean() && !self.persistence.writer_owns_current() {
            return false;
        }
        self.transcript = Transcript::default();
        self.budget = BudgetState::default();
        self.changed_rows.clear();
        true
    }
    pub(super) fn restart_snapshot(&self) -> DocumentRestart {
        DocumentRestart {
            identity: self.identity.clone(),
            metadata: self.metadata.clone(),
            transcript: self.transcript.clone(),
            persistence: self.persistence.restart_snapshot(),
        }
    }
    pub(super) fn hydrate_restart(&mut self, data: DocumentRestart) {
        self.identity = data.identity;
        self.metadata = data.metadata;
        self.transcript = data.transcript;
        self.persistence = data.persistence;
        self.budget = BudgetState::default();
        self.draft_debounce = None;
    }
}

#[cfg(test)]
mod tests;
