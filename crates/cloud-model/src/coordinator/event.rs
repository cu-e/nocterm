use super::{ApprovalAnswer, BackgroundObservation, ObservedEffect, ObservedPrompt};
use crate::{
    authority::RunBinding,
    engine::EngineEvent,
    identity::{CommandId, IntentId, Timestamp},
    journal::CommandAcceptance,
    payload::BoundedJson,
    run::ToolIntent,
    tool::{ToolEffectStatus, ToolExecutionResult},
};
use nocterm_ai::{TerminalCall, tool_display::ToolDisplay};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunStatus {
    Queued,
    Running,
    WaitingApproval,
    NeedsGrant,
    NeedsUnlock,
    NeedsProviderAccess,
    OutcomeUnknown,
    Interrupted,
    Stopped,
    Completed,
}

/// Trusted host/backend observation for the existing renderer. Presentation is
/// never interpreted as execution authority, including restored display args.
#[derive(Clone)]
pub struct ToolObservation {
    intent: IntentId,
    output: BoundedJson,
    display: ToolDisplay,
    effect: ObservedEffect,
    failed: bool,
}
impl ToolObservation {
    pub fn from_result(result: ToolExecutionResult) -> Self {
        Self {
            intent: result.intent(),
            output: result.output().clone(),
            display: result.display().clone(),
            effect: result.effect_status().into(),
            failed: false,
        }
    }
    pub fn failed(
        intent: IntentId,
        output: BoundedJson,
        display: ToolDisplay,
        effect: ToolEffectStatus,
    ) -> Self {
        Self {
            intent,
            output,
            display,
            effect: (&effect).into(),
            failed: true,
        }
    }
    pub fn new(
        intent: IntentId,
        output: BoundedJson,
        display: ToolDisplay,
        effect: ObservedEffect,
        failed: bool,
    ) -> Self {
        Self {
            intent,
            output,
            display,
            effect,
            failed,
        }
    }
    pub fn intent(&self) -> IntentId {
        self.intent
    }
    pub fn output(&self) -> &BoundedJson {
        &self.output
    }
    pub fn display(&self) -> &ToolDisplay {
        &self.display
    }
    pub fn effect(&self) -> &ObservedEffect {
        &self.effect
    }
    pub fn is_failed(&self) -> bool {
        self.failed
    }
}

/// Semantic public observations; the sole contract encoder owns signed wire
/// bytes. Native Postcard frames never cross this consumer seam. User content
/// enters PromptAccepted atomically with admission so another account device
/// never needs the R-only original command envelope to reconstruct its history.
#[derive(Clone)]
pub enum CoordinatorEvent {
    CommandAccepted {
        command: CommandAcceptance,
        run: Option<RunBinding>,
    },
    PromptAccepted {
        command: CommandAcceptance,
        run: RunBinding,
        request: ObservedPrompt,
    },
    Status {
        run: RunBinding,
        status: RunStatus,
    },
    /// Usage here is a complete optional cumulative reported run snapshot.
    /// Unknown fields replace prior known values; it is never a call delta or
    /// the conservative inference charge. Text retains chunk semantics.
    Progress {
        run: RunBinding,
        event: EngineEvent,
    },
    ToolIntent {
        intent: ToolIntent,
        call: TerminalCall,
        display: ToolDisplay,
    },
    Approval {
        intent: ToolIntent,
        call: TerminalCall,
        display: ToolDisplay,
        expires: Timestamp,
    },
    ApprovalResolved {
        command: CommandId,
        answer: ApprovalAnswer,
    },
    ToolResult {
        run: RunBinding,
        observation: ToolObservation,
    },
    BackgroundLifecycle(Box<BackgroundObservation>),
    Deleted {
        command: CommandId,
    },
}
