use super::{BackgroundRecordOutcome, JournalError, RECORD_METADATA_CHARGE};
use crate::{
    authority::{CommandIssuer, RunBinding},
    identity::{AttemptId, ChatControlRevision, CommandId, Fingerprint, GrantId, IntentId, StepId},
    payload::OpaqueBytes,
    run::{CommitStamp, EffectOutcome, RunPhase, ToolIntent},
};

/// Semantic operations only. Numeric signed/wire tags remain contract-owned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommandKind {
    SubmitPrompt,
    Stop,
    ResolveApproval,
    IssueGrant,
    RevokeGrant,
    AuthenticationConsent,
    DeleteChat,
    RevokeIssuer,
    BeginSession,
}

impl CommandKind {
    /// Control and replies use finite reserved capacity independently from
    /// ordinary prompt admission. This classification grants no authority.
    pub fn high_priority(self) -> bool {
        !matches!(self, Self::SubmitPrompt)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandAcceptance {
    id: CommandId,
    kind: CommandKind,
    digest: Fingerprint,
    issuer: CommandIssuer,
    expected_control: ChatControlRevision,
}
impl CommandAcceptance {
    pub fn new(
        id: CommandId,
        kind: CommandKind,
        digest: Fingerprint,
        issuer: CommandIssuer,
        expected_control: ChatControlRevision,
    ) -> Self {
        Self {
            id,
            kind,
            digest,
            issuer,
            expected_control,
        }
    }
    pub fn id(&self) -> CommandId {
        self.id
    }
    pub fn kind(&self) -> CommandKind {
        self.kind
    }
    pub fn digest(&self) -> Fingerprint {
        self.digest
    }
    pub fn issuer(&self) -> &CommandIssuer {
        &self.issuer
    }
    pub fn expected_control(&self) -> ChatControlRevision {
        self.expected_control
    }
}

#[derive(Clone, Debug)]
pub enum RecordContext {
    Chat,
    Run(RunBinding),
    Transition(CommitStamp),
}
impl RecordContext {
    pub fn run(&self) -> Option<&RunBinding> {
        match self {
            Self::Chat => None,
            Self::Run(run) => Some(run),
            Self::Transition(stamp) => Some(stamp.run()),
        }
    }
    pub fn stamp(&self) -> Option<&CommitStamp> {
        match self {
            Self::Transition(stamp) => Some(stamp),
            _ => None,
        }
    }
}

#[derive(Clone, Debug)]
pub enum JournalRecordKind {
    /// Signed public baseline publication; changes neither user control nor effects.
    PublicSnapshot,
    /// Host observation without a model call or a run FSM transition.
    RunStatus,
    /// A completed model action or its host-correlated results; native payload
    /// is typed/versioned independently from the public signed wire.
    ModelAction {
        step: StepId,
    },
    Approval {
        intent: IntentId,
        open: bool,
    },
    Declined {
        intent: IntentId,
    },
    BudgetReserved {
        step: StepId,
        attempt: AttemptId,
    },
    CommandAccepted {
        id: CommandId,
        kind: CommandKind,
    },
    Progress {
        step: StepId,
    },
    Intent(Box<ToolIntent>),
    Dispatched {
        intent: IntentId,
    },
    Outcome {
        intent: IntentId,
        outcome: EffectOutcome,
    },
    Phase(RunPhase),
    Reconciled {
        intent: IntentId,
        command: CommandId,
    },
    AuthorityChanged {
        grant: GrantId,
    },
    QueueChanged {
        command: CommandId,
    },
    Tombstone {
        command: CommandId,
    },
    BackgroundCommand {
        intent: IntentId,
        terminal: String,
        command: String,
        outcome: BackgroundRecordOutcome,
    },
}

/// Native record body has an adapter-owned versioned codec; it is not another
/// serializer for signed hub bytes. The native store encrypts/protects it on disk.
#[derive(Clone, Debug)]
pub struct JournalRecord {
    context: RecordContext,
    kind: JournalRecordKind,
    payload: OpaqueBytes,
}
impl JournalRecord {
    pub fn new(
        context: RecordContext,
        kind: JournalRecordKind,
        payload: OpaqueBytes,
    ) -> Result<Self, JournalError> {
        let valid = match (&context, &kind) {
            (
                RecordContext::Chat | RecordContext::Run(_),
                JournalRecordKind::CommandAccepted { .. },
            ) => true,
            (
                RecordContext::Run(_),
                JournalRecordKind::Progress { .. }
                | JournalRecordKind::BudgetReserved { .. }
                | JournalRecordKind::AuthorityChanged { .. }
                | JournalRecordKind::BackgroundCommand { .. }
                | JournalRecordKind::ModelAction { .. }
                | JournalRecordKind::RunStatus
                | JournalRecordKind::Approval { .. }
                | JournalRecordKind::Reconciled { .. },
            ) => true,
            (RecordContext::Transition(stamp), JournalRecordKind::Intent(intent)) => {
                stamp.run() == intent.authority().run()
            }
            (
                RecordContext::Transition(_),
                JournalRecordKind::Dispatched { .. }
                | JournalRecordKind::Declined { .. }
                | JournalRecordKind::Outcome { .. }
                | JournalRecordKind::Phase(_)
                | JournalRecordKind::Reconciled { .. },
            ) => true,
            (
                RecordContext::Chat,
                JournalRecordKind::Tombstone { .. }
                | JournalRecordKind::QueueChanged { .. }
                | JournalRecordKind::PublicSnapshot,
            ) => true,
            _ => false,
        };
        if !valid
            || matches!(&kind, JournalRecordKind::BackgroundCommand { terminal, command, .. } if [terminal,command].into_iter().any(|id| id.is_empty() || id.len()>128 || id.contains('\0')))
        {
            return Err(JournalError::InvalidBatch);
        }
        Ok(Self {
            context,
            kind,
            payload,
        })
    }
    pub fn context(&self) -> &RecordContext {
        &self.context
    }
    pub fn kind(&self) -> &JournalRecordKind {
        &self.kind
    }
    pub fn payload(&self) -> &OpaqueBytes {
        &self.payload
    }
    pub fn budget_bytes(&self) -> usize {
        self.payload.len().saturating_add(RECORD_METADATA_CHARGE)
    }
    pub fn changes_control(&self) -> bool {
        matches!(
            self.kind,
            JournalRecordKind::CommandAccepted { .. }
                | JournalRecordKind::Phase(_)
                | JournalRecordKind::Declined { .. }
                | JournalRecordKind::Approval { .. }
                | JournalRecordKind::Outcome {
                    outcome: EffectOutcome::Unknown,
                    ..
                }
                | JournalRecordKind::Reconciled { .. }
                | JournalRecordKind::AuthorityChanged { .. }
                | JournalRecordKind::QueueChanged { .. }
                | JournalRecordKind::Tombstone { .. }
        )
    }
    pub(crate) fn requires_outbox(&self) -> bool {
        !matches!(self.kind, JournalRecordKind::BudgetReserved { .. })
    }
}
