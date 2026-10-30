//! Versioned semantic recovery frames, scoped to their enclosing journal context.
use super::format::{self, Command, Inference, Selection, Task, ToolGrant};
use crate::codec::{identity::Run, record::Intent};
use nocterm_ai::{TerminalCall, tool_display::ToolDisplay};
use nocterm_cloud_model::{
    coordinator::*,
    engine::*,
    grant::*,
    identity::*,
    inference::*,
    journal::JournalError,
    payload::{BoundedJson, OpaqueBytes},
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Call {
    name: String,
    arguments: serde_json::Value,
}
impl TryFrom<&TerminalCall> for Call {
    type Error = CoordinatorError;
    fn try_from(call: &TerminalCall) -> Result<Self, Self::Error> {
        Ok(Self {
            name: call.name().into(),
            arguments: call.arguments().map_err(EngineError::from)?,
        })
    }
}
impl Call {
    pub(super) fn restore(self) -> Result<TerminalCall, CoordinatorError> {
        let call = nocterm_ai::tool_display::parse(&self.name, self.arguments)
            .ok_or(JournalError::Corrupt)?;
        call.validate().map_err(EngineError::from)?;
        Ok(call)
    }
}
#[derive(Clone, Copy, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Usage(
    Option<u64>,
    Option<u64>,
    Option<u64>,
    Option<u64>,
    Option<u64>,
    Option<u64>,
);
impl From<UsageObservation> for Usage {
    fn from(value: UsageObservation) -> Self {
        Self(
            value.input_tokens,
            value.output_tokens,
            value.total_tokens,
            value.cached_input_tokens,
            value.thought_tokens,
            value.cost_micros,
        )
    }
}
impl Usage {
    fn restore(self) -> UsageObservation {
        UsageObservation {
            input_tokens: self.0,
            output_tokens: self.1,
            total_tokens: self.2,
            cached_input_tokens: self.3,
            thought_tokens: self.4,
            cost_micros: self.5,
        }
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) enum Progress {
    Text(bool, String),
    Usage(Usage),
}
impl From<&EngineEvent> for Progress {
    fn from(value: &EngineEvent) -> Self {
        match value {
            EngineEvent::Text(text) => {
                Self::Text(text.channel() == TextChannel::Thought, text.text().into())
            }
            EngineEvent::Usage(usage) => Self::Usage((*usage).into()),
        }
    }
}
impl Progress {
    pub(super) fn restore(self) -> Result<EngineEvent, CoordinatorError> {
        Ok(match self {
            Self::Text(thought, text) => EngineEvent::Text(EngineText::new(
                if thought {
                    TextChannel::Thought
                } else {
                    TextChannel::Assistant
                },
                text,
            )?),
            Self::Usage(usage) => EngineEvent::Usage(usage.restore()),
        })
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Reservation {
    attempt: [u8; 16],
    grant: Inference,
    run: Run,
    step: [u8; 16],
    bytes: usize,
    tokens: u64,
    selection: Selection,
    spend: u64,
    output: usize,
    admitted: u64,
    observation: Option<(Usage, usize)>,
    finalized: bool,
}
impl From<&InferenceCallSnapshot> for Reservation {
    fn from(value: &InferenceCallSnapshot) -> Self {
        let reserve = value.reservation();
        Self {
            attempt: *reserve.attempt().as_bytes(),
            grant: reserve.grant().into(),
            run: reserve.request().run().into(),
            step: *reserve.request().step().as_bytes(),
            bytes: reserve.request().request_bytes(),
            tokens: reserve.request().output_token_limit(),
            selection: reserve.quote().selection().into(),
            spend: reserve.quote().spend_micros(),
            output: reserve.output_bytes(),
            admitted: reserve.admitted().as_millis(),
            observation: value
                .observation()
                .map(|observation| (observation.usage().into(), observation.output_bytes())),
            finalized: value.finalized(),
        }
    }
}
impl Reservation {
    fn restore(self) -> Result<InferenceCallSnapshot, CoordinatorError> {
        Ok(InferenceCallSnapshot::new(
            InferenceReservation::from_journal(
                AttemptId::from_bytes(self.attempt),
                self.grant.restore()?,
                ModelCallRequest::new(
                    self.run.restore()?,
                    StepId::from_bytes(self.step),
                    self.bytes,
                    self.tokens,
                )?,
                ModelQuote::new(self.selection.restore()?, self.spend)?,
                self.output,
                Timestamp::from_millis(self.admitted),
            )?,
            self.observation
                .map(|(usage, bytes)| ModelCallObservation::new(usage.restore(), bytes))
                .transpose()?,
            self.finalized,
        )?)
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Ledger {
    task: Task,
    calls: Vec<Reservation>,
    observed: u64,
    clock_failed: bool,
}
impl From<&InferenceLedger> for Ledger {
    fn from(value: &InferenceLedger) -> Self {
        let snapshot = value.snapshot();
        Self {
            task: snapshot.task().into(),
            calls: snapshot.calls().iter().map(Reservation::from).collect(),
            observed: snapshot.observed().as_millis(),
            clock_failed: snapshot.clock_failed(),
        }
    }
}
impl Ledger {
    pub(super) fn restore(self) -> Result<InferenceLedger, CoordinatorError> {
        Ok(InferenceLedger::from_snapshot(InferenceSnapshot::new(
            self.task.restore()?,
            self.calls
                .into_iter()
                .map(Reservation::restore)
                .collect::<Result<_, _>>()?,
            Timestamp::from_millis(self.observed),
            self.clock_failed,
        )?))
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Grant {
    policy: ToolGrant,
    remaining: u32,
    observed: u64,
    revoked: Option<bool>,
}
impl From<&GrantState> for Grant {
    fn from(value: &GrantState) -> Self {
        let snapshot = value.snapshot();
        Self {
            policy: snapshot.policy().into(),
            remaining: snapshot.remaining(),
            observed: snapshot.observed().as_millis(),
            revoked: snapshot
                .revocation()
                .map(|reason| reason == GrantRevocation::ClockRollback),
        }
    }
}
impl Grant {
    pub(super) fn restore(self) -> Result<GrantState, CoordinatorError> {
        Ok(GrantState::from_snapshot(GrantSnapshot::new(
            self.policy.restore()?,
            self.remaining,
            Timestamp::from_millis(self.observed),
            self.revoked.map(|clock| {
                if clock {
                    GrantRevocation::ClockRollback
                } else {
                    GrantRevocation::Explicit
                }
            }),
        )?))
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PlannedCall {
    intent: [u8; 16],
    index: u32,
    provider: Option<String>,
    call: Call,
}
impl PlannedCall {
    pub(super) fn new(intent: IntentId, call: &EngineToolCall) -> Result<Self, CoordinatorError> {
        Ok(Self {
            intent: *intent.as_bytes(),
            index: call.slot().index(),
            provider: call.provider_id().map(|id| id.as_str().into()),
            call: Call::try_from(call.call())?,
        })
    }
    pub(super) fn intent(&self) -> IntentId {
        IntentId::from_bytes(self.intent)
    }
    pub(super) fn call(&self) -> Result<TerminalCall, CoordinatorError> {
        self.call.clone().restore()
    }
    pub(super) fn restore(&self, step: StepId) -> Result<EngineToolCall, CoordinatorError> {
        Ok(EngineToolCall::new(
            ToolSlot::new(step, self.index)?,
            self.provider
                .clone()
                .map(crate::codec::identity::namespace)
                .transpose()?,
            self.call()?,
        )?)
    }
}

/// Enclosing JournalRecordKind/Run/Intent is independently checked on replay.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) enum Frame {
    Session(super::session_fact::SessionFact),
    Authentication {
        attempt: [u8; 16],
        nonce: [u8; 32],
    },
    Accepted {
        command: Command,
        allocated: Option<Run>,
    },
    Status(u8),
    Progress(Progress),
    Budget(Ledger),
    Action {
        ledger: Ledger,
        text: String,
        calls: Vec<PlannedCall>,
        complete: bool,
    },
    Intent {
        intent: Intent,
        call: Call,
        grant: Grant,
    },
    Approval {
        intent: [u8; 16],
        expires: u64,
    },
    Dispatch {
        intent: [u8; 16],
        grant: Grant,
    },
    Result {
        intent: [u8; 16],
        output: serde_json::Value,
        display: ToolDisplay,
        failed: bool,
        unknown: bool,
    },
    Reconciled {
        original: Intent,
        declared: [u8; 32],
    },
}
impl Frame {
    pub(super) fn bytes(&self) -> Result<OpaqueBytes, CoordinatorError> {
        format::encode(self)
    }
    pub(super) fn read(bytes: &OpaqueBytes) -> Result<Self, CoordinatorError> {
        format::decode(bytes)
    }
    pub(super) fn accepted(
        command: &ReceivedCommand,
        run: Option<&nocterm_cloud_model::authority::RunBinding>,
    ) -> Self {
        Self::Accepted {
            command: command.into(),
            allocated: run.map(Run::from),
        }
    }
    pub(super) fn result(
        intent: IntentId,
        output: &BoundedJson,
        display: ToolDisplay,
        failed: bool,
        unknown: bool,
    ) -> Self {
        Self::Result {
            intent: *intent.as_bytes(),
            output: output.value().clone(),
            display,
            failed,
            unknown,
        }
    }
}
