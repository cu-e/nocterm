//! Cloud command requests remain distinct from neutral Local renderer actions.
use super::{ProjectionError, ProviderAccess};
use crate::{coordinator::ApprovalAnswer, identity::Timestamp};
use nocterm_ai::conversation::ConversationAction;

#[derive(Clone, Debug)]
pub struct ApprovalRequest {
    answer: ApprovalAnswer,
    expires: Timestamp,
}
impl ApprovalRequest {
    pub fn new(answer: ApprovalAnswer, expires: Timestamp) -> Result<Self, ProjectionError> {
        if expires.as_millis() == 0 {
            return Err(ProjectionError::InvalidRecord);
        }
        Ok(Self { answer, expires })
    }
    pub fn answer(&self) -> &ApprovalAnswer {
        &self.answer
    }
    pub fn expires(&self) -> Timestamp {
        self.expires
    }
}
impl PartialEq for ApprovalRequest {
    fn eq(&self, other: &Self) -> bool {
        self.expires == other.expires
            && self.answer.intent() == other.answer.intent()
            && self.answer.authority() == other.answer.authority()
            && self.answer.arguments() == other.answer.arguments()
            && self.answer.choice() == other.answer.choice()
    }
}
impl Eq for ApprovalRequest {}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConversationIntent {
    Prompt(String),
    Stop,
    /// Lets the run named by the command coordinates use the model (ADR 0129).
    GrantInference,
    AnswerApproval(Box<ApprovalRequest>),
    /// Hands the runner a sealed lease of the provider credential.
    GrantProvider(Box<ProviderAccess>),
}
impl From<ConversationAction> for ConversationIntent {
    fn from(action: ConversationAction) -> Self {
        match action {
            ConversationAction::Prompt(text) => Self::Prompt(text),
            ConversationAction::Stop => Self::Stop,
            ConversationAction::Allow => Self::GrantInference,
        }
    }
}
impl PartialEq<ConversationAction> for ConversationIntent {
    fn eq(&self, other: &ConversationAction) -> bool {
        match (self, other) {
            (Self::Prompt(a), ConversationAction::Prompt(b)) => a == b,
            (Self::Stop, ConversationAction::Stop) => true,
            (Self::GrantInference, ConversationAction::Allow) => true,
            _ => false,
        }
    }
}
impl ConversationIntent {
    pub fn resident_bytes(&self) -> usize {
        4096usize.saturating_add(match self {
            Self::Prompt(text) => text.len().saturating_mul(2),
            Self::Stop | Self::GrantInference | Self::AnswerApproval(_) => 0,
            Self::GrantProvider(access) => access.resident_bytes(),
        })
    }
}
