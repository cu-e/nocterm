use super::{
    MAX_PROJECTION_ENTRIES, MAX_REPLAY_EVENTS, ProjectionBatch, ProjectionBinding,
    ProjectionCursor, ProjectionError, RecordKind, runs::ProjectionRuns, tools::ProjectionTools,
};
use crate::{
    authority::RunBinding,
    coordinator::CoordinatorEvent,
    engine::{EngineEvent, TextChannel},
    identity::{ChatControlRevision, CommandId, EventId},
};
use nocterm_ai::{
    conversation::ConversationPhase,
    history::{MAX_RESIDENT_BYTES, retained_value_bytes},
    model::{content::Content, session::SessionUpdate},
    thread::ThreadState,
};
use std::collections::VecDeque;

#[derive(Clone)]
struct SeenEvent {
    id: EventId,
    after: ProjectionCursor,
}

/// The domain owns accepted transcript state, never wire verification or I/O.
/// Preparing a page creates exactly one candidate; failures preserve all old state.
#[derive(Clone)]
pub struct ConversationProjection {
    binding: ProjectionBinding,
    cursor: ProjectionCursor,
    known_head: ProjectionCursor,
    control: ChatControlRevision,
    runs: ProjectionRuns,
    deleted: bool,
    transcript: ThreadState,
    seen: VecDeque<SeenEvent>,
    tools: ProjectionTools,
    bytes: usize,
    updated: u64,
    usage: super::usage::ProjectionUsage,
    approvals: super::approvals::ProjectionApprovals,
    row_bytes: Vec<usize>,
}
impl ConversationProjection {
    pub fn new(binding: ProjectionBinding) -> Self {
        Self {
            binding,
            cursor: ProjectionCursor::origin(),
            known_head: ProjectionCursor::origin(),
            control: ChatControlRevision::default(),
            runs: ProjectionRuns::default(),
            deleted: false,
            transcript: ThreadState::default(),
            seen: VecDeque::new(),
            tools: ProjectionTools::default(),
            bytes: 1024,
            updated: 0,
            usage: super::usage::ProjectionUsage::default(),
            approvals: super::approvals::ProjectionApprovals::default(),
            row_bytes: Vec::new(),
        }
    }
    pub fn binding(&self) -> &ProjectionBinding {
        &self.binding
    }
    pub fn cursor(&self) -> ProjectionCursor {
        self.cursor
    }
    pub fn known_head(&self) -> ProjectionCursor {
        self.known_head
    }
    pub fn control(&self) -> ChatControlRevision {
        self.control
    }
    pub fn run(&self) -> Option<&RunBinding> {
        self.runs.selected()
    }
    pub fn phase(&self) -> ConversationPhase {
        if self.deleted {
            ConversationPhase::Deleted
        } else {
            self.runs.phase()
        }
    }
    pub fn transcript(&self) -> &ThreadState {
        &self.transcript
    }
    pub fn resident_bytes(&self) -> usize {
        self.bytes
    }
    /// Partial display facts for the selected run; absent values remain unknown.
    pub fn usage(&self) -> crate::engine::UsageObservation {
        self.usage.observation()
    }
    /// Descriptive late facts never rewrite a terminal ToolResult or select a run.
    pub fn background_observations(
        &self,
    ) -> impl Iterator<Item = &crate::coordinator::BackgroundObservation> {
        self.tools.observations()
    }
    pub fn approvals(&self) -> impl Iterator<Item = &super::ProjectedApproval> {
        self.approvals.iter()
    }
    pub fn approval(&self, key: super::ApprovalKey) -> Option<&super::ProjectedApproval> {
        self.approvals.get(key)
    }
    pub fn updated_at(&self) -> u64 {
        self.updated
    }

