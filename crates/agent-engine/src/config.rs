use crate::{call::ModelCalls, driver::RigEngine, http::ProviderHttp, lease::ProviderLease};
use nocterm_cloud_model::{
    engine::{AgentEngine, AgentEngineFactory, EngineCheckpoint, EngineConfig, EngineError},
    identity::{AccountScope, NamespaceId},
};
use rig_core::{
    DynModel,
    operation::Completion,
    providers::{anthropic::AnthropicConfig, openai::OpenAIConfig},
};
use std::{collections::BTreeSet, fmt, net::IpAddr};
use url::Url;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProviderKind {
    OpenAiChat,
    AnthropicMessages,
}

/// An administrator-selected API root, captured before any model input.
#[derive(Clone)]
pub struct ProviderEndpoint {
    url: Url,
    fixture: bool,
    broker: bool,
}
impl ProviderEndpoint {
    /// Only HTTPS roots without user information, query or fragment are allowed.
    /// Every DNS answer is checked again by the connecting transport.
    pub fn new(root: &str) -> Result<Self, EngineError> {
        if root.is_empty() || root.len() > 4096 {
            return Err(EngineError::Unavailable);
        }
        let url = Url::parse(root).map_err(|_| EngineError::Unavailable)?;
        if url.as_str().len() > 4096 {
            return Err(EngineError::Unavailable);
        }
        if url.scheme() != "https" || !valid_root(&url) {
            return Err(EngineError::Unavailable);
        }
        if let Some(host) = url.host_str()
            && let Ok(ip) = host.trim_matches(['[', ']']).parse::<IpAddr>()
            && !crate::http::is_public(ip)
        {
            return Err(EngineError::Unavailable);
        }
        Ok(Self {
            url,
            fixture: false,
            broker: false,
        })
    }
    /// The credential broker on the pool host that substitutes the real API key.
    /// The host chooses where it listens, so a private or loopback address is
    /// allowed here, and plain HTTP only for such an address. No public name
    /// ever receives a plain-text request.
    pub fn broker(root: &str) -> Result<Self, EngineError> {
        if root.is_empty() || root.len() > 4096 {
            return Err(EngineError::Unavailable);
        }
        let url = Url::parse(root).map_err(|_| EngineError::Unavailable)?;
        if !valid_root(&url) {
            return Err(EngineError::Unavailable);
        }
        let local_ip = url
            .host_str()
            .and_then(|host| host.trim_matches(['[', ']']).parse::<IpAddr>().ok())
            .is_some_and(|ip| !crate::http::is_public(ip));
        match url.scheme() {
            "https" => {}
            "http" if local_ip => {}
            _ => return Err(EngineError::Unavailable),
        }
        Ok(Self {
            url,
            fixture: false,
            broker: true,
        })
    }
    pub(crate) fn root(&self) -> &str {
        self.url.as_str().trim_end_matches('/')
    }
    pub(crate) fn request_url(&self, kind: ProviderKind) -> String {
        let root = self.root();
        match kind {
            ProviderKind::OpenAiChat => format!("{root}/chat/completions"),
            ProviderKind::AnthropicMessages => {
                format!(
                    "{}/v1/messages",
                    rig_core::providers::anthropic::wire::normalize_base_url(root)
                )
            }
        }
    }
    /// A broker endpoint is host-chosen, so it is neither https-only nor
    /// restricted to public addresses.
    pub(crate) fn plain_allowed(&self) -> bool {
        self.fixture || (self.broker && self.url.scheme() == "http")
    }
    pub(crate) fn private_allowed(&self) -> bool {
        self.broker
    }
    #[cfg(test)]
    pub(crate) fn for_fixture(root: &str) -> Self {
        let url = Url::parse(root).expect("fixture URL");
        assert_eq!(url.scheme(), "http");
        assert!(
            url.host_str()
                .is_some_and(|s| s.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback()))
        );
        assert!(valid_root(&url));
        Self {
            url,
            fixture: true,
            broker: false,
        }
    }
}
impl fmt::Debug for ProviderEndpoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ProviderEndpoint([configured])")
    }
}
fn valid_root(url: &Url) -> bool {
    url.host_str().is_some()
        && url.username().is_empty()
        && url.password().is_none()
        && url.query().is_none()
        && url.fragment().is_none()
        && !url.cannot_be_a_base()
}

