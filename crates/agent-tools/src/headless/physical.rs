//! Exactly one native session and bounded VT state, independent of run views.
use super::{
    HeadlessError, HeadlessSessionStatus,
    initial::InitialLifecycle,
    projection::{LiveOwner, TerminalProjection},
    terminal_failure::TerminalFailure,
};
use nocterm_cloud_model::{
    authority::{ExecutionAuthority, SessionReceipt},
    identity::Timestamp,
    tool::{ScopedToolRequest, ToolAuthority},
};
use nocterm_session::{Event, Prompt, Session};
use std::sync::Arc;

pub(super) struct PhysicalTerminal {
    receipt: SessionReceipt,
    session: Session,
    projection: TerminalProjection,
    status: HeadlessSessionStatus,
    prompt: Option<Prompt>,
    publication: Publication,
    retirement: async_channel::Sender<()>,
    failure: TerminalFailure,
    lifecycle: Option<InitialLifecycle>,
    initial_owner: Option<Arc<()>>,
}
impl PhysicalTerminal {
    pub(super) fn new(
        receipt: SessionReceipt,
        session: Session,
        options: super::HeadlessTerminalOptions,
        retirement: async_channel::Sender<()>,
        publication: PhysicalPublication,
    ) -> Self {
        let (publication, lifecycle) = match publication {
            PhysicalPublication::Published => (Publication::Published, None),
            PhysicalPublication::Pending => (Publication::Pending, None),
            PhysicalPublication::Initial(lifecycle) => {
                (Publication::InitialPending, Some(lifecycle))
            }
        };
        Self {
            receipt,
            session,
            projection: TerminalProjection::new(options.size, options.charset),
            status: HeadlessSessionStatus::Connecting,
            prompt: None,
            publication,
            retirement,
            failure: Default::default(),
            lifecycle,
            initial_owner: None,
        }
    }
    /// Transfer this connected candidate to its physical supervisor. Publication
    /// and abandoned-candidate retirement serialize on this same physical lock.
    pub(super) fn publish(&mut self) -> Result<(), HeadlessError> {
        match self.publication {
            Publication::Retired | Publication::RetiredPending => Err(HeadlessError::StaleSession),
            Publication::Published => {
                // A real new run view accepts this same physical generation.
                // The older initial-publication waiter no longer owns rollback.
                self.initial_owner = None;
                Ok(())
            }
            Publication::Pending if self.status == HeadlessSessionStatus::Connected => {
                self.publication = Publication::Published;
                Ok(())
            }
            Publication::Pending | Publication::InitialPending => Err(HeadlessError::NotConnected),
        }
    }
    pub(super) fn publish_initial(&mut self) -> Result<(), HeadlessError> {
        if self.publication != Publication::InitialPending {
            return Err(HeadlessError::StaleSession);
        }
        if self.status != HeadlessSessionStatus::Connected {
            return Err(HeadlessError::NotConnected);
        }
        self.publication = Publication::Published;
        Ok(())
    }
    pub(super) fn publish_initial_owned(&mut self, owner: Arc<()>) -> Result<(), HeadlessError> {
        self.publish_initial()?;
        self.initial_owner = Some(owner);
        Ok(())
    }
    pub(super) fn accept_initial(&mut self, owner: &Arc<()>) {
        if self
            .initial_owner
            .as_ref()
            .is_some_and(|held| Arc::ptr_eq(held, owner))
        {
            self.initial_owner = None;
        }
    }
    pub(super) fn retire_initial(&mut self, owner: &Arc<()>) -> bool {
        if !self
            .initial_owner
            .as_ref()
            .is_some_and(|held| Arc::ptr_eq(held, owner))
        {
            return false;
        }
        self.initial_owner = None;
        self.retire();
        true
    }
    pub(super) fn retire_pending(&mut self) -> bool {
        if !matches!(
            self.publication,
            Publication::Pending | Publication::InitialPending | Publication::RetiredPending
        ) {
            return false;
        }
        self.retire();
        true
    }
    pub(super) fn receipt(&self) -> &SessionReceipt {
        &self.receipt
    }
    pub(super) fn observed_loss(&self, observation: &super::initial::SessionObservation) -> bool {
        matches!(
            observation.event(),
            super::initial::SessionLifecycleEvent::Lost { .. }
        ) && self
            .lifecycle
            .as_ref()
            .is_some_and(|lifecycle| lifecycle.matches(observation))
    }
    pub(super) fn info(&self, terminal_id: String) -> super::HeadlessSessionInfo {
        super::HeadlessSessionInfo::new(
            terminal_id,
            self.receipt.clone(),
            if self.publication == Publication::InitialPending
                && self.status == HeadlessSessionStatus::Connected
            {
                HeadlessSessionStatus::Connecting
            } else {
                self.status
            },
            self.session.exec().is_some(),
            self.session.fs().is_some(),
            self.failure.error(),
        )
    }
    pub(super) fn take_prompt(
        &mut self,
        receipt: &SessionReceipt,
    ) -> Result<Option<Prompt>, HeadlessError> {
        if self.is_retired() || &self.receipt != receipt {
            return Err(HeadlessError::StaleSession);
        }
        Ok(self.prompt.take())
    }

