//! Boot document versions 2 and 3: the managed document of a runner whose chat tokens
//! arrive on the control channel. Version 2 is boot v1 without `provider.token`, plus
//! the pricing ceilings the runner applies to every model (ADR 0123). Version 3 adds
//! `ssh_egress`, the addresses the host's SSH connections come from, which chat
//! certificate requests state, and per target an optional host key and session settings;
//! version 2 means no egress (ADR 0145). The SSH section is the contract's
//! (`boot_ssh`, contract ADR 0011, ADR 0147) and checked by it.
use super::{
    MAX_BOOT_BYTES, Shape, check_shape,
    policy::{ManagedPolicy, PolicyDocument},
};
use crate::RunnerError;
use nocterm_agent_engine::{ProviderEndpoint, ProviderKind};
use nocterm_cloud_agents_model::ProviderKind as DesktopKind;
use nocterm_cloud_model::{
    identity::NamespaceId,
    inference::{ConservativeModelPricing, ModelSelection},
};
use nocterm_hub_io::runner_control::boot_ssh::{BootSshTarget, check_boot_ssh};
use serde::Deserialize;
use std::sync::Arc;

/// Maps the provider kind a boot document names to the engine's. The contract reads the
/// canonical spelling and the legacy `anthropic` the web panel used to seal.
fn provider_kind(text: &str) -> Result<ProviderKind, RunnerError> {
    match nocterm_hub_io::parse_provider_kind(text) {
        Some(DesktopKind::OpenAiChat) => Ok(ProviderKind::OpenAiChat),
        Some(DesktopKind::AnthropicMessages) => Ok(ProviderKind::AnthropicMessages),
        None => Err(RunnerError::Value("boot.provider")),
    }
}

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
    ssh_targets: Vec<BootSshTarget>,
    /// Absent means every tool, the existing approval rule and no extra limits.
    #[serde(default)]
    policy: Option<Box<PolicyDocument>>,
    /// Boot v3 only: one to eight IP addresses or CIDRs.
    #[serde(default)]
    ssh_egress: Option<Vec<String>>,
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
        check_boot_ssh(
            document.version,
            &document.ssh_targets,
            document.ssh_egress.as_deref(),
        )?;
        check_shape(&Shape {
            identity: [&document.instance, &document.agent, &document.lease],
            name: &document.name,
            secret_names: document.secret_names.len(),
            allowed_hosts: document.allowed_hosts.len(),
            ssh_targets: document.ssh_targets.len(),
        })?;
        let provider = &document.provider;
        let kind = provider_kind(&provider.kind)?;
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
    /// The agent's system prompt, empty when the web panel set none.
    /// The agent this seat runs, as its chat summaries name it.
    pub(crate) fn agent(&self) -> &str {
        &self.document.agent
    }
    /// The SSH targets the host lets the agent reach, as the boot document states them.
    pub(crate) fn ssh_targets(&self) -> &[BootSshTarget] {
        &self.document.ssh_targets
    }
    /// The host's SSH egress; empty for boot v2.
    pub(crate) fn ssh_egress(&self) -> &[String] {
        self.document.ssh_egress.as_deref().unwrap_or_default()
    }
    pub(crate) fn instructions(&self) -> &str {
        &self.document.instructions
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
    use nocterm_hub_io::runner_control::boot_ssh::{BootSshError, BootSshField};
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
    fn the_web_panels_anthropic_kind_is_the_messages_kind_and_unknown_kinds_are_refused() {
        for (text, expected) in [
            ("openai_chat", ProviderKind::OpenAiChat),
            ("anthropic_messages", ProviderKind::AnthropicMessages),
            ("anthropic", ProviderKind::AnthropicMessages),
        ] {
            let mut value = document();
            value["provider"]["kind"] = text.into();
            assert_eq!(read(&value).unwrap().kind(), expected, "{text}");
        }
        let mut unknown = document();
        unknown["provider"]["kind"] = "gemini".into();
        assert!(matches!(
            read(&unknown),
            Err(RunnerError::Value("boot.provider"))
        ));
    }
    #[test]
    fn a_token_other_versions_and_zero_ceilings_are_refused() {
        let mut token = document();
        token["provider"]["token"] = "nvt1_x".into();
        assert!(read(&token).is_err());
        let mut v1 = document();
        v1["version"] = 1.into();
        assert!(read(&v1).is_err());
        let mut v4 = document();
        v4["version"] = 4.into();
        assert!(matches!(
            read(&v4),
            Err(RunnerError::BootSsh(BootSshError::Version))
        ));
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
    fn boot_v2_states_no_egress_and_boot_v3_states_its_addresses() {
        let v2 = read(&document()).unwrap();
        assert!(v2.ssh_egress().is_empty());
        let target = &v2.ssh_targets()[0];
        assert_eq!((target.host.as_str(), target.port), ("h", 22));
        assert!(target.host_key_sha256().is_none());
        let mut v3 = document();
        v3["version"] = 3.into();
        v3["ssh_egress"] = serde_json::json!(["203.0.113.7", "2001:db8::/64"]);
        assert_eq!(
            read(&v3).unwrap().ssh_egress(),
            ["203.0.113.7", "2001:db8::/64"]
        );
    }
    #[test]
    fn egress_in_v2_none_in_v3_and_host_names_or_bad_prefixes_are_refused() {
        let mut v2 = document();
        v2["ssh_egress"] = serde_json::json!(["203.0.113.7"]);
        let mut empty = document();
        empty["version"] = 3.into();
        let mut bad = Vec::new();
        for entry in ["host.example", "10.0.0.0/33", "2001:db8::/129", ""] {
            let mut value = empty.clone();
            value["ssh_egress"] = serde_json::json!([entry]);
            bad.push(value);
        }
        let mut many = empty.clone();
        many["ssh_egress"] = serde_json::json!(vec!["10.0.0.1"; 9]);
        for value in [v2, empty, many].iter().chain(&bad) {
            assert!(matches!(
                read(value),
                Err(RunnerError::BootSsh(BootSshError::Egress))
            ));
        }
    }
    fn v3() -> serde_json::Value {
        let mut value = document();
        value["version"] = 3.into();
        value["ssh_egress"] = serde_json::json!(["203.0.113.7"]);
        value
    }
    fn host_key() -> String {
        nocterm_ssh_keys::KeyMaterial::ed25519_from_seed(&[3; 32], "h")
            .unwrap()
            .public_key()
            .as_str()
            .to_owned()
    }
    #[test]
    fn boot_v3_targets_carry_an_optional_host_key_and_session_settings() {
        let mut keyed = v3();
        keyed["ssh_targets"] = serde_json::json!([
            {"host": "h", "user": "u", "port": 22, "host_public_key": host_key(),
             "session": {"terminal": "xterm", "columns": 100, "rows": 30,
                         "connect_timeout_ms": 5000, "program": "/bin/sh", "args": ["-l"]}},
            {"host": "k", "user": "u", "port": 22}
        ]);
        let boot = read(&keyed).unwrap();
        let [with, without] = boot.ssh_targets() else {
            panic!("two targets");
        };
        let key = nocterm_ssh_keys::PublicKeyText::parse(&host_key()).unwrap();
        assert_eq!(
            with.host_key_sha256(),
            Some(key.fingerprint_bytes().unwrap())
        );
        assert_eq!(with.session_or_default().columns, 100);
        assert_eq!(without.host_key_sha256(), None);
        assert_eq!(without.session_or_default().terminal, "xterm-256color");
    }
    #[test]
    fn host_keys_and_sessions_in_v2_bad_keys_and_duplicate_v3_targets_are_refused() {
        let mut v2 = document();
        v2["ssh_targets"][0]["host_public_key"] = host_key().into();
        let mut bad_key = v3();
        bad_key["ssh_targets"][0]["host_public_key"] = "ssh-ed25519 AAAA".into();
        let mut bad_session = v3();
        bad_session["ssh_targets"][0]["session"] = serde_json::json!({"terminal": "xterm",
            "columns": 1, "rows": 24, "connect_timeout_ms": 1000});
        let mut duplicate = v3();
        duplicate["ssh_targets"] = serde_json::json!([
            {"host": "h", "user": "u", "port": 22}, {"host": "h", "user": "u", "port": 22}
        ]);
        for (value, expected) in [
            (v2, BootSshField::NotInVersion),
            (bad_key, BootSshField::HostKey),
            (bad_session, BootSshField::Session),
            (duplicate, BootSshField::Duplicate),
        ] {
            assert!(
                matches!(
                    read(&value),
                    Err(RunnerError::BootSsh(BootSshError::Target { field, .. })) if field == expected
                ),
                "{expected:?}"
            );
        }
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
