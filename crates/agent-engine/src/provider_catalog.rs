//! Trusted provider policies and original-command memory credentials for actual Rig engines.
mod candidate;
mod state;
use crate::{ProviderBinding, ProviderLease, RigEngineFactory};
use candidate::Candidate;
use futures::future::BoxFuture;
use nocterm_cloud_model::{
    coordinator::{
        CommandAcknowledgement, ConfiguredModel, CoordinatorError, CoordinatorProviders,
        GrantClass, GrantReference, ProviderCredentialSource, ProviderOffer,
    },
    engine::EngineError,
    identity::{NamespaceId, Timestamp},
    inference::{ModelPricing, ModelSelection},
    tool::ToolTimer,
};
use state::CatalogState;
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

/// Endpoint, provider protocol, model allowlist and prices are administrator
/// configuration. A device offer can narrow these choices, never replace them.
pub struct ProviderPolicy {
    provider: NamespaceId,
    binding: ProviderBinding,
    pricing: Vec<Arc<dyn ModelPricing>>,
}
impl ProviderPolicy {
    pub fn new(
        provider: NamespaceId,
        binding: ProviderBinding,
        pricing: Vec<Arc<dyn ModelPricing>>,
    ) -> Result<Self, CoordinatorError> {
        if pricing.is_empty()
            || pricing.len() > 128
            || pricing.iter().enumerate().any(|(index, price)| {
                price.selection().provider() != &provider
                    || !binding.models().contains(price.selection().model())
                    || pricing[..index]
                        .iter()
                        .any(|old| old.selection() == price.selection())
            })
        {
            return Err(CoordinatorError::InvalidCommand);
        }
        Ok(Self {
            provider,
            binding,
            pricing,
        })
    }
    fn binding_for(&self, offer: &ProviderOffer) -> Result<ProviderBinding, CoordinatorError> {
        if offer
            .models()
            .iter()
            .any(|model| !self.binding.models().contains(model))
        {
            return Err(CoordinatorError::Unsupported);
        }
        Ok(ProviderBinding::new(
            self.binding.scope().clone(),
            self.binding.kind(),
            self.binding.endpoint().clone(),
            offer.models().to_vec(),
        )?)
    }
}