    pub(super) fn check(&self, authority: &ExecutionAuthority) -> Result<(), HeadlessError> {
        if self.is_retired() || &self.receipt != authority.session() {
            return Err(HeadlessError::StaleSession);
        }
        if self.publication == Publication::InitialPending {
            return Err(HeadlessError::NotConnected);
        }
        if let Some(error) = self.failure.error() {
            return Err(HeadlessError::Session(error));
        }
        match self.status {
            HeadlessSessionStatus::Connected => Ok(()),
            HeadlessSessionStatus::NeedsUnlock => Err(HeadlessError::NeedsUnlock),
            _ => Err(HeadlessError::NotConnected),
        }
    }
    pub(super) fn event(&mut self, event: Event, now: Timestamp) {
        if self.is_retired() {
            return;
        }
        let replies = match event {
            Event::Connecting(_) => {
                self.status = HeadlessSessionStatus::Connecting;
                Vec::new()
            }
            Event::Connected => {
                self.status = HeadlessSessionStatus::Connected;
                if let Some(lifecycle) = &mut self.lifecycle {
                    lifecycle.connected();
                }
                Vec::new()
            }
            Event::Output(bytes) => self.projection.output(&bytes, false, now.as_millis()),
            Event::Restore(bytes) => {
                self.projection.restore(&bytes, now.as_millis());
                Vec::new()
            }
            Event::Prompt(prompt) => {
                self.status = HeadlessSessionStatus::NeedsUnlock;
                self.prompt = Some(prompt);
                Vec::new()
            }
            Event::Closed(reason) => {
                self.status = self.failure.observe(reason);
                if let Some(lifecycle) = &mut self.lifecycle {
                    lifecycle.lost(self.status, self.failure.error());
                }
                self.prompt = None;
                self.projection.output(&[], true, now.as_millis())
            }
        };
        if !self.is_retired() {
            for reply in replies {
                if !self.session.input(reply) {
                    self.retire();
                    break;
                }
            }
        }
    }
    pub(super) fn retire(&mut self) {
        if let Some(lifecycle) = &mut self.lifecycle {
            lifecycle.lost(HeadlessSessionStatus::Closed, self.failure.error());
        }
        self.retirement.close();
        self.publication = if matches!(
            self.publication,
            Publication::Pending | Publication::InitialPending | Publication::RetiredPending
        ) {
            Publication::RetiredPending
        } else {
            Publication::Retired
        };
        self.status = HeadlessSessionStatus::Closed;
        self.prompt = None;
        self.session.close();
    }
    fn is_retired(&self) -> bool {
        matches!(
            self.publication,
            Publication::Retired | Publication::RetiredPending
        )
    }
    pub(super) fn read(&self, lines: usize, since: Option<u64>) -> nocterm_vt::TextTail {
        self.projection.text(lines, since)
    }
    pub(super) fn send(
        &mut self,
        request: &ScopedToolRequest,
        authority: &dyn ToolAuthority,
        text: &str,
        command: bool,
    ) -> Result<Option<LiveOwner>, nocterm_cloud_model::tool::ToolExecutionError> {
        self.check(request.permit().intent().authority())?;
        let bytes = self.projection.prepare_input(text, command)?;
        authority.check(request)?;
        let owner = command.then(|| LiveOwner::new(request));
        if !self.session.input(bytes.clone()) {
            return Err(HeadlessError::InputRefused.into());
        }
        self.projection.input_admitted(&bytes, owner.clone());
        Ok(owner)
    }
    pub(super) fn executor(&self) -> Result<Arc<dyn nocterm_session::HostExec>, HeadlessError> {
        self.session
            .exec()
            .ok_or(HeadlessError::ExecutionUnsupported)
    }
    pub(super) fn end_live(&mut self, owner: &LiveOwner) {
        self.projection.end_live(owner);
    }
    pub(super) fn prompt_returned(&self, owner: &LiveOwner) -> bool {
        self.projection.prompt_returned(owner)
    }

    pub(super) fn generation(&self) -> u64 {
        self.projection.generation()
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Publication {
    Pending,
    InitialPending,
    RetiredPending,
    Published,
    Retired,
}

pub(super) enum PhysicalPublication {
    Published,
    Pending,
    Initial(InitialLifecycle),
}
