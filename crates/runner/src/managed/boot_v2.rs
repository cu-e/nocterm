//! Boot document version 2: the managed document of a runner whose chat tokens
//! arrive on the control channel. It is boot v1 without `provider.token`, plus
//! the pricing ceilings the runner applies to every model (ADR 0123).
use super::{
    MAX_BOOT_BYTES, Shape, SshTarget, check_shape,
    policy::{ManagedPolicy, PolicyDocument},
};
use crate::RunnerError;
use nocterm_agent_engine::{ProviderEndpoint, ProviderKind};
use nocterm_cloud_model::{
    identity::NamespaceId,
    inference::{ConservativeModelPricing, ModelSelection},
};
use serde::Deserialize;
use std::sync::Arc;

/// The only format version of this document.
const BOOT_VERSION: u32 = 2;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Pricing {
    maximum_request_bytes: usize,
    maximum_output_tokens: u64,
    maximum_call_cost_micros: u64,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Provider {
    kind: String,
    base_url: String,
    models: Vec<String>,
    pricing: Pricing,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    version: u32,
    instance: String,
    agent: String,
    lease: String,
    name: String,
    instructions: String,
    provider: Provider,
    secret_names: Vec<String>,
    allowed_hosts: Vec<String>,
    ssh_targets: Vec<SshTarget>,
    /// Absent means every tool, the existing approval rule and no extra limits.
    #[serde(default)]
    policy: Option<Box<PolicyDocument>>,
}

/// A validated boot v2 document. It holds no secret.
pub(crate) struct BootV2 {
    document: Document,
    kind: ProviderKind,
    models: Vec<NamespaceId>,
    policy: Box<ManagedPolicy>,
}
impl BootV2 {
    /// Parses and validates one document, refusing anything over the bound,
    /// unknown fields (a `provider.token` among them) and a non-broker endpoint.
    pub(crate) fn parse(bytes: &[u8]) -> Result<Self, RunnerError> {
        if bytes.len() > MAX_BOOT_BYTES {
            return Err(RunnerError::Value("boot document size"));
        }
        let document: Document = serde_json::from_slice(bytes)
            .map_err(|error| RunnerError::Boot(nocterm_core::SharedCause::new(error)))?;
        Self::prepare(document)
    }
    fn prepare(mut document: Document) -> Result<Self, RunnerError> {
        if document.version != BOOT_VERSION {
            return Err(RunnerError::Value("boot.version"));
        }
        check_shape(&Shape {
            identity: [&document.instance, &document.agent, &document.lease],
            name: &document.name,
            secret_names: document.secret_names.len(),
            allowed_hosts: document.allowed_hosts.len(),
            ssh_targets: &document.ssh_targets,
        })?;
        let provider = &document.provider;
        let kind = match provider.kind.as_str() {
            "openai_chat" => ProviderKind::OpenAiChat,
            "anthropic_messages" => ProviderKind::AnthropicMessages,
            _ => return Err(RunnerError::Value("boot.provider")),
        };
        if provider.models.is_empty() || provider.models.len() > super::MAX_ENTRIES {
            return Err(RunnerError::Value("boot.provider"));
        }
        let models = provider
            .models
            .iter()
            .map(|model| {
                NamespaceId::new(model.clone()).map_err(|_| RunnerError::Value("boot.provider"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        // The broker is the only provider endpoint a managed runner may use.
        ProviderEndpoint::broker(&provider.base_url)?;
        let policy = document
            .policy
            .take()
            .map(|document| ManagedPolicy::from_document(*document))
            .transpose()?
            .unwrap_or_default();
        let policy = Box::new(policy);
        let candidate = Self {
            document,
            kind,
            models,
            policy,
        };
        // Refuse unusable ceilings now rather than at the first model call.
        let probe = NamespaceId::new("probe".to_owned())
            .map_err(|_| RunnerError::Value("boot.provider"))?;
        candidate.pricing(&probe, &probe)?;
        Ok(candidate)
    }
    pub(crate) fn kind(&self) -> ProviderKind {
        self.kind
    }
    pub(crate) fn models(&self) -> &[NamespaceId] {
        &self.models
    }
    pub(crate) fn policy(&self) -> &ManagedPolicy {
        &self.policy
    }
    pub(crate) fn endpoint(&self) -> Result<ProviderEndpoint, RunnerError> {
        Ok(ProviderEndpoint::broker(&self.document.provider.base_url)?)
    }
    /// One conservative ceiling per boot model, all under the same selection
    /// provider and pricing revision.
    pub(crate) fn pricing(
        &self,
        provider: &NamespaceId,
        revision: &NamespaceId,
    ) -> Result<Vec<Arc<ConservativeModelPricing>>, RunnerError> {
        let limits = &self.document.provider.pricing;
        self.models
            .iter()
            .map(|model| {
                ConservativeModelPricing::new(
                    ModelSelection::new(provider.clone(), model.clone(), revision.clone()),
                    limits.maximum_request_bytes,
                    limits.maximum_output_tokens,
                    limits.maximum_call_cost_micros,
                )
                .map(Arc::new)
                .map_err(|_| RunnerError::Value("boot.provider.pricing"))
            })
            .collect()
    }
    /// Non-secret report for the line the runner prints once it knows whether
    /// execution is ready.
    pub(crate) fn summary(&self, execution_ready: bool) -> String {
        let doc = &self.document;
        format!(
            "mode=managed\ninstance={}\nagent={}\nlease={}\ninstructions_bytes={}\nmodels={}\nsecret_names={}\nssh_targets={}\nexecution_ready={execution_ready}\n",
            doc.instance,
            doc.agent,
            doc.lease,
            doc.instructions.len(),
            doc.provider.models.len(),
            doc.secret_names.len(),
            doc.ssh_targets.len()
        )
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::managed::Approval;
    fn document() -> serde_json::Value {
        serde_json::json!({
            "version": 2, "instance": "inst1", "agent": "agent1", "lease": "lease1",
            "name": "Scribe", "instructions": "be brief",
            "provider": {"kind": "openai_chat", "base_url": "http://10.0.0.2:8080/v1",
                "models": ["gpt-x"],
                "pricing": {"maximum_request_bytes": 65536, "maximum_output_tokens": 2048,
                    "maximum_call_cost_micros": 50000}},
            "secret_names": ["GITHUB"], "allowed_hosts": ["example.org"],
            "ssh_targets": [{"host": "h", "user": "u", "port": 22}]
        })
    }
    fn read(value: &serde_json::Value) -> Result<BootV2, RunnerError> {
        BootV2::parse(value.to_string().as_bytes())
    }
    #[test]
    fn a_tokenless_v2_document_is_accepted_and_reports_execution_readiness() {
        let boot = read(&document()).unwrap();
        assert_eq!(boot.models().len(), 1);
        assert!(boot.summary(true).contains("execution_ready=true"));
        assert!(boot.summary(false).contains("execution_ready=false"));
    }
    #[test]
    fn a_token_other_versions_and_zero_ceilings_are_refused() {
        let mut token = document();
        token["provider"]["token"] = "nvt1_x".into();
        assert!(read(&token).is_err());
        let mut v1 = document();
        v1["version"] = 1.into();
        assert!(read(&v1).is_err());
        let mut zero = document();
        zero["provider"]["pricing"]["maximum_call_cost_micros"] = 0.into();
        assert!(matches!(
            read(&zero),
            Err(RunnerError::Value("boot.provider.pricing"))
        ));
        let mut public = document();
        public["provider"]["base_url"] = "http://api.example.org/v1".into();
        assert!(read(&public).is_err());
    }
    #[test]
    fn a_document_with_or_without_a_policy_parses_and_a_bad_policy_is_refused() {
        let plain = read(&document()).unwrap();
        assert!(plain.policy().allowed_tools().is_none());
        assert_eq!(plain.policy().approval(), Approval::Ask);
        let mut with = document();
        with["policy"] = serde_json::json!({"tools": {"allowed": ["exec_command"]},
            "approval": "never", "limits": {"max_turns": 3}});
        let boot = read(&with).unwrap();
        assert_eq!(boot.policy().approval(), Approval::Never);
        assert_eq!(boot.policy().allowed_tools().unwrap().len(), 1);
        let mut unknown = with.clone();
        unknown["policy"]["token"] = "x".into();
        assert!(read(&unknown).is_err());
        let mut oversize = document();
        oversize["policy"] = serde_json::json!({"tools": {"allowed": "all"},
            "limits": {"max_turns": 1_000_000}});
        assert!(matches!(
            read(&oversize),
            Err(RunnerError::Value("boot.policy.limits"))
        ));
    }
}
