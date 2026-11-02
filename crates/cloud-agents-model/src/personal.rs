//! A personal agent a person describes in the desktop: what is accepted before anything is
//! sealed or sent.

use nocterm_session::Secret;

use crate::ids::PoolId;

/// Longest agent name.
pub const MAX_NAME_CHARS: usize = 80;
/// Longest endpoint URL.
pub const MAX_ENDPOINT_BYTES: usize = 512;
/// Longest model id.
pub const MAX_MODEL_BYTES: usize = 128;
/// Longest provider key.
pub const MAX_KEY_BYTES: usize = 1024;
/// Longest instructions text: the hub's bound for a system prompt.
pub const MAX_INSTRUCTIONS_BYTES: usize = 64 * 1024;

/// How the agent's provider is spoken to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ProviderKind {
    /// OpenAI-compatible chat completions.
    OpenAiChat,
    /// Anthropic messages.
    AnthropicMessages,
}

impl ProviderKind {
    /// Every kind, in the order a chooser lists them.
    pub const ALL: [Self; 2] = [Self::OpenAiChat, Self::AnthropicMessages];

    /// The id the hub and the runner use for the kind.
    pub fn id(self) -> &'static str {
        match self {
            Self::OpenAiChat => "openai_chat",
            Self::AnthropicMessages => "anthropic_messages",
        }
    }

    /// The name a chooser shows.
    pub fn label(self) -> &'static str {
        match self {
            Self::OpenAiChat => "OpenAI-compatible",
            Self::AnthropicMessages => "Anthropic",
        }
    }

    /// The kind with this id.
    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.id() == id)
    }
}

/// What the person left out or got wrong in the form.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum DraftError {
    /// The name is empty or too long.
    #[error("give the agent a name of up to {MAX_NAME_CHARS} characters")]
    Name,
    /// The endpoint is not an https URL (plain http is accepted for this computer only).
    #[error("the endpoint must be an https URL, or http for localhost")]
    Endpoint,
    /// The model is empty or too long.
    #[error("name the model")]
    Model,
    /// A new agent needs a provider key, or the key is too long.
    #[error("the provider key is missing or too long")]
    Key,
    /// Running while the person is offline needs a pool to run on.
    #[error("choose the pool the agent runs on")]
    Pool,
    /// The instructions are longer than the hub accepts.
    #[error("the instructions are longer than {MAX_INSTRUCTIONS_BYTES} bytes")]
    Instructions,
}

/// The provider key of a form: write-only, never read back.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KeyInput {
    /// Keep the key the saved agent has (an edit that does not retype it).
    Keep,
    /// Replace the key.
    Replace(Secret),
}

/// A personal agent as the form describes it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PersonalAgentDraft {
    name: String,
    provider: ProviderKind,
    endpoint: String,
    model: String,
    key: KeyInput,
    work_offline: bool,
    pool: Option<PoolId>,
    instructions: String,
}

impl PersonalAgentDraft {
    /// Validates the form. An empty endpoint means the provider's default. `pool` is required
    /// while `work_offline` is on, because the pool is where the agent works without a device.
    pub fn new(
        name: &str,
        provider: ProviderKind,
        endpoint: &str,
        model: &str,
        key: KeyInput,
        work_offline: bool,
        pool: Option<PoolId>,
    ) -> Result<Self, DraftError> {
        let name = name.trim();
        if name.is_empty()
            || name.chars().count() > MAX_NAME_CHARS
            || name.chars().any(char::is_control)
        {
            return Err(DraftError::Name);
        }
        let endpoint = endpoint.trim();
        if !endpoint.is_empty() && !acceptable_endpoint(endpoint) {
            return Err(DraftError::Endpoint);
        }
        let model = model.trim();
        if model.is_empty()
            || model.len() > MAX_MODEL_BYTES
            || model.chars().any(char::is_whitespace)
        {
            return Err(DraftError::Model);
        }
        if let KeyInput::Replace(secret) = &key {
            let key = secret.expose();
            if key.trim().is_empty() || key.len() > MAX_KEY_BYTES {
                return Err(DraftError::Key);
            }
        }
        if work_offline && pool.is_none() {
            return Err(DraftError::Pool);
        }
        Ok(Self {
            name: name.to_owned(),
            provider,
            endpoint: endpoint.to_owned(),
            model: model.to_owned(),
            key,
            work_offline,
            pool,
            instructions: String::new(),
        })
    }

    /// Gives the agent instructions: the system prompt it works under, sealed with the
    /// definition. Surrounding blank space is dropped.
    pub fn with_instructions(mut self, instructions: &str) -> Result<Self, DraftError> {
        let instructions = instructions.trim();
        if instructions.len() > MAX_INSTRUCTIONS_BYTES {
            return Err(DraftError::Instructions);
        }
        self.instructions = instructions.to_owned();
        Ok(self)
    }

    /// The instructions; empty for none.
    pub fn instructions(&self) -> &str {
        &self.instructions
    }

    /// The agent's name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The provider family.
    pub fn provider(&self) -> ProviderKind {
        self.provider
    }

