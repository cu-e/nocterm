//! Administrator-selected API endpoints, model allowlists and finite cost ceilings.
//! Credentials arrive only through the native boot-bound account capsule path.
use crate::RunnerError;
use nocterm_agent_engine::{ProviderBinding, ProviderEndpoint, ProviderKind, ProviderPolicy};
use nocterm_cloud_model::{
    identity::{AccountScope, NamespaceId},
    inference::{ConservativeModelPricing, ModelPricing, ModelSelection},
};
use serde::Deserialize;
#[cfg(target_os = "linux")]
use std::path::Path;
use std::{path::PathBuf, sync::Arc};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ExecutionConfig {
    containment: ContainmentDocument,
    /// Empty for a managed runner: its provider is the host broker (ADR 0124).
    #[serde(default)]
    providers: Vec<ProviderDocument>,
    maximum_provider_grants: usize,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ContainmentDocument {
    workspace: PathBuf,
    maximum_address_space_bytes: u64,
    maximum_processes: u32,
    maximum_descriptors: u32,
    maximum_cpu_seconds: u64,
    maximum_file_bytes: u64,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProviderDocument {
    id: String,
    protocol: Protocol,
    endpoint: String,
    models: Vec<ModelDocument>,
}
#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Protocol {
    OpenaiChat,
    AnthropicMessages,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ModelDocument {
    id: String,
    pricing_revision: String,
    maximum_request_bytes: usize,
    maximum_output_tokens: u64,
    maximum_call_cost_micros: u64,
}
impl ExecutionConfig {
    #[cfg(target_os = "linux")]
    pub(crate) async fn prepare_containment(
        &self,
        state: &Path,
    ) -> Result<nocterm_local::ContainedLocal, RunnerError> {
        let configured = &self.containment;
        let limits = nocterm_local::ContainmentLimits::new(
            configured.maximum_address_space_bytes,
            configured.maximum_processes,
            configured.maximum_descriptors,
            configured.maximum_cpu_seconds,
            configured.maximum_file_bytes,
        )?;
        Ok(nocterm_local::ContainedLocal::prepare(
            configured.workspace.clone(),
            state.to_owned(),
            limits,
        )
        .await?)
    }
    /// A managed runner reaches providers only through the host broker, so its
    /// configuration lists none.
    pub(crate) fn is_brokered(&self) -> bool {
        self.providers.is_empty()
    }
    pub(crate) fn maximum_grants(&self) -> Result<usize, RunnerError> {
        if self.maximum_provider_grants == 0 || self.maximum_provider_grants > 4096 {
            return Err(RunnerError::Value("execution.maximum_provider_grants"));
        }
        Ok(self.maximum_provider_grants)
    }
    pub(crate) fn policies(self, scope: &AccountScope) -> Result<Vec<ProviderPolicy>, RunnerError> {
        self.maximum_grants()?;
        if self.providers.is_empty() || self.providers.len() > 16 {
            return Err(RunnerError::Value("execution.providers"));
        }
        self.providers
            .into_iter()
            .map(|entry| entry.policy(scope))
            .collect()
    }
}
impl ProviderDocument {
    fn policy(self, scope: &AccountScope) -> Result<ProviderPolicy, RunnerError> {
        if self.models.is_empty() || self.models.len() > 32 {
            return Err(RunnerError::Value("execution.provider.models"));
        }
        let provider = namespace(self.id)?;
        let endpoint = ProviderEndpoint::new(&self.endpoint)?;
        let kind = match self.protocol {
            Protocol::OpenaiChat => ProviderKind::OpenAiChat,
            Protocol::AnthropicMessages => ProviderKind::AnthropicMessages,
        };
        let mut models = Vec::with_capacity(self.models.len());
        let mut pricing: Vec<Arc<dyn ModelPricing>> = Vec::with_capacity(self.models.len());
        for model in self.models {
            let id = namespace(model.id)?;
            let selection = ModelSelection::new(
                provider.clone(),
                id.clone(),
                namespace(model.pricing_revision)?,
            );
            pricing.push(Arc::new(
                ConservativeModelPricing::new(
                    selection,
                    model.maximum_request_bytes,
                    model.maximum_output_tokens,
                    model.maximum_call_cost_micros,
                )
                .map_err(|_| RunnerError::Value("execution.provider.pricing"))?,
            ));
            models.push(id);
        }
        let binding = ProviderBinding::new(scope.clone(), kind, endpoint, models)?;
        Ok(ProviderPolicy::new(provider, binding, pricing)?)
    }
}
fn namespace(value: String) -> Result<NamespaceId, RunnerError> {
    NamespaceId::new(value).map_err(|_| RunnerError::Value("execution.provider.identity"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use nocterm_cloud_model::identity::{AccountIdentity, Fingerprint, HubIdentity};
    fn document() -> String {
        "maximum_provider_grants = 16\n[containment]\nworkspace = \"/private/account-workspace\"\nmaximum_address_space_bytes = 268435456\nmaximum_processes = 16\nmaximum_descriptors = 64\nmaximum_cpu_seconds = 60\nmaximum_file_bytes = 1048576\n[[providers]]\nid = \"provider\"\nprotocol = \"openai_chat\"\nendpoint = \"https://api.example.org/v1\"\n[[providers.models]]\nid = \"model\"\npricing_revision = \"revision1\"\nmaximum_request_bytes = 65536\nmaximum_output_tokens = 2048\nmaximum_call_cost_micros = 50000\n".into()
    }
    fn scope() -> AccountScope {
        AccountScope::new(
            HubIdentity::new(
                namespace("hub".into()).unwrap(),
                Fingerprint::from_bytes([1; 32]),
            ),
            AccountIdentity::new(
                namespace("account".into()).unwrap(),
                Fingerprint::from_bytes([2; 32]),
            ),
        )
    }
    #[test]
    fn execution_configuration_requires_explicit_public_endpoint_and_bounded_cost_without_keys() {
        let text = document();
        let valid: ExecutionConfig = toml::from_str(&text).unwrap();
        assert_eq!(valid.policies(&scope()).unwrap().len(), 1);
        let plaintext_key = text.replace(
            "protocol =",
            "api_key = \"never-configured-here\"\nprotocol =",
        );
        assert!(toml::from_str::<ExecutionConfig>(&plaintext_key).is_err());
        let insecure: ExecutionConfig =
            toml::from_str(&text.replace("https://api.example.org", "http://127.0.0.1")).unwrap();
        assert!(insecure.policies(&scope()).is_err());
        let unknown_cost: ExecutionConfig = toml::from_str(&text.replace(
            "maximum_call_cost_micros = 50000",
            "maximum_call_cost_micros = 0",
        ))
        .unwrap();
        assert!(unknown_cost.policies(&scope()).is_err());
        let unbounded: ExecutionConfig = toml::from_str(&text.replace(
            "maximum_provider_grants = 16",
            "maximum_provider_grants = 4097",
        ))
        .unwrap();
        assert!(unbounded.policies(&scope()).is_err());
    }

    #[test]
    fn a_managed_configuration_lists_no_provider_and_still_bounds_containment() {
        let text = document();
        let head = text.split("[[providers]]").next().unwrap();
        let managed: ExecutionConfig = toml::from_str(head).unwrap();
        assert!(managed.is_brokered());
        assert!(managed.policies(&scope()).is_err());
        let own: ExecutionConfig = toml::from_str(&text).unwrap();
        assert!(!own.is_brokered());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn unusable_containment_is_a_typed_error_the_managed_runner_can_degrade_on() {
        let managed: ExecutionConfig =
            toml::from_str(document().split("[[providers]]").next().unwrap()).unwrap();
        let state = tempfile::tempdir().unwrap();
        let prepared = futures::executor::block_on(managed.prepare_containment(state.path()));
        assert!(matches!(prepared, Err(RunnerError::Containment(_))));
    }
}
