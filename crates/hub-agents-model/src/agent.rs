//! An agent: a named configuration that costs nothing while no chat runs it.

use crate::error::DraftError;
use crate::ids::{AgentId, ProviderId, ServerId};

/// The longest agent name, in characters.
pub const MAX_NAME_CHARS: usize = 80;
/// The longest instructions, in bytes.
pub const MAX_INSTRUCTIONS_BYTES: usize = 32 * 1024;
/// The longest model name, in bytes.
const MAX_MODEL_BYTES: usize = 200;

/// When an agent waits for a person before it uses a tool.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ApprovalMode {
    /// Every tool runs at once.
    #[default]
    Auto,
    /// Tools that change something wait for "Allow / Allow in this chat / Deny" from any
    /// device; read-only tools run at once.
    Ask,
}

/// One agent of the account, as the hub lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HubAgent {
    pub id: AgentId,
    pub name: String,
    pub instructions: String,
    pub provider: ProviderId,
    /// The model new chats start with; always one of `models`.
    pub model: String,
    /// The models a chat may switch to.
    pub models: Vec<String>,
    pub approval: ApprovalMode,
    /// The servers new chats start with.
    pub servers: Vec<ServerId>,
}

impl HubAgent {
    /// The form that edits this agent.
    pub fn draft(&self) -> AgentDraft {
        AgentDraft {
            name: self.name.clone(),
            instructions: self.instructions.clone(),
            provider: Some(self.provider.clone()),
            model: self.model.clone(),
            models: self.models.clone(),
            approval: self.approval,
            servers: self.servers.clone(),
        }
    }
}

/// What a person typed to create or change an agent. [`AgentDraft::validate`] checks it
/// before it reaches the hub.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AgentDraft {
    pub name: String,
    pub instructions: String,
    pub provider: Option<ProviderId>,
    /// The default model; empty takes the first of `models`.
    pub model: String,
    pub models: Vec<String>,
    pub approval: ApprovalMode,
    pub servers: Vec<ServerId>,
}

impl AgentDraft {
    /// The draft with trimmed text, duplicate models dropped and the default model among the
    /// models, or the first rule it breaks.
    pub fn validate(mut self) -> Result<Self, DraftError> {
        self.name = self.name.trim().to_owned();
        if self.name.is_empty() {
            return Err(DraftError::NameMissing);
        }
        if self.name.chars().count() > MAX_NAME_CHARS {
            return Err(DraftError::NameTooLong);
        }
        if self.instructions.len() > MAX_INSTRUCTIONS_BYTES {
            return Err(DraftError::InstructionsTooLong);
        }
        if self.provider.is_none() {
            return Err(DraftError::ProviderMissing);
        }
        let mut models: Vec<String> = Vec::with_capacity(self.models.len());
        for model in self.models.iter().map(|model| model.trim()) {
            if model.is_empty() || models.iter().any(|known| known == model) {
                continue;
            }
            if model.len() > MAX_MODEL_BYTES {
                return Err(DraftError::ModelTooLong);
            }
            models.push(model.to_owned());
        }
        let model = self.model.trim();
        let model = if model.is_empty() {
            models.first().cloned().ok_or(DraftError::ModelMissing)?
        } else {
            model.to_owned()
        };
        if !models.contains(&model) {
            models.insert(0, model.clone());
        }
        self.model = model;
        self.models = models;
        self.servers.dedup();
        Ok(self)
    }
}