    /// The endpoint; empty for the provider's default.
    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    /// The model id.
    pub fn model(&self) -> &str {
        &self.model
    }

    /// What the form does with the key.
    pub fn key(&self) -> &KeyInput {
        &self.key
    }

    /// Whether the agent keeps working while no device of the person is online.
    pub fn work_offline(&self) -> bool {
        self.work_offline
    }

    /// Where the agent runs: on a pool when it works offline, otherwise on the own runner.
    pub fn placement(&self) -> crate::placement::Placement {
        if self.work_offline {
            crate::placement::Placement::Pool
        } else {
            crate::placement::Placement::OwnRunner
        }
    }

    /// The pool the agent works on while the person is offline.
    pub fn pool(&self) -> Option<&PoolId> {
        self.pool.as_ref()
    }
}

fn acceptable_endpoint(endpoint: &str) -> bool {
    if endpoint.len() > MAX_ENDPOINT_BYTES
        || endpoint
            .chars()
            .any(|c| c.is_control() || c.is_whitespace())
    {
        return false;
    }
    if let Some(rest) = endpoint.strip_prefix("https://") {
        return !host_of(rest).is_empty();
    }
    endpoint.strip_prefix("http://").is_some_and(|rest| {
        let host = host_of(rest);
        host == "localhost" || host == "127.0.0.1" || host == "[::1]"
    })
}

fn host_of(rest: &str) -> &str {
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    let authority = authority.rsplit('@').next().unwrap_or_default();
    match authority.rfind(':') {
        Some(colon) if !authority.ends_with(']') => &authority[..colon],
        _ => authority,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draft(
        endpoint: &str,
        key: KeyInput,
        offline: bool,
        pool: bool,
    ) -> Result<PersonalAgentDraft, DraftError> {
        PersonalAgentDraft::new(
            " Helper ",
            ProviderKind::OpenAiChat,
            endpoint,
            "gpt-4o-mini",
            key,
            offline,
            pool.then(|| PoolId::new("p1").unwrap()),
        )
    }

    #[test]
    fn an_agent_that_does_not_work_offline_runs_on_the_own_runner() {
        let key = || KeyInput::Replace(Secret::new("sk-1"));
        let offline = draft("", key(), true, true).unwrap();
        let online = draft("", key(), false, false).unwrap();
        assert_eq!(offline.placement(), crate::Placement::Pool);
        assert_eq!(online.placement(), crate::Placement::OwnRunner);
    }

    #[test]
    fn a_complete_form_is_trimmed_and_accepted() {
        let ok = draft("", KeyInput::Replace(Secret::new("sk-1")), true, true).unwrap();
        assert_eq!(ok.name(), "Helper");
        assert!(ok.work_offline());
    }

    #[test]
    fn plain_http_is_accepted_for_localhost_only() {
        let key = || KeyInput::Replace(Secret::new("sk"));
        assert!(draft("http://localhost:11434/v1", key(), false, false).is_ok());
        assert!(draft("https://api.example.com/v1", key(), false, false).is_ok());
        assert_eq!(
            draft("http://api.example.com", key(), false, false),
            Err(DraftError::Endpoint)
        );
        assert_eq!(
            draft("https://", key(), false, false),
            Err(DraftError::Endpoint)
        );
        assert_eq!(
            draft("http://localhost@evil.example/", key(), false, false),
            Err(DraftError::Endpoint)
        );
    }

    #[test]
    fn working_offline_needs_a_pool_and_a_replaced_key_must_not_be_blank() {
        assert_eq!(
            draft("", KeyInput::Keep, true, false),
            Err(DraftError::Pool)
        );
        assert_eq!(
            draft("", KeyInput::Replace(Secret::new("  ")), false, false),
            Err(DraftError::Key)
        );
        assert!(draft("", KeyInput::Keep, false, false).is_ok());
    }

    #[test]
    fn instructions_are_trimmed_and_bounded() {
        let ok = draft("", KeyInput::Keep, false, false).unwrap();
        assert_eq!(ok.instructions(), "");
        let told = ok.clone().with_instructions("  Be brief.\n").unwrap();
        assert_eq!(told.instructions(), "Be brief.");
        let long = "x".repeat(MAX_INSTRUCTIONS_BYTES + 1);
        assert_eq!(ok.with_instructions(&long), Err(DraftError::Instructions));
    }

    #[test]
    fn name_and_model_are_required() {
        let key = || KeyInput::Replace(Secret::new("sk"));
        assert_eq!(
            PersonalAgentDraft::new("  ", ProviderKind::OpenAiChat, "", "m", key(), false, None),
            Err(DraftError::Name)
        );
        assert_eq!(
            PersonalAgentDraft::new(
                "a",
                ProviderKind::OpenAiChat,
                "",
                "two words",
                key(),
                false,
                None
            ),
            Err(DraftError::Model)
        );
    }

    #[test]
    fn a_secret_never_shows_in_debug_output() {
        let ok = draft(
            "",
            KeyInput::Replace(Secret::new("sk-secret")),
            false,
            false,
        )
        .unwrap();
        assert!(!format!("{ok:?}").contains("sk-secret"));
    }
}