/// Immutable provider authority captured at key installation, never from a prompt.
#[derive(Clone, Debug)]
pub struct ProviderBinding {
    scope: AccountScope,
    kind: ProviderKind,
    endpoint: ProviderEndpoint,
    models: Vec<NamespaceId>,
}
impl ProviderBinding {
    pub fn new(
        scope: AccountScope,
        kind: ProviderKind,
        endpoint: ProviderEndpoint,
        models: Vec<NamespaceId>,
    ) -> Result<Self, EngineError> {
        if models.is_empty()
            || models.len() > 128
            || models
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
                .len()
                != models.len()
        {
            return Err(EngineError::Unavailable);
        }
        Ok(Self {
            scope,
            kind,
            endpoint,
            models,
        })
    }
    pub fn scope(&self) -> &AccountScope {
        &self.scope
    }
    pub fn kind(&self) -> ProviderKind {
        self.kind
    }
    pub fn endpoint(&self) -> &ProviderEndpoint {
        &self.endpoint
    }
    pub fn models(&self) -> &[NamespaceId] {
        &self.models
    }
}

/// Creates engines using only the captured policy of its original provider lease.
pub struct RigEngineFactory {
    binding: ProviderBinding,
    transport: ProviderHttp,
    lease: ProviderLease,
    tools: Option<BTreeSet<String>>,
}
impl RigEngineFactory {
    pub fn new(lease: ProviderLease) -> Result<Self, EngineError> {
        if !lease.is_available() {
            return Err(EngineError::NeedsProviderAccess);
        }
        let binding = lease.binding().clone();
        let transport = ProviderHttp::new(lease.clone())?;
        Ok(Self {
            binding,
            transport,
            lease,
            tools: None,
        })
    }
    /// Offers the model only the named catalog tools, and refuses a call to any
    /// other. A name that is not in the catalog never matches. The default
    /// offers the whole catalog.
    pub fn with_allowed_tools(mut self, names: BTreeSet<String>) -> Self {
        self.tools = Some(names);
        self
    }
    fn model(&self, model: &str, transport: ProviderHttp) -> DynModel<Completion> {
        // The SDK never receives the real credential. The pinned transport adds
        // it only after checking the exact configured request destination.
        match self.binding.kind {
            ProviderKind::OpenAiChat => OpenAIConfig::new("")
                .with_base_url(self.binding.endpoint.root())
                .connect(transport)
                .chat(model)
                .erase(),
            ProviderKind::AnthropicMessages => AnthropicConfig::new("")
                .with_base_url(self.binding.endpoint.root())
                .connect(transport)
                .completion(model)
                .erase(),
        }
    }
}
impl AgentEngineFactory for RigEngineFactory {
    fn create(
        &self,
        config: EngineConfig,
        restore: Option<EngineCheckpoint>,
    ) -> Result<Box<dyn AgentEngine>, EngineError> {
        if config.run.scope() != &self.binding.scope
            || !self.binding.models.contains(&config.model)
            || !self.lease.allows_run(&config.run)
        {
            return Err(EngineError::Unavailable);
        }
        if !self.lease.is_available() {
            return Err(EngineError::NeedsProviderAccess);
        }
        if let Some(usage) = self.lease.usage() {
            usage.run_started();
        }
        let calls = ModelCalls::new(config.run.clone());
        let model = self.model(
            config.model.as_str(),
            self.transport.for_engine(calls.clone()),
        );
        let identity = crate::checkpoint::EngineIdentity::new(
            &config,
            self.binding.kind,
            self.binding.endpoint.root(),
        )?;
        Ok(Box::new(RigEngine::new(
            config,
            identity,
            model,
            self.lease.clone(),
            restore,
            calls,
            self.tools.as_ref(),
        )?))
    }
}
impl fmt::Debug for RigEngineFactory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RigEngineFactory")
            .field("binding", &self.binding)
            .finish()
    }
}