#[derive(Clone)]
pub struct ConfiguredProviderCatalog {
    owner: Arc<CatalogOwner>,
}
struct CatalogOwner {
    policies: BTreeMap<NamespaceId, ProviderPolicy>,
    source: Arc<dyn ProviderCredentialSource>,
    timer: Arc<dyn ToolTimer>,
    state: Mutex<CatalogState>,
}
impl ConfiguredProviderCatalog {
    /// Grant facts and revocation tombstones have a shared finite cap. Exhausted
    /// revocation metadata fails closed rather than forgetting an old grant.
    pub fn new(
        source: Arc<dyn ProviderCredentialSource>,
        timer: Arc<dyn ToolTimer>,
        policies: Vec<ProviderPolicy>,
        maximum_grants: usize,
    ) -> Result<Self, CoordinatorError> {
        if policies.is_empty()
            || policies.len() > 128
            || maximum_grants == 0
            || maximum_grants > 4096
        {
            return Err(CoordinatorError::InvalidCommand);
        }
        source.check_current()?;
        let mut configured = BTreeMap::new();
        for policy in policies {
            if policy.binding.scope() != source.binding().scope()
                || configured.contains_key(&policy.provider)
            {
                return Err(CoordinatorError::InvalidCommand);
            }
            configured.insert(policy.provider.clone(), policy);
        }
        Ok(Self {
            owner: Arc::new(CatalogOwner {
                policies: configured,
                source,
                timer,
                state: Mutex::new(CatalogState::new(maximum_grants)),
            }),
        })
    }
}
impl CatalogOwner {
    fn current(&self) -> Result<Timestamp, CoordinatorError> {
        if let Err(error) = self.source.check_current() {
            self.retire();
            return Err(error);
        }
        let now = self.timer.now();
        let healthy = self
            .state
            .lock()
            .map_err(|_| CoordinatorError::NativeTaskFailed)?
            .observe(now);
        if !healthy {
            self.retire();
            return Err(CoordinatorError::Expired);
        }
        Ok(now)
    }
    fn retire(&self) {
        let leases = self
            .state
            .lock()
            .map(|mut state| state.retire())
            .unwrap_or_default();
        for lease in leases {
            lease.revoke();
        }
    }
    fn install(
        &self,
        command: CommandAcknowledgement,
        offer: ProviderOffer,
    ) -> Result<(), CoordinatorError> {
        let now = self.current()?;
        if !self.source.binding().contains(command.stream())
            || command.acceptance().kind() != nocterm_cloud_model::journal::CommandKind::IssueGrant
            || offer.reference().class() != GrantClass::Provider
            || now >= offer.expires()
        {
            return Err(CoordinatorError::InvalidCommand);
        }
        let policy = self
            .policies
            .get(offer.provider())
            .ok_or(CoordinatorError::Unsupported)?;
        let binding = policy.binding_for(&offer)?;
        let ticket = self
            .state
            .lock()
            .map_err(|_| CoordinatorError::NativeTaskFailed)?
            .begin(&command, &offer)?;
        let Some(ticket) = ticket else {
            return Ok(());
        };
        let lease = match self.prepare(binding, &command, &offer) {
            Ok(lease) => lease,
            Err(error) => {
                self.state
                    .lock()
                    .map_err(|_| CoordinatorError::NativeTaskFailed)?
                    .abort(&ticket);
                return Err(error);
            }
        };
        let committed = self
            .state
            .lock()
            .map_err(|_| CoordinatorError::NativeTaskFailed)?
            .commit(&ticket, &offer, lease.clone());
        match committed {
            Ok(old) => {
                if let Some(old) = old {
                    old.revoke();
                }
                Ok(())
            }
            Err(error) => {
                self.state
                    .lock()
                    .map_err(|_| CoordinatorError::NativeTaskFailed)?
                    .abort(&ticket);
                lease.revoke();
                Err(error)
            }
        }
    }
    fn prepare(
        &self,
        binding: ProviderBinding,
        command: &CommandAcknowledgement,
        offer: &ProviderOffer,
    ) -> Result<ProviderLease, CoordinatorError> {
        let mut candidate = Candidate::new(
            binding,
            offer.expires(),
            self.source.clone(),
            self.timer.clone(),
        );
        self.source.consume(command, offer, &mut candidate)?;
        if self.current()? >= offer.expires() {
            return Err(CoordinatorError::Expired);
        }
        candidate.take()
    }
    fn model(&self, selection: &ModelSelection) -> Result<ConfiguredModel, EngineError> {
        let now = self
            .current()
            .map_err(|_| EngineError::NeedsProviderAccess)?;
        let policy = self
            .policies
            .get(selection.provider())
            .ok_or(EngineError::Unavailable)?;
        let pricing = policy
            .pricing
            .iter()
            .find(|price| price.selection() == selection)
            .ok_or(EngineError::Unavailable)?
            .clone();
        let lease = self
            .state
            .lock()
            .map_err(|_| EngineError::Unavailable)?
            .model(selection, now)?;
        if !lease.is_available() {
            return Err(EngineError::NeedsProviderAccess);
        }
        let factory = Arc::new(RigEngineFactory::new(lease)?);
        ConfiguredModel::new(selection.clone(), factory, pricing)
            .map_err(|_| EngineError::Unavailable)
    }
}
impl CoordinatorProviders for ConfiguredProviderCatalog {
    fn model(&self, selection: &ModelSelection) -> Result<ConfiguredModel, EngineError> {
        self.owner.model(selection)
    }
    fn install(
        &self,
        command: CommandAcknowledgement,
        offer: ProviderOffer,
    ) -> BoxFuture<'static, Result<(), CoordinatorError>> {
        let owner = self.owner.clone();
        Box::pin(async move { owner.install(command, offer) })
    }
    fn revoke(&self, reference: GrantReference) {
        if reference.class() != GrantClass::Provider {
            return;
        }
        let leases = self
            .owner
            .state
            .lock()
            .map(|mut state| state.revoke(reference))
            .unwrap_or_default();
        for lease in leases {
            lease.revoke();
        }
    }
}
