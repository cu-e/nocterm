//! Explicit public CLI allowlist. Private configuration/state are never serialized.
use crate::RunnerError;
use nocterm_cloud_model::journal::JournalCatalogBinding;
use nocterm_hub_io::cloud::RunnerPublicIdentity;
use nocterm_hubs_model::HubAddress;
use serde::Serialize;

const MAX_PUBLIC_BYTES: usize = 4 * 1024;
#[derive(Clone)]
pub(crate) struct PublicSetup {
    origin: String,
    raw_hub: String,
    authority: [u8; 32],
    binding: JournalCatalogBinding,
}
impl PublicSetup {
    pub(crate) fn new(
        origin: &HubAddress,
        raw_hub: &str,
        authority: [u8; 32],
        binding: JournalCatalogBinding,
    ) -> Result<Self, RunnerError> {
        if raw_hub.is_empty() || raw_hub.len() > 128 {
            return Err(RunnerError::Value("hub.instance"));
        }
        Ok(Self {
            origin: origin.to_string(),
            raw_hub: raw_hub.to_owned(),
            authority,
            binding,
        })
    }
    pub(crate) fn encode(&self, identity: &RunnerPublicIdentity) -> Result<String, RunnerError> {
        if identity.binding() != &self.binding {
            return Err(RunnerError::Value("setup identity"));
        }
        self.require_pin(identity)?;
        let scope = self.binding.scope();
        let document = PublicDocument {
            format: "nocterm-cloud-runner-setup-v1",
            hub_address: &self.origin,
            raw_hub_id: &self.raw_hub,
            hub_namespace: scope.hub().instance().as_str(),
            hub_authority: hex::encode(self.authority),
            account_id: scope.account().id().as_str(),
            account_key_fingerprint: hex::encode(scope.account().key().as_bytes()),
            runner: hex::encode(self.binding.runner().as_bytes()),
            runner_key_epoch: self.binding.epoch().get(),
            encryption_key: hex::encode(identity.encryption_key()),
            signing_key: hex::encode(identity.signing_key()),
            proof_key: hex::encode(identity.proof_key()),
        };
        // Every string is pre-bounded above/by native identity constructors.
        // TOML escapes those fixed inputs; no secret-bearing value is borrowed.
        let bytes = toml::to_string(&document)
            .map_err(|error| RunnerError::Configuration(nocterm_core::SharedCause::new(error)))?;
        if bytes.len() > MAX_PUBLIC_BYTES {
            return Err(RunnerError::Value("public setup size"));
        }
        Ok(bytes)
    }
    fn require_pin(&self, identity: &RunnerPublicIdentity) -> Result<(), RunnerError> {
        use nocterm_hub_io::cloud::{ChatMaterialPins, CloudRunnerSetup, RunnerRegistration};
        let runner = RunnerRegistration::new(
            self.binding.clone(),
            ChatMaterialPins {
                encryption: *identity.encryption_key(),
                signing: *identity.signing_key(),
                proof: *identity.proof_key(),
            },
            hex::encode(self.binding.runner().as_bytes()),
        )
        .map_err(|_| RunnerError::Value("setup runner"))?;
        let origin =
            HubAddress::parse(&self.origin).map_err(|_| RunnerError::Value("setup origin"))?;
        CloudRunnerSetup::new(origin.into(), self.raw_hub.clone(), self.authority, runner)
            .map(|_| ())
            .map_err(|_| RunnerError::Value("setup authority"))
    }
}
#[derive(Serialize)]
struct PublicDocument<'a> {
    format: &'static str,
    hub_address: &'a str,
    raw_hub_id: &'a str,
    hub_namespace: &'a str,
    hub_authority: String,
    account_id: &'a str,
    account_key_fingerprint: String,
    runner: String,
    runner_key_epoch: u64,
    encryption_key: String,
    signing_key: String,
    proof_key: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use nocterm_cloud_model::identity::{RunnerId, RunnerKeyEpoch};
    use nocterm_hub_io::cloud::{CloudRunnerIdentity, RunnerHubTrust};
    use nocterm_hubs_model::{HubEndpointSpec, TrustPolicy};

    #[test]
    fn public_setup_exports_only_complete_bound_public_metadata() {
        let address = HubAddress::parse("hub.test").unwrap();
        let trust = RunnerHubTrust::new(
            HubEndpointSpec {
                address: address.clone(),
                trust: TrustPolicy::PublicCa,
            },
            "normal-hub-id".into(),
            [5; 32],
        );
        let binding = trust
            .binding(
                "account",
                [3; 32],
                RunnerId::from_bytes([7; 16]),
                RunnerKeyEpoch::new(1).unwrap(),
            )
            .unwrap();
        let directory = tempfile::tempdir().unwrap();
        let identity =
            CloudRunnerIdentity::create_new(&directory.path().join("identity"), binding.clone())
                .unwrap();
        let public = identity.public_identity().unwrap();
        let setup = PublicSetup::new(&address, "normal-hub-id", [5; 32], binding.clone()).unwrap();
        let output = setup.encode(&public).unwrap();
        let parsed: toml::Value = toml::from_str(&output).unwrap();
        let mut fields = parsed
            .as_table()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>();
        fields.sort();
        assert_eq!(
            fields,
            [
                "account_id",
                "account_key_fingerprint",
                "encryption_key",
                "format",
                "hub_address",
                "hub_authority",
                "hub_namespace",
                "proof_key",
                "raw_hub_id",
                "runner",
                "runner_key_epoch",
                "signing_key"
            ]
        );
        assert_eq!(
            parsed["hub_namespace"].as_str(),
            Some(binding.scope().hub().instance().as_str())
        );
        assert_eq!(
            parsed["encryption_key"].as_str(),
            Some(hex::encode(public.encryption_key()).as_str())
        );
        assert_eq!(
            parsed["hub_authority"].as_str(),
            Some(hex::encode([5; 32]).as_str())
        );
        assert!(output.len() <= MAX_PUBLIC_BYTES);
        assert!(
            PublicSetup::new(&address, "normal-hub-id", [6; 32], binding.clone())
                .unwrap()
                .encode(&public)
                .is_err()
        );
        assert!(PublicSetup::new(&address, &"x".repeat(129), [5; 32], binding.clone()).is_err());
        let other = trust
            .binding("other-account", [3; 32], binding.runner(), binding.epoch())
            .unwrap();
        assert!(
            PublicSetup::new(&address, "normal-hub-id", [5; 32], other)
                .unwrap()
                .encode(&public)
                .is_err()
        );
    }
}
