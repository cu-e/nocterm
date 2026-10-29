//! Descriptive publication facts, deliberately without execution allowances or permits.
use super::{CoordinatorError, PromptRequest, RunDestination};
use crate::{
    authority::ExecutionAuthority,
    engine::EnginePrompt,
    identity::{Fingerprint, IntentId, StepId},
    inference::ModelSelection,
    journal::BackgroundRecordOutcome,
    run::CommitStamp,
    tool::BackgroundCommand,
};

#[derive(Clone, Debug)]
pub struct ObservedPrompt {
    prompt: EnginePrompt,
    selection: ModelSelection,
    destination: Option<RunDestination>,
}
impl ObservedPrompt {
    pub fn new(
        prompt: EnginePrompt,
        selection: ModelSelection,
        destination: Option<RunDestination>,
    ) -> Self {
        Self {
            prompt,
            selection,
            destination,
        }
    }
    pub fn from_request(request: &PromptRequest) -> Self {
        Self::new(
            request.prompt().clone(),
            request.selection().clone(),
            Some(request.destination().clone()),
        )
    }
    pub fn prompt(&self) -> &EnginePrompt {
        &self.prompt
    }
    pub fn selection(&self) -> &ModelSelection {
        &self.selection
    }
    pub fn destination(&self) -> Option<&RunDestination> {
        self.destination.as_ref()
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ObservedJobWindow {
    created_at_ms: u64,
    deadline_ms: u64,
}
impl ObservedJobWindow {
    pub fn new(created_at_ms: u64, deadline_ms: u64) -> Result<Self, CoordinatorError> {
        if created_at_ms == 0 || deadline_ms <= created_at_ms {
            return Err(CoordinatorError::InvalidCommand);
        }
        Ok(Self {
            created_at_ms,
            deadline_ms,
        })
    }
    pub fn created_at_ms(self) -> u64 {
        self.created_at_ms
    }
    pub fn deadline_ms(self) -> u64 {
        self.deadline_ms
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObservedBackground {
    intent: IntentId,
    execution: ExecutionAuthority,
    terminal: String,
    command: String,
    window: Option<ObservedJobWindow>,
}
impl ObservedBackground {
    pub fn new(
        intent: IntentId,
        execution: ExecutionAuthority,
        terminal: String,
        command: String,
        window: Option<ObservedJobWindow>,
    ) -> Result<Self, CoordinatorError> {
        if [&terminal, &command]
            .iter()
            .any(|value| value.is_empty() || value.len() > 128 || value.contains('\0'))
        {
            return Err(CoordinatorError::InvalidCommand);
        }
        Ok(Self {
            intent,
            execution,
            terminal,
            command,
            window,
        })
    }
    pub fn from_command(original: &BackgroundCommand) -> Self {
        Self {
            intent: original.dispatch().intent().id(),
            execution: original.dispatch().intent().authority().clone(),
            terminal: original.terminal().to_owned(),
            command: original.command().to_owned(),
            window: None,
        }
    }
    pub fn intent(&self) -> IntentId {
        self.intent
    }
    pub fn execution(&self) -> &ExecutionAuthority {
        &self.execution
    }
    pub fn terminal(&self) -> &str {
        &self.terminal
    }
    pub fn command(&self) -> &str {
        &self.command
    }
    pub fn window(&self) -> Option<ObservedJobWindow> {
        self.window
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ObservedEffect {
    Observed,
    Unknown,
    Background(Box<ObservedBackground>),
}
#[derive(Clone, Debug)]
pub struct BackgroundObservation {
    job: ObservedBackground,
    step: StepId,
    arguments: Fingerprint,
    mutates: bool,
    dispatch: CommitStamp,
    outcome: BackgroundRecordOutcome,
}
impl BackgroundObservation {
    pub fn new(
        job: ObservedBackground,
        step: StepId,
        arguments: Fingerprint,
        mutates: bool,
        dispatch: CommitStamp,
        outcome: BackgroundRecordOutcome,
    ) -> Result<Self, CoordinatorError> {
        if dispatch.run() != job.execution().run() || dispatch.revision().get() == 0 {
            return Err(CoordinatorError::InvalidCommand);
        }
        Ok(Self {
            job,
            step,
            arguments,
            mutates,
            dispatch,
            outcome,
        })
    }
    pub fn job(&self) -> &ObservedBackground {
        &self.job
    }
    pub fn step(&self) -> StepId {
        self.step
    }
    pub fn arguments(&self) -> Fingerprint {
        self.arguments
    }
    pub fn mutates(&self) -> bool {
        self.mutates
    }
    pub fn dispatch(&self) -> &CommitStamp {
        &self.dispatch
    }
    pub fn outcome(&self) -> BackgroundRecordOutcome {
        self.outcome
    }
}

impl From<&crate::tool::ToolEffectStatus> for ObservedEffect {
    fn from(value: &crate::tool::ToolEffectStatus) -> Self {
        match value {
            crate::tool::ToolEffectStatus::Observed => Self::Observed,
            crate::tool::ToolEffectStatus::Unknown => Self::Unknown,
            crate::tool::ToolEffectStatus::Background(command) => {
                Self::Background(Box::new(ObservedBackground::from_command(command)))
            }
        }
    }
}
