//! Model providers: an API endpoint with a key, the hub's or the account's own.

use std::fmt;

use nocterm_session::Secret;

use crate::error::DraftError;
use crate::ids::ProviderId;

/// The longest provider name, in characters.
const MAX_PROVIDER_NAME_CHARS: usize = 80;
/// The longest base URL, in bytes.
const MAX_BASE_URL_BYTES: usize = 2048;

/// The API a provider speaks.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ProviderKind {
    /// OpenAI-compatible chat completions (OpenAI, Gemini, OpenRouter, vLLM, Ollama, ...).
    #[default]
    OpenAi,
    /// The Anthropic messages API.
    Anthropic,
}

impl ProviderKind {
    /// The base URL a new provider of this kind starts with.
    pub fn default_base_url(self) -> &'static str {
        match self {
            Self::OpenAi => "https://api.openai.com/v1",
            Self::Anthropic => "https://api.anthropic.com",
        }
    }
}

/// Who a provider belongs to.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ProviderScope {
    /// The account's own; only its owner sees and uses it.
    #[default]
    Account,
    /// The hub's; administrators manage it and every account may use it.
    Hub,
}

/// A provider the account may use, as the hub lists it. Its key is never returned.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Provider {
    pub id: ProviderId,
    pub scope: ProviderScope,
    pub kind: ProviderKind,
    pub name: String,
    pub base_url: String,
    /// The models agents may use through it.
    pub models: Vec<String>,
    pub has_key: bool,
    /// The caller may change or delete it.
    pub editable: bool,
}

/// What a person typed to add or change a provider of their own.
#[derive(Clone, Default, PartialEq)]
pub struct ProviderDraft {
    pub kind: ProviderKind,
    pub name: String,
    pub base_url: String,
    pub models: Vec<String>,
    /// Write-only. `None` on a change keeps the stored key.
    pub api_key: Option<Secret>,
}

impl fmt::Debug for ProviderDraft {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProviderDraft")
            .field("kind", &self.kind)
            .field("name", &self.name)
            .field("base_url", &self.base_url)
            .field("models", &self.models)
            .field("api_key", &self.api_key.as_ref().map(|_| "<redacted>"))
            .finish()
    }
}

impl ProviderDraft {
    /// The draft with trimmed text and models, or the first rule it breaks.
    pub fn validate(mut self) -> Result<Self, DraftError> {
        self.name = self.name.trim().to_owned();
        if self.name.is_empty() {
            return Err(DraftError::NameMissing);
        }
        if self.name.chars().count() > MAX_PROVIDER_NAME_CHARS {
            return Err(DraftError::NameTooLong);
        }
        self.base_url = self.base_url.trim().trim_end_matches('/').to_owned();
        if !self.base_url.starts_with("https://") && !self.base_url.starts_with("http://") {
            return Err(DraftError::BaseUrl);
        }
        if self.base_url.len() > MAX_BASE_URL_BYTES {
            return Err(DraftError::BaseUrl);
        }
        let mut models: Vec<String> = Vec::new();
        for model in self.models.iter().map(|model| model.trim()) {
            if !model.is_empty() && !models.iter().any(|known| known == model) {
                models.push(model.to_owned());
            }
        }
        self.models = models;
        if self
            .api_key
            .as_ref()
            .is_some_and(|key| key.expose().trim().is_empty())
        {
            self.api_key = None;
        }
        Ok(self)
    }
}

/// What testing a provider showed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProviderTest {
    pub ok: bool,
    /// What went wrong, readable by a person; empty when it worked.
    pub message: String,
    /// The models the endpoint lists, when it lists any.
    pub models: Vec<String>,
}
