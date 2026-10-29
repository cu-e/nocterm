use super::identity::{Authority, Run, fp};
use nocterm_cloud_model::{identity::*, journal::*, payload::OpaqueBytes, run::*};
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Stamp {
    run: Run,
    writer: u64,
    revision: u64,
}
impl From<&CommitStamp> for Stamp {
    fn from(value: &CommitStamp) -> Self {
        Self {
            run: value.run().into(),
            writer: value.writer().get(),
            revision: value.revision().get(),
        }
    }
}
impl Stamp {
    pub(crate) fn restore_ref(&self) -> Result<CommitStamp, JournalError> {
        self.clone().restore()
    }
    pub(crate) fn restore(self) -> Result<CommitStamp, JournalError> {
        Ok(CommitStamp::new(
            self.run.restore()?,
            WriterEpoch::new(self.writer)?,
            Revision::from_u64(self.revision),
        ))
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Intent {
    id: [u8; 16],
    step: [u8; 16],
    authority: Authority,
    digest: [u8; 32],
    mutates: bool,
}
impl From<&ToolIntent> for Intent {
    fn from(value: &ToolIntent) -> Self {
        Self {
            id: *value.id().as_bytes(),
            step: *value.step().as_bytes(),
            authority: value.authority().into(),
            digest: *value.arguments().as_bytes(),
            mutates: value.mutates(),
        }
    }
}
impl Intent {
    pub(crate) fn restore_ref(&self) -> Result<ToolIntent, JournalError> {
        self.clone().restore()
    }
    pub(crate) fn restore(self) -> Result<ToolIntent, JournalError> {
        Ok(ToolIntent::from_journal(
            IntentId::from_bytes(self.id),
            StepId::from_bytes(self.step),
            self.authority.restore()?,
            fp(self.digest),
            self.mutates,
        ))
    }
}

#[derive(Serialize, Deserialize)]
enum Context {
    Chat,
    Run(Run),
    Transition(Stamp),
}
#[derive(Serialize, Deserialize)]
enum Kind {
    Budget([u8; 16], [u8; 16]),
    Command([u8; 16], u8),
    Progress([u8; 16]),
    Intent(Box<Intent>),
    Dispatched([u8; 16]),
    Outcome([u8; 16], Option<[u8; 32]>),
    Phase(u8),
    Reconciled([u8; 16], [u8; 16]),
    Authority([u8; 16]),
    Queue([u8; 16]),
    Tombstone([u8; 16]),
    Background([u8; 16], String, String, Option<Option<u32>>),
    // Appended native Postcard discriminants; existing stored variants retain
    // their indices. These are not public signed command/event wire tags.
    ModelAction([u8; 16]),
    Approval([u8; 16], bool),
    Declined([u8; 16]),
    RunStatus,
    PublicSnapshot,
}
#[derive(Serialize, Deserialize)]
pub(crate) struct Record {
    context: Context,
    kind: Kind,
    payload: Vec<u8>,
}
impl From<&JournalRecord> for Record {
    fn from(value: &JournalRecord) -> Self {
        let context = match value.context() {
            RecordContext::Chat => Context::Chat,
            RecordContext::Run(run) => Context::Run(run.into()),
            RecordContext::Transition(stamp) => Context::Transition(stamp.into()),
        };
        let kind = match value.kind() {
            JournalRecordKind::PublicSnapshot => Kind::PublicSnapshot,
            JournalRecordKind::RunStatus => Kind::RunStatus,
            JournalRecordKind::ModelAction { step } => Kind::ModelAction(*step.as_bytes()),
            JournalRecordKind::Approval { intent, open } => {
                Kind::Approval(*intent.as_bytes(), *open)
            }
            JournalRecordKind::Declined { intent } => Kind::Declined(*intent.as_bytes()),
            JournalRecordKind::BudgetReserved { step, attempt } => {
                Kind::Budget(*step.as_bytes(), *attempt.as_bytes())
            }
            JournalRecordKind::CommandAccepted { id, kind } => {
                Kind::Command(*id.as_bytes(), command_tag(*kind))
            }
            JournalRecordKind::Progress { step } => Kind::Progress(*step.as_bytes()),
            JournalRecordKind::Intent(intent) => Kind::Intent(Box::new(intent.as_ref().into())),
            JournalRecordKind::Dispatched { intent } => Kind::Dispatched(*intent.as_bytes()),
            JournalRecordKind::Outcome { intent, outcome } => Kind::Outcome(
                *intent.as_bytes(),
                match outcome {
                    EffectOutcome::Known(value) => Some(*value.as_bytes()),
                    EffectOutcome::Unknown => None,
                },
            ),
            JournalRecordKind::Phase(phase) => Kind::Phase(phase_tag(*phase)),
            JournalRecordKind::Reconciled { intent, command } => {
                Kind::Reconciled(*intent.as_bytes(), *command.as_bytes())
            }
            JournalRecordKind::AuthorityChanged { grant } => Kind::Authority(*grant.as_bytes()),
            JournalRecordKind::QueueChanged { command } => Kind::Queue(*command.as_bytes()),
            JournalRecordKind::Tombstone { command } => Kind::Tombstone(*command.as_bytes()),
            JournalRecordKind::BackgroundCommand {
                intent,
                terminal,
                command,
                outcome,
            } => Kind::Background(
                *intent.as_bytes(),
                terminal.clone(),
                command.clone(),
                match outcome {
                    BackgroundRecordOutcome::Retained => None,
                    BackgroundRecordOutcome::Unknown => Some(None),
                    BackgroundRecordOutcome::ObservedExit(exit) => Some(Some(*exit)),
                },
            ),
        };
        Self {
            context,
            kind,
            payload: value.payload().as_bytes().to_vec(),
        }
    }
}
impl Record {
    pub(crate) fn restore(self) -> Result<JournalRecord, JournalError> {
        let context = match self.context {
            Context::Chat => RecordContext::Chat,
            Context::Run(run) => RecordContext::Run(run.restore()?),
            Context::Transition(stamp) => RecordContext::Transition(stamp.restore()?),
        };
        JournalRecord::new(
            context,
            self.kind.restore()?,
            OpaqueBytes::new(self.payload)?,
        )
    }
}
impl Kind {
    fn restore(self) -> Result<JournalRecordKind, JournalError> {
        Ok(match self {
            Self::PublicSnapshot => JournalRecordKind::PublicSnapshot,
            Self::RunStatus => JournalRecordKind::RunStatus,
            Self::ModelAction(step) => JournalRecordKind::ModelAction {
                step: StepId::from_bytes(step),
            },
            Self::Approval(intent, open) => JournalRecordKind::Approval {
                intent: IntentId::from_bytes(intent),
                open,
            },
            Self::Declined(intent) => JournalRecordKind::Declined {
                intent: IntentId::from_bytes(intent),
            },
            Self::Budget(step, attempt) => JournalRecordKind::BudgetReserved {
                step: StepId::from_bytes(step),
                attempt: AttemptId::from_bytes(attempt),
            },
            Self::Command(id, kind) => JournalRecordKind::CommandAccepted {
                id: CommandId::from_bytes(id),
                kind: command_kind(kind)?,
            },
            Self::Progress(step) => JournalRecordKind::Progress {
                step: StepId::from_bytes(step),
            },
            Self::Intent(intent) => JournalRecordKind::Intent(Box::new(intent.restore()?)),
            Self::Dispatched(intent) => JournalRecordKind::Dispatched {
                intent: IntentId::from_bytes(intent),
            },
            Self::Outcome(intent, outcome) => JournalRecordKind::Outcome {
                intent: IntentId::from_bytes(intent),
                outcome: outcome.map_or(EffectOutcome::Unknown, |value| {
                    EffectOutcome::Known(fp(value))
                }),
            },
            Self::Phase(phase) => JournalRecordKind::Phase(phase_kind(phase)?),
            Self::Reconciled(intent, command) => JournalRecordKind::Reconciled {
                intent: IntentId::from_bytes(intent),
                command: CommandId::from_bytes(command),
            },
            Self::Authority(grant) => JournalRecordKind::AuthorityChanged {
                grant: GrantId::from_bytes(grant),
            },
            Self::Queue(command) => JournalRecordKind::QueueChanged {
                command: CommandId::from_bytes(command),
            },
            Self::Tombstone(command) => JournalRecordKind::Tombstone {
                command: CommandId::from_bytes(command),
            },
            Self::Background(intent, terminal, command, outcome) => {
                JournalRecordKind::BackgroundCommand {
                    intent: IntentId::from_bytes(intent),
                    terminal,
                    command,
                    outcome: match outcome {
                        None => BackgroundRecordOutcome::Retained,
                        Some(None) => BackgroundRecordOutcome::Unknown,
                        Some(Some(exit)) => BackgroundRecordOutcome::ObservedExit(exit),
                    },
                }
            }
        })
    }
}

pub(crate) fn command_tag(kind: CommandKind) -> u8 {
    match kind {
        CommandKind::SubmitPrompt => 1,
        CommandKind::Stop => 2,
        CommandKind::ResolveApproval => 3,
        CommandKind::IssueGrant => 4,
        CommandKind::RevokeGrant => 5,
        CommandKind::AuthenticationConsent => 6,
        CommandKind::DeleteChat => 7,
        CommandKind::RevokeIssuer => 8,
    }
}
pub(crate) fn command_kind(tag: u8) -> Result<CommandKind, JournalError> {
    match tag {
        1 => Ok(CommandKind::SubmitPrompt),
        2 => Ok(CommandKind::Stop),
        3 => Ok(CommandKind::ResolveApproval),
        4 => Ok(CommandKind::IssueGrant),
        5 => Ok(CommandKind::RevokeGrant),
        6 => Ok(CommandKind::AuthenticationConsent),
        7 => Ok(CommandKind::DeleteChat),
        8 => Ok(CommandKind::RevokeIssuer),
        _ => Err(JournalError::Corrupt),
    }
}
fn phase_tag(phase: RunPhase) -> u8 {
    match phase {
        RunPhase::Running => 1,
        RunPhase::OutcomeUnknown => 2,
        RunPhase::Interrupted => 3,
        RunPhase::Completed => 4,
    }
}
fn phase_kind(tag: u8) -> Result<RunPhase, JournalError> {
    match tag {
        1 => Ok(RunPhase::Running),
        2 => Ok(RunPhase::OutcomeUnknown),
        3 => Ok(RunPhase::Interrupted),
        4 => Ok(RunPhase::Completed),
        _ => Err(JournalError::Corrupt),
    }
}