    pub fn apply(&mut self, batch: &ProjectionBatch) -> Result<Vec<CommandId>, ProjectionError> {
        let (next, accepted) = self.advanced(batch)?;
        *self = next;
        Ok(accepted)
    }
    pub fn advanced(
        &self,
        batch: &ProjectionBatch,
    ) -> Result<(Self, Vec<CommandId>), ProjectionError> {
        if batch.binding() != &self.binding {
            return Err(ProjectionError::WrongSource);
        }
        Self::require_head(batch.known_head(), self.known_head)?;
        let mut next = self.clone();
        let mut accepted = Vec::new();
        let mut dirty = Vec::new();
        for envelope in batch.events() {
            if envelope.binding != self.binding {
                return Err(ProjectionError::WrongSource);
            }
            let after = envelope.after()?;
            if after.sequence() <= next.cursor.sequence() {
                if !next
                    .seen
                    .iter()
                    .any(|seen| seen.id == envelope.id && seen.after == after)
                {
                    return Err(ProjectionError::Discontinuous);
                }
                continue;
            }
            if envelope.before != next.cursor || next.seen.iter().any(|seen| seen.id == envelope.id)
            {
                return Err(ProjectionError::Discontinuous);
            }
            if envelope.control < next.control {
                return Err(ProjectionError::Rollback);
            }
            if next.deleted && !matches!(&envelope.event, CoordinatorEvent::BackgroundLifecycle(_))
            {
                return Err(ProjectionError::InvalidRecord(RecordKind::Event));
            }
            if matches!(&envelope.event, CoordinatorEvent::BackgroundLifecycle(_))
                && envelope.control != next.control
            {
                return Err(ProjectionError::InvalidRecord(RecordKind::Event));
            }
            let now = envelope.time.as_millis() / 1000;
            next.reduce(&envelope.event, now, &mut accepted, &mut dirty)?;
            next.approvals.observe(&envelope.event, envelope.control)?;
            next.control = envelope.control;
            next.cursor = after;
            next.updated = next.updated.max(now);
            next.seen.push_back(SeenEvent {
                id: envelope.id,
                after,
            });
            if next.seen.len() > MAX_REPLAY_EVENTS {
                next.seen.pop_front();
            }
        }
        Self::require_head(batch.known_head(), next.cursor)?;
        dirty.sort_unstable();
        dirty.dedup();
        next.measure(&dirty)?;
        next.known_head = batch.known_head();
        Ok((next, accepted))
    }
    fn require_head(
        advertised: ProjectionCursor,
        minimum: ProjectionCursor,
    ) -> Result<(), ProjectionError> {
        if (advertised.sequence().get() == 0) != advertised.head().is_none()
            || advertised.sequence() < minimum.sequence()
            || (advertised.sequence() == minimum.sequence() && advertised.head() != minimum.head())
        {
            return Err(ProjectionError::Rollback);
        }
        Ok(())
    }
    fn measure(&mut self, dirty: &[usize]) -> Result<(), ProjectionError> {
        if self.transcript.entries.len() > MAX_PROJECTION_ENTRIES {
            return Err(ProjectionError::Capacity);
        }
        let previous = self.row_bytes.len();
        self.row_bytes.resize(self.transcript.entries.len(), 0);
        for row in (previous..self.transcript.entries.len())
            .chain(dirty.iter().copied().filter(|row| *row < previous))
        {
            self.row_bytes[row] = retained_value_bytes(&self.transcript.entries[row])
                .map_err(|_| ProjectionError::Capacity)?;
        }
        self.bytes = 1024usize
            .saturating_add(self.row_bytes.iter().sum::<usize>())
            .saturating_add(self.row_bytes.capacity().saturating_mul(size_of::<usize>()))
            .saturating_add(self.transcript.times.capacity().saturating_mul(8))
            .saturating_add(self.runs.bytes())
            .saturating_add(self.tools.bytes())
            .saturating_add(self.approvals.bytes()?)
            .saturating_add(self.seen.capacity().saturating_mul(96));
        if self.bytes > MAX_RESIDENT_BYTES {
            return Err(ProjectionError::Capacity);
        }
        Ok(())
    }
    fn require_stream(&self, run: &RunBinding) -> Result<(), ProjectionError> {
        (run.stream() == *self.binding.stream())
            .then_some(())
            .ok_or(ProjectionError::WrongSource)
    }
    fn require_current(&self, run: &RunBinding) -> Result<(), ProjectionError> {
        self.require_stream(run)?;
        self.runs.require_current(run)
    }
    fn reduce(
        &mut self,
        event: &CoordinatorEvent,
        now: u64,
        accepted: &mut Vec<CommandId>,
        dirty: &mut Vec<usize>,
    ) -> Result<(), ProjectionError> {
        let previous_run = self.runs.selected().map(super::runs::key);
        match event {
            CoordinatorEvent::AuthenticationRequested { .. }
            | CoordinatorEvent::SessionState(_) => {
                return Err(ProjectionError::Unsupported);
            }
            CoordinatorEvent::CommandAccepted { command, run } => {
                if let Some(run) = run {
                    self.require_stream(run)?;
                    self.runs.require_known(run)?;
                }
                accepted.push(command.id());
            }
            CoordinatorEvent::PromptAccepted {
                command,
                run,
                request,
            } => {
                self.require_stream(run)?;
                self.runs.admit(run)?;
                self.transcript
                    .push_user(request.prompt().content().to_vec(), now);
                accepted.push(command.id());
            }
            CoordinatorEvent::Status { run, status } => {
                self.require_stream(run)?;
                self.runs.update(run, super::runs::phase(*status))?;
            }
            CoordinatorEvent::Progress { run, event } => {
                self.require_current(run)?;
                self.progress(event, now, dirty);
            }
            CoordinatorEvent::ToolIntent {
                intent,
                call,
                display,
            } => {
                self.require_current(intent.authority().run())?;
                self.tools
                    .push(intent, call, display, &mut self.transcript, now)?;
            }
            CoordinatorEvent::ToolResult { run, observation } => {
                self.require_stream(run)?;
                self.runs.require_known(run)?;
                dirty.push(self.tools.finish(run, observation, &mut self.transcript)?);
            }
            CoordinatorEvent::Approval { intent, .. } => {
                let run = intent.authority().run();
                self.require_current(run)?;
                if !self.tools.contains(intent) {
                    return Err(ProjectionError::InvalidRecord(RecordKind::Event));
                }
                self.runs.update(run, ConversationPhase::WaitingApproval)?;
            }
            CoordinatorEvent::ApprovalResolved { command, answer } => {
                self.require_current(answer.authority().run())?;
                accepted.push(*command);
            }
            CoordinatorEvent::BackgroundLifecycle(value) => {
                self.require_stream(value.job().execution().run())?;
                self.runs.require_known(value.job().execution().run())?;
                self.tools.background(value)?;
            }
            CoordinatorEvent::Deleted { command } => {
                self.deleted = true;
                accepted.push(*command);
            }
        }
        if previous_run != self.runs.selected().map(super::runs::key) {
            self.usage = super::usage::ProjectionUsage::default();
            self.transcript.tokens = None;
        }
        Ok(())
    }
    fn progress(&mut self, event: &EngineEvent, now: u64, dirty: &mut Vec<usize>) {
        match event {
            EngineEvent::Text(text) => {
                if let Some(row) = self.transcript.entries.len().checked_sub(1) {
                    dirty.push(row);
                }
                let content = Content::text(text.text());
                self.transcript.apply(
                    match text.channel() {
                        TextChannel::Assistant => SessionUpdate::AgentMessageChunk(content),
                        TextChannel::Thought => SessionUpdate::AgentThoughtChunk(content),
                    },
                    now,
                );
            }
            EngineEvent::Usage(usage) => {
                self.transcript.tokens = self.usage.observe(*usage);
            }
        }
    }
}
