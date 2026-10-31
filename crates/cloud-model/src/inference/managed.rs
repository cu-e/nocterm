//! Identity of the inference a managed (pool-hosted) runner performs for a chat.
//! A device and a runner must agree on these names: the device sends them in a
//! prompt and the runner accepts only a selection whose provider and pricing match.

use super::ModelSelection;
use crate::identity::{NamespaceId, SourceIdentityError};

/// The provider name of every managed model: the host's credential broker, never a
/// provider the device or the runner configured.
pub const MANAGED_PROVIDER: &str = "managed";
/// The pricing policy of managed models. The broker, not this name, enforces the budget.
pub const MANAGED_PRICING: &str = "managed-v1";

impl ModelSelection {
    /// The selection of `model` on a managed runner.
    pub fn managed(model: &str) -> Result<Self, SourceIdentityError> {
        Ok(Self::new(
            NamespaceId::new(MANAGED_PROVIDER)?,
            NamespaceId::new(model)?,
            NamespaceId::new(MANAGED_PRICING)?,
        ))
    }
}

/// The pricing policy of a model the person's own runner calls with a key that a device of
/// the person leases to it (ADR 0126). The runner's configuration must name it as the
/// `pricing_revision` of the provider's models.
pub const OWN_PRICING: &str = "own-v1";

impl ModelSelection {
    /// The selection of `model` of `provider` on the person's own runner.
    pub fn own(provider: &str, model: &str) -> Result<Self, SourceIdentityError> {
        Ok(Self::new(
            NamespaceId::new(provider)?,
            NamespaceId::new(model)?,
            NamespaceId::new(OWN_PRICING)?,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_managed_selection_names_the_broker_provider_and_pricing() {
        let selection = ModelSelection::managed("gpt-5").unwrap();
        assert_eq!(selection.provider().as_str(), MANAGED_PROVIDER);
        assert_eq!(selection.model().as_str(), "gpt-5");
        assert_eq!(selection.pricing().as_str(), MANAGED_PRICING);
    }

    #[test]
    fn an_own_selection_names_the_provider_the_runner_configured() {
        let selection = ModelSelection::own("openai", "gpt-5").unwrap();
        assert_eq!(selection.provider().as_str(), "openai");
        assert_eq!(selection.pricing().as_str(), OWN_PRICING);
        assert!(ModelSelection::own("", "gpt-5").is_err());
    }

    #[test]
    fn a_model_that_is_not_a_namespace_id_is_refused() {
        assert!(ModelSelection::managed("").is_err());
    }
}
