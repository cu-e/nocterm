//! Immutable signed approval observations. A card never grants execution.
use super::{
    MAX_PENDING_COMMANDS, ProjectionError,
    runs::{RunKey, key},
};
use crate::{
    coordinator::{ApprovalAnswer, ApprovalChoice, CoordinatorEvent, RunStatus},
    identity::*,
    run::ToolIntent,
};
use nocterm_ai::{TerminalCall, tool_display::ToolDisplay};
use std::collections::BTreeMap;

/// Actual semantic identity also exists in sparse snapshot facts. No fabricated
/// event ID or Local ApprovalToken is needed to capture a Cloud card.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ApprovalKey {
    run: RunId,
    epoch: RunEpoch,
    intent: IntentId,
    step: StepId,
    args: Fingerprint,
    expires: Timestamp,
    control: ChatControlRevision,
    session: RemoteSessionId,
    session_epoch: SessionEpoch,
    grant: GrantId,
    grant_epoch: GrantEpoch,
}
impl ApprovalKey {
    pub fn intent(self) -> IntentId {
        self.intent
    }
    pub fn expires(self) -> Timestamp {
        self.expires
    }
    pub fn matches_request(self, request: &super::ApprovalRequest) -> bool {
        let answer = request.answer();
        let authority = answer.authority();
        let run = authority.run();
        let session = authority.session();
        self.intent == answer.intent()
            && self.run == run.run()
            && self.epoch == run.epoch()
            && self.args == answer.arguments()
            && self.expires == request.expires()
            && self.session == session.id()
            && self.session_epoch == session.epoch()
            && self.grant == authority.grant()
            && self.grant_epoch == authority.grant_epoch()
    }
}
#[derive(Clone)]
pub struct ProjectedApproval {
    key: ApprovalKey,
    intent: ToolIntent,
    call: TerminalCall,
    display: ToolDisplay,
}
impl ProjectedApproval {
    pub fn key(&self) -> ApprovalKey {
        self.key
    }
    pub fn intent(&self) -> &ToolIntent {
        &self.intent
    }
    pub fn call(&self) -> &TerminalCall {
        &self.call
    }
    pub fn display(&self) -> &ToolDisplay {
        &self.display
    }
    /// Caller reserves the bounded command metadata before this authority clone.
    pub fn request(
        &self,
        choice: ApprovalChoice,
    ) -> Result<super::intent::ApprovalRequest, ProjectionError> {
        super::intent::ApprovalRequest::new(
            ApprovalAnswer::new(
                self.intent.id(),
                self.intent.authority().clone(),
                self.intent.arguments(),
                choice,
            ),
            self.key.expires,
        )
    }
}
#[derive(Clone, Default)]
pub(super) struct ProjectionApprovals(BTreeMap<(RunKey, IntentId), ProjectedApproval>);
impl ProjectionApprovals {
    pub(super) fn iter(&self) -> impl Iterator<Item = &ProjectedApproval> {
        self.0.values()
    }
    pub(super) fn get(&self, wanted: ApprovalKey) -> Option<&ProjectedApproval> {
        self.0
            .get(&((wanted.run, wanted.epoch), wanted.intent))
            .filter(|card| card.key == wanted)
    }
    pub(super) fn bytes(&self) -> Result<usize, ProjectionError> {
        self.0.values().try_fold(0usize, |total, card| {
            Ok(total.saturating_add(super::sizing::approval_bytes(&card.call, &card.display)?))
        })
    }
    pub(super) fn observe(
        &mut self,
        event: &CoordinatorEvent,
        control: ChatControlRevision,
    ) -> Result<(), ProjectionError> {
        match event {
            CoordinatorEvent::Approval {
                intent,
                call,
                display,
                expires,
            } => self.insert(intent, call, display, *expires, control)?,
            CoordinatorEvent::ApprovalResolved { answer, .. } => {
                if let Some(card) = self
                    .0
                    .get(&(key(answer.authority().run()), answer.intent()))
                {
                    if card.intent.authority() != answer.authority()
                        || card.intent.arguments() != answer.arguments()
                    {
                        return Err(ProjectionError::InvalidRecord);
                    }
                    self.0
                        .remove(&(key(answer.authority().run()), answer.intent()));
                }
            }
            CoordinatorEvent::ToolResult { run, observation } => {
                self.0.remove(&(key(run), observation.intent()));
            }
            CoordinatorEvent::Status {
                run,
                status: RunStatus::Stopped | RunStatus::Completed | RunStatus::Interrupted,
            } => self.0.retain(|(owner, _), _| *owner != key(run)),
            CoordinatorEvent::Deleted { .. } => self.0.clear(),
            _ => {}
        }
        Ok(())
    }
    fn insert(
        &mut self,
        intent: &ToolIntent,
        call: &TerminalCall,
        display: &ToolDisplay,
        expires: Timestamp,
        control: ChatControlRevision,
    ) -> Result<(), ProjectionError> {
        // A historical signed card remains readable after its answer deadline.
        // The native command preparer enforces expiry before sealing an answer.
        if expires.as_millis() == 0 || call.name() != display.tool || display.version != 2 {
            return Err(ProjectionError::InvalidRecord);
        }
        call.validate()
            .map_err(|_| ProjectionError::InvalidRecord)?;
        if call.writes() != intent.mutates() {
            return Err(ProjectionError::InvalidRecord);
        }
        let index = (key(intent.authority().run()), intent.id());
        if self.0.contains_key(&index) {
            return Err(ProjectionError::InvalidRecord);
        }
        if self.0.len() >= MAX_PENDING_COMMANDS {
            return Err(ProjectionError::Capacity);
        }
        let authority = intent.authority();
        let run = authority.run();
        let session = authority.session();
        let key = ApprovalKey {
            run: run.run(),
            epoch: run.epoch(),
            intent: intent.id(),
            step: intent.step(),
            args: intent.arguments(),
            expires,
            control,
            session: session.id(),
            session_epoch: session.epoch(),
            grant: authority.grant(),
            grant_epoch: authority.grant_epoch(),
        };
        self.0.insert(
            index,
            ProjectedApproval {
                key,
                intent: intent.clone(),
                call: call.clone(),
                display: display.clone(),
            },
        );
        Ok(())
    }
}
