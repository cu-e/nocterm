//! Immutable causal indices built from the real, verified semantic history.
use nocterm_cloud_model::journal::JournalError;
use nocterm_hub_model::cloud::{
    RunCoordinate,
    events::{
        BackgroundJob, BackgroundOutcome, CloudEvent, CloudEventPayload, CloudRunStatus,
        ObservedEffect, ToolIntentPayload,
    },
};
use std::mem::size_of;

struct RunFacts {
    run: RunCoordinate,
    prompt: usize,
    status: Option<usize>,
    activation: Option<usize>,
    usage: Option<usize>,
    phase: CloudRunStatus,
}
struct ToolFacts {
    run: usize,
    id: [u8; 16],
    intent: usize,
    approval: Option<usize>,
    resolution: Option<usize>,
    result: Option<usize>,
    background: Option<usize>,
    lifecycle: Option<usize>,
    outcome: Option<BackgroundOutcome>,
}
struct EventFacts {
    run: Option<usize>,
    tool: Option<usize>,
    bytes: usize,
    required: bool,
}
pub(super) struct CausalFacts {
    runs: Vec<RunFacts>,
    tools: Vec<ToolFacts>,
    events: Vec<EventFacts>,
    latest_prompt: Option<usize>,
    selected: Option<usize>,
}
impl CausalFacts {
    pub(super) fn storage_bytes(count: usize) -> Option<usize> {
        count.checked_mul(size_of::<RunFacts>() + size_of::<ToolFacts>() + size_of::<EventFacts>())
    }
    pub(super) fn empty(count: usize) -> Self {
        Self {
            runs: Vec::with_capacity(count),
            tools: Vec::with_capacity(count),
            events: Vec::with_capacity(count),
            latest_prompt: None,
            selected: None,
        }
    }
    pub(super) fn index(&mut self, events: &[CloudEvent<'_>]) -> Result<(), JournalError> {
        for (index, event) in events.iter().enumerate() {
            let (run, tool) = self.observe(index, events)?;
            self.events.push(EventFacts {
                run,
                tool,
                bytes: event
                    .canonical_len()
                    .map_err(super::super::super::encoding_error)?,
                required: matches!(&event.payload, CloudEventPayload::Deleted { .. })
                    || matches!(&event.payload, CloudEventPayload::ToolResult { observation, .. }
                        if observation.effect == ObservedEffect::Unknown)
                    || matches!(&event.payload, CloudEventPayload::BackgroundLifecycle(fact)
                        if fact.outcome == BackgroundOutcome::Unknown),
            });
        }
        for (key, run) in self.runs.iter().enumerate() {
            if Some(key) == self.selected
                || Some(run.prompt) == self.latest_prompt
                || !terminal(run.phase)
            {
                self.events
                    .get_mut(run.prompt)
                    .ok_or(JournalError::Corrupt)?
                    .required = true;
            }
        }
        for tool in &self.tools {
            let pending_approval = tool.approval.is_some_and(|approval| {
                tool.resolution
                    .is_none_or(|resolution| resolution < approval)
            });
            let active_background = tool.background.is_some()
                && !matches!(tool.outcome, Some(BackgroundOutcome::ObservedExit(_)));
            if tool.result.is_none() || pending_approval || active_background {
                self.events
                    .get_mut(tool.intent)
                    .ok_or(JournalError::Corrupt)?
                    .required = true;
            }
        }
        Ok(())
    }
    pub(super) fn count(&self) -> usize {
        self.events.len()
    }
    pub(super) fn required(&self, index: usize) -> Result<bool, JournalError> {
        Ok(self.event(index)?.required)
    }
    pub(super) fn bytes(&self, index: usize) -> Result<usize, JournalError> {
        Ok(self.event(index)?.bytes)
    }
    pub(super) fn links(
        &self,
        index: usize,
    ) -> Result<(Option<usize>, Option<usize>), JournalError> {
        let event = self.event(index)?;
        Ok((event.run, event.tool))
    }
    pub(super) fn run_roots(&self, index: usize) -> Result<[Option<usize>; 4], JournalError> {
        let run = self.runs.get(index).ok_or(JournalError::Corrupt)?;
        Ok([Some(run.prompt), run.activation, run.status, run.usage])
    }
    pub(super) fn tool_roots(&self, index: usize) -> Result<[Option<usize>; 6], JournalError> {
        let tool = self.tools.get(index).ok_or(JournalError::Corrupt)?;
        Ok([
            Some(tool.intent),
            tool.approval,
            tool.resolution,
            tool.result,
            tool.background,
            tool.lifecycle,
        ])
    }
    fn event(&self, index: usize) -> Result<&EventFacts, JournalError> {
        self.events.get(index).ok_or(JournalError::Corrupt)
    }
    fn run(&self, run: RunCoordinate) -> Result<usize, JournalError> {
        self.runs
            .iter()
            .position(|facts| facts.run == run)
            .ok_or(JournalError::Corrupt)
    }
    fn tool(&self, run: usize, id: [u8; 16]) -> Result<usize, JournalError> {
        self.tools
            .iter()
            .position(|facts| facts.run == run && facts.id == id)
            .ok_or(JournalError::Corrupt)
    }
    fn observe(
        &mut self,
        index: usize,
        events: &[CloudEvent<'_>],
    ) -> Result<(Option<usize>, Option<usize>), JournalError> {
        let event = events.get(index).ok_or(JournalError::Corrupt)?;
        let run = match &event.payload {
            CloudEventPayload::PromptAccepted { run, .. } => {
                if self.run(*run).is_ok() {
                    return Err(JournalError::Corrupt);
                }
                let key = self.runs.len();
                if self
                    .selected
                    .and_then(|key| self.runs.get(key))
                    .is_none_or(|run| terminal(run.phase))
                {
                    self.selected = Some(key);
                }
                self.runs.push(RunFacts {
                    run: *run,
                    prompt: index,
                    status: None,
                    activation: None,
                    usage: None,
                    phase: CloudRunStatus::Queued,
                });
                self.latest_prompt = Some(index);
                return Ok((Some(key), None));
            }
            CloudEventPayload::Deleted { .. } => return Ok((None, None)),
            CloudEventPayload::CommandAccepted { run, .. } => {
                return Ok((run.map(|run| self.run(run)).transpose()?, None));
            }
            CloudEventPayload::Status { run, .. }
            | CloudEventPayload::Text { run, .. }
            | CloudEventPayload::Usage { run, .. }
            | CloudEventPayload::ToolResult { run, .. } => *run,
            CloudEventPayload::ToolIntent(intent) | CloudEventPayload::Approval { intent, .. } => {
                intent.execution.run
            }
            CloudEventPayload::ApprovalResolved { answer, .. } => answer.execution.run,
            CloudEventPayload::BackgroundLifecycle(fact) => fact.job.execution.run,
            // Managed-session payloads are not projected by this desktop build yet.
            CloudEventPayload::AuthenticationRequested(_) | CloudEventPayload::SessionState(_) => {
                return Err(JournalError::Corrupt);
            }
        };
        let run = self.run(run)?;
        let tool = self.observe_run(index, run, events)?;
        Ok((Some(run), tool))
    }
    fn observe_run(
        &mut self,
        index: usize,
        run: usize,
        events: &[CloudEvent<'_>],
    ) -> Result<Option<usize>, JournalError> {
        let event = events.get(index).ok_or(JournalError::Corrupt)?;
        let id = match &event.payload {
            CloudEventPayload::Status { status, .. } => {
                if *status == CloudRunStatus::Running && self.selected != Some(run) {
                    if self
                        .selected
                        .and_then(|key| self.runs.get(key))
                        .is_some_and(|run| !terminal(run.phase))
                    {
                        return Err(JournalError::Corrupt);
                    }
                    self.selected = Some(run);
                    self.runs
                        .get_mut(run)
                        .ok_or(JournalError::Corrupt)?
                        .activation = Some(index);
                }
                let facts = self.runs.get_mut(run).ok_or(JournalError::Corrupt)?;
                if matches!(
                    facts.phase,
                    CloudRunStatus::Completed | CloudRunStatus::Stopped
                ) && facts.phase != *status
                {
                    return Err(JournalError::Corrupt);
                }
                facts.phase = *status;
                facts.status = Some(index);
                return Ok(None);
            }
            CloudEventPayload::Usage { .. } => {
                self.runs.get_mut(run).ok_or(JournalError::Corrupt)?.usage = Some(index);
                return Ok(None);
            }
            CloudEventPayload::ToolIntent(intent) => {
                if self.tool(run, intent.id).is_ok() {
                    return Err(JournalError::Corrupt);
                }
                let key = self.tools.len();
                self.tools.push(ToolFacts {
                    run,
                    id: intent.id,
                    intent: index,
                    approval: None,
                    resolution: None,
                    result: None,
                    background: None,
                    lifecycle: None,
                    outcome: None,
                });
                return Ok(Some(key));
            }
            CloudEventPayload::Approval { intent, .. } => intent.id,
            CloudEventPayload::ApprovalResolved { answer, .. } => answer.intent,
            CloudEventPayload::ToolResult { observation, .. } => observation.intent,
            CloudEventPayload::BackgroundLifecycle(fact) => fact.job.intent,
            _ => return Ok(None),
        };
        let key = self.tool(run, id)?;
        self.tools
            .get_mut(key)
            .ok_or(JournalError::Corrupt)?
            .observe(index, events)?;
        Ok(Some(key))
    }
}
impl ToolFacts {
    fn observe(&mut self, index: usize, events: &[CloudEvent<'_>]) -> Result<(), JournalError> {
        match &events.get(index).ok_or(JournalError::Corrupt)?.payload {
            CloudEventPayload::Approval { intent, .. } => {
                if !same_intent(original_intent(events, self.intent)?, intent) {
                    return Err(JournalError::Corrupt);
                }
                self.approval = Some(index);
            }
            CloudEventPayload::ApprovalResolved { answer, .. } => {
                let original = original_intent(events, self.intent)?;
                if original.execution != answer.execution
                    || original.arguments_digest != answer.arguments_digest
                {
                    return Err(JournalError::Corrupt);
                }
                self.resolution = Some(index);
            }
            CloudEventPayload::ToolResult { observation, .. } => {
                if self.result.is_some() {
                    return Err(JournalError::Corrupt);
                }
                if let ObservedEffect::Background(job) = &observation.effect {
                    self.observe_job(index, job, events)?;
                }
                self.result = Some(index);
            }
            CloudEventPayload::BackgroundLifecycle(fact) => {
                self.observe_job(index, &fact.job, events)?;
                let original = original_intent(events, self.intent)?;
                if original.step != fact.step
                    || original.arguments_digest != fact.arguments_digest
                    || original.mutates != fact.mutates
                {
                    return Err(JournalError::Corrupt);
                }
                if matches!(self.outcome, Some(BackgroundOutcome::ObservedExit(_)))
                    && self.outcome != Some(fact.outcome)
                {
                    return Err(JournalError::Corrupt);
                }
                if let Some(previous) = self.lifecycle {
                    let CloudEventPayload::BackgroundLifecycle(previous) =
                        &events.get(previous).ok_or(JournalError::Corrupt)?.payload
                    else {
                        return Err(JournalError::Corrupt);
                    };
                    if previous.dispatch_writer_epoch != fact.dispatch_writer_epoch
                        || previous.dispatch_revision != fact.dispatch_revision
                    {
                        return Err(JournalError::Corrupt);
                    }
                }
                self.lifecycle = Some(index);
                self.outcome = Some(fact.outcome);
            }
            _ => return Err(JournalError::Corrupt),
        }
        Ok(())
    }
    fn observe_job(
        &mut self,
        index: usize,
        job: &BackgroundJob<'_>,
        events: &[CloudEvent<'_>],
    ) -> Result<(), JournalError> {
        let original = original_intent(events, self.intent)?;
        if original.id != job.intent || original.execution != job.execution {
            return Err(JournalError::Corrupt);
        }
        if let Some(previous) = self.background {
            if original_job(events, previous)? != job {
                return Err(JournalError::Corrupt);
            }
        } else {
            self.background = Some(index);
        }
        Ok(())
    }
}

fn original_intent<'a, 'b>(
    events: &'a [CloudEvent<'b>],
    index: usize,
) -> Result<&'a ToolIntentPayload<'b>, JournalError> {
    match &events.get(index).ok_or(JournalError::Corrupt)?.payload {
        CloudEventPayload::ToolIntent(intent) => Ok(intent),
        _ => Err(JournalError::Corrupt),
    }
}
fn original_job<'a, 'b>(
    events: &'a [CloudEvent<'b>],
    index: usize,
) -> Result<&'a BackgroundJob<'b>, JournalError> {
    match &events.get(index).ok_or(JournalError::Corrupt)?.payload {
        CloudEventPayload::ToolResult { observation, .. } => match &observation.effect {
            ObservedEffect::Background(job) => Ok(job),
            _ => Err(JournalError::Corrupt),
        },
        CloudEventPayload::BackgroundLifecycle(fact) => Ok(&fact.job),
        _ => Err(JournalError::Corrupt),
    }
}
fn same_intent(first: &ToolIntentPayload<'_>, next: &ToolIntentPayload<'_>) -> bool {
    first.id == next.id
        && first.step == next.step
        && first.execution == next.execution
        && first.arguments_digest == next.arguments_digest
        && first.mutates == next.mutates
        && first.arguments == next.arguments
}
fn terminal(status: CloudRunStatus) -> bool {
    matches!(
        status,
        CloudRunStatus::Completed | CloudRunStatus::Stopped | CloudRunStatus::Interrupted
    )
}
