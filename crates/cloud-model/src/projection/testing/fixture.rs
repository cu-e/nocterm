//! Deterministic semantic pages shared by domain and desktop behavior tests.
use super::super::{ProjectionBatch, ProjectionBinding, ProjectionCursor, ProjectionEvent};
use crate::{
    authority::{
        AuthenticationBinding, ExecutionAuthority, RunBinding, SessionReceipt, TargetBinding,
    },
    coordinator::{CoordinatorEvent, PromptAllowance, PromptRequest, RunDestination},
    engine::{EnginePrompt, ModelBudget},
    grant::GrantWindow,
    identity::*,
    inference::ModelSelection,
    journal::{CommandAcceptance, CommandKind},
};
use nocterm_ai::model::content::Content;

pub fn run_binding(id: u8) -> RunBinding {
    RunBinding::new(
        AccountScope::new(
            HubIdentity::new(NamespaceId::new("hub").unwrap(), fingerprint(1)),
            AccountIdentity::new(NamespaceId::new("account").unwrap(), fingerprint(2)),
        ),
        RunnerId::from_bytes([3; 16]),
        RunnerKeyEpoch::new(1).unwrap(),
        CloudChatId::from_bytes([4; 16]),
        RunId::from_bytes([id; 16]),
        RunEpoch::new(1).unwrap(),
    )
}
fn fingerprint(byte: u8) -> Fingerprint {
    Fingerprint::from_bytes([byte; 32])
}
fn target() -> TargetBinding {
    TargetBinding::new(
        fingerprint(5),
        fingerprint(6),
        NamespaceId::new("user").unwrap(),
        fingerprint(7),
    )
}
fn authentication() -> AuthenticationBinding {
    AuthenticationBinding::new(fingerprint(8), fingerprint(9))
}
pub fn execution_authority(run: RunBinding) -> ExecutionAuthority {
    ExecutionAuthority::new(
        run,
        SessionReceipt::new(
            RemoteSessionId::from_bytes([10; 16]),
            SessionEpoch::new(1).unwrap(),
            target(),
            authentication(),
        ),
        GrantId::from_bytes([11; 16]),
        GrantEpoch::new(1).unwrap(),
    )
}
pub fn prompt_event(run: &RunBinding, text: &str, id: CommandId) -> CoordinatorEvent {
    let command = CommandAcceptance::new(
        id,
        CommandKind::SubmitPrompt,
        fingerprint(12),
        crate::authority::CommandIssuer::new(
            NamespaceId::new("device").unwrap(),
            fingerprint(13),
            fingerprint(14),
        ),
        ChatControlRevision::default(),
    );
    let request = PromptRequest::new(
        EnginePrompt::new(vec![Content::text(text)]).unwrap(),
        ModelSelection::new(
            NamespaceId::new("provider").unwrap(),
            NamespaceId::new("model").unwrap(),
            NamespaceId::new("pricing").unwrap(),
        ),
        RunDestination::new(target(), authentication(), None).unwrap(),
        PromptAllowance::new(
            ModelBudget::new(8, 1024 * 1024, 1024 * 1024, 4096).unwrap(),
            100,
            GrantWindow::new(Timestamp::from_millis(1), Timestamp::from_millis(10000)).unwrap(),
        )
        .unwrap(),
    );
    CoordinatorEvent::PromptAccepted {
        command,
        run: run.clone(),
        request,
    }
}

pub struct ProjectionFixture {
    binding: ProjectionBinding,
    cursor: ProjectionCursor,
    event_id: u128,
    control: ChatControlRevision,
}
impl Default for ProjectionFixture {
    fn default() -> Self {
        Self::new()
    }
}
impl ProjectionFixture {
    pub fn new() -> Self {
        Self {
            binding: ProjectionBinding::new(run_binding(1).stream(), WriterEpoch::new(1).unwrap()),
            cursor: ProjectionCursor::origin(),
            event_id: 1,
            control: ChatControlRevision::from_u64(1),
        }
    }
    pub fn binding(&self) -> &ProjectionBinding {
        &self.binding
    }
    pub fn cursor(&self) -> ProjectionCursor {
        self.cursor
    }
    pub fn set_control(&mut self, control: ChatControlRevision) {
        self.control = control;
    }
    pub fn page(&mut self, events: Vec<CoordinatorEvent>) -> ProjectionBatch {
        let mut envelopes = Vec::new();
        for event in events {
            let mut head = [0; 32];
            head[..16].copy_from_slice(&self.event_id.to_le_bytes());
            let envelope = ProjectionEvent {
                binding: self.binding.clone(),
                id: EventId::from_bytes(self.event_id.to_le_bytes()),
                before: self.cursor,
                head: Fingerprint::from_bytes(head),
                control: self.control,
                time: Timestamp::from_millis(self.event_id.min(u64::MAX as u128) as u64 * 1000),
                event,
            };
            self.cursor = envelope.after().unwrap();
            self.event_id += 1;
            envelopes.push(envelope);
        }
        ProjectionBatch::new(self.binding.clone(), envelopes, self.cursor).unwrap()
    }
}
