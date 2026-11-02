//! Provider access of a managed runner: model calls go to the host's credential
//! broker with the calling chat's own virtual token (ADR 0124). The runner never
//! holds a provider key and installs no device grant.
use super::{
    BootV2, ChatTokens, ManagedPolicy, ProviderWake,
    spend::{CeilingPricing, SpendCeilings},
};
use crate::RunnerError;
use futures::{FutureExt, future::BoxFuture};
use nocterm_agent_engine::RigEngineFactory;
use nocterm_cloud_model::{
    coordinator::{
        CommandAcknowledgement, ConfiguredModel, CoordinatorError, CoordinatorProviders,
        GrantReference, ProviderOffer,
    },
    engine::{AgentEngine, AgentEngineFactory, EngineCheckpoint, EngineConfig, EngineError},
    identity::{AccountScope, CloudChatId, NamespaceId},
    inference::{ModelPricing, ModelSelection},
};
use nocterm_cloud_runner::coordinator::RunCoordinator;
use std::sync::Arc;

/// The provider namespace every managed model selection uses.
pub(crate) const MANAGED_PROVIDER: &str = "managed";
/// The pricing revision of the ceilings the host renders into boot v2.
pub(crate) const MANAGED_PRICING: &str = "managed-v1";

/// Creates an engine from the lease of the chat that asks for it, so one
/// selection serves many chats without sharing a token between them.
struct ChatBrokeredFactory {
    tokens: Arc<ChatTokens>,
    policy: ManagedPolicy,
    instructions: Arc<str>,
}
impl AgentEngineFactory for ChatBrokeredFactory {
    fn create(
        &self,
        mut config: EngineConfig,
        restore: Option<EngineCheckpoint>,
    ) -> Result<Box<dyn AgentEngine>, EngineError> {
        let lease = self.tokens.lease(config.run.chat())?;
        // The policy lowers the command's limits and the offered tools; it
        // never raises either (ADR 0127).
        config.budget = self.policy.narrow(config.budget)?;
        let mut factory = RigEngineFactory::new(lease)?.with_instructions(&self.instructions);
        if let Some(tools) = self.policy.allowed_tools() {
            factory = factory.with_allowed_tools(tools.clone());
        }
        factory.create(config, restore)
    }
}

pub(crate) struct BrokeredProviders {
    tokens: Arc<ChatTokens>,
    provider: NamespaceId,
    revision: NamespaceId,
    pricing: Vec<Arc<dyn ModelPricing>>,
    policy: ManagedPolicy,
    instructions: Arc<str>,
}
impl BrokeredProviders {
    /// The boot document names the broker, the models and their ceilings. The
    /// runtime hosts the leases the control reader issues.
    pub(crate) fn new(
        boot: &BootV2,
        scope: &AccountScope,
        runtime: tokio::runtime::Handle,
    ) -> Result<Self, RunnerError> {
        let identity = |text: &str| {
            NamespaceId::new(text.to_owned()).map_err(|_| RunnerError::Value("managed.provider"))
        };
        let provider = identity(MANAGED_PROVIDER)?;
        let revision = identity(MANAGED_PRICING)?;
        let binding = nocterm_agent_engine::ProviderBinding::new(
            scope.clone(),
            boot.kind(),
            boot.endpoint()?,
            boot.models().to_vec(),
        )?;
        let policy = boot.policy().clone();
        let ceilings = SpendCeilings::new(policy.spend_per_run(), policy.spend_per_chat());
        let pricing = boot
            .pricing(&provider, &revision)?
            .into_iter()
            .map(|price| {
                let price: Arc<dyn ModelPricing> = price;
                match &ceilings {
                    Some(ceilings) => Arc::new(CeilingPricing::new(price, ceilings.clone())),
                    None => price,
                }
            })
            .collect();
        Ok(Self {
            tokens: Arc::new(ChatTokens::new(binding, runtime)),
            provider,
            revision,
            pricing,
            policy,
            instructions: Arc::from(boot.instructions()),
        })
    }
    pub(crate) fn tokens(&self) -> &Arc<ChatTokens> {
        &self.tokens
    }
    pub(crate) fn policy(&self) -> &ManagedPolicy {
        &self.policy
    }
    pub(crate) fn provider(&self) -> &NamespaceId {
        &self.provider
    }
}
impl CoordinatorProviders for BrokeredProviders {
    fn model(&self, selection: &ModelSelection) -> Result<ConfiguredModel, EngineError> {
        if selection.provider() != &self.provider || selection.pricing() != &self.revision {
            return Err(EngineError::Unavailable);
        }
        let pricing = self
            .pricing
            .iter()
            .find(|price| price.selection() == selection)
            .ok_or(EngineError::Unavailable)?
            .clone();
        let factory = Arc::new(ChatBrokeredFactory {
            tokens: self.tokens.clone(),
            policy: self.policy.clone(),
            instructions: self.instructions.clone(),
        });
        ConfiguredModel::new(selection.clone(), factory, pricing)
            .map_err(|_| EngineError::Unavailable)
    }
    /// A device cannot install a provider key into a managed runner.
    fn install(
        &self,
        _command: CommandAcknowledgement,
        _offer: ProviderOffer,
    ) -> BoxFuture<'static, Result<(), CoordinatorError>> {
        futures::future::ready(Err(CoordinatorError::Unsupported)).boxed()
    }
    fn revoke(&self, _reference: GrantReference) {}
}

/// Lets the control reader wake the coordinator's parked runs.
pub(crate) struct CoordinatorWake {
    coordinator: RunCoordinator,
    provider: NamespaceId,
}
impl CoordinatorWake {
    pub(crate) fn new(coordinator: RunCoordinator, provider: NamespaceId) -> Self {
        Self {
            coordinator,
            provider,
        }
    }
}
impl ProviderWake for CoordinatorWake {
    fn wake(&self, chat: CloudChatId) {
        self.coordinator.wake_provider(chat, &self.provider);
    }
}

#[cfg(test)]
#[path = "providers_tests.rs"]
mod tests;
