//! Closed inverse of the runner's public setup document, never private config.
use super::{CloudRunnerSetup, CloudTrustError, RunnerRegistration};
use crate::cloud::ChatMaterialPins;
use nocterm_cloud_model::{
    identity::{
        AccountIdentity, AccountScope, Fingerprint, HubIdentity, NamespaceId, RunnerId,
        RunnerKeyEpoch,
    },
    journal::JournalCatalogBinding,
};
use nocterm_hubs_model::HubAddress;
use serde::Deserialize;
use sha2::{Digest as _, Sha256};

const MAX_BYTES: usize = 4096;
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PublicDocument {
    format: String,
    hub_address: String,
    raw_hub_id: String,
    hub_namespace: String,
    hub_authority: String,
    account_id: String,
    account_key_fingerprint: String,
    runner: String,
    runner_key_epoch: u64,
    encryption_key: String,
    signing_key: String,
    proof_key: String,
}
pub(super) fn decode(bytes: &[u8], label: String) -> Result<CloudRunnerSetup, CloudTrustError> {
    if bytes.len() > MAX_BYTES {
        return Err(CloudTrustError::Capacity);
    }
    let text = std::str::from_utf8(bytes).map_err(|_| CloudTrustError::Format)?;
    let document: PublicDocument = toml::from_str(text).map_err(|_| CloudTrustError::Format)?;
    if document.format != "nocterm-cloud-runner-setup-v1" {
        return Err(CloudTrustError::Format);
    }
    let authority = hex(&document.hub_authority)?;
    let scope = AccountScope::new(
        HubIdentity::new(
            NamespaceId::new(document.hub_namespace).map_err(|_| CloudTrustError::Format)?,
            Fingerprint::from_bytes(Sha256::digest(authority).into()),
        ),
        AccountIdentity::new(
            NamespaceId::new(document.account_id).map_err(|_| CloudTrustError::Format)?,
            Fingerprint::from_bytes(hex(&document.account_key_fingerprint)?),
        ),
    );
    let runner = RunnerRegistration::new(
        JournalCatalogBinding::new(
            scope,
            RunnerId::from_bytes(hex(&document.runner)?),
            RunnerKeyEpoch::new(document.runner_key_epoch).map_err(|_| CloudTrustError::Format)?,
        ),
        ChatMaterialPins {
            encryption: hex(&document.encryption_key)?,
            signing: hex(&document.signing_key)?,
            proof: hex(&document.proof_key)?,
        },
        label,
    )
    .map_err(|_| CloudTrustError::Format)?;
    CloudRunnerSetup::new(
        HubAddress::parse(&document.hub_address)
            .map_err(|_| CloudTrustError::Format)?
            .into(),
        document.raw_hub_id,
        authority,
        runner,
    )
}
fn hex<const N: usize>(text: &str) -> Result<[u8; N], CloudTrustError> {
    // This exported setup format is canonical lowercase; config and seed readers
    // independently retain their existing case-insensitive input policy.
    if !text
        .bytes()
        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(CloudTrustError::Format);
    }
    let mut decoded = [0; N];
    hex::decode_to_slice(text, &mut decoded).map_err(|_| CloudTrustError::Format)?;
    Ok(decoded)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> String {
        let authority = [15; 32];
        let digest = Sha256::digest(authority).into();
        let namespace = nocterm_hub_model::cloud::hub_namespace_id("instance", digest).unwrap();
        format!(
            "format = \"nocterm-cloud-runner-setup-v1\"\nhub_address = \"https://hub.test:443\"\nraw_hub_id = \"instance\"\nhub_namespace = \"{}\"\nhub_authority = \"{}\"\naccount_id = \"account\"\naccount_key_fingerprint = \"{}\"\nrunner = \"{}\"\nrunner_key_epoch = 1\nencryption_key = \"{}\"\nsigning_key = \"{}\"\nproof_key = \"{}\"\n",
            namespace.as_str(),
            "0f".repeat(32),
            "01".repeat(32),
            "02".repeat(16),
            "03".repeat(32),
            "04".repeat(32),
            "05".repeat(32)
        )
    }

    #[test]
    fn public_setup_keeps_lowercase_hex_as_a_closed_input_format() {
        let original = fixture();
        assert!(decode(original.as_bytes(), "Runner".into()).is_ok());
        for replacement in [
            "0F".repeat(32),
            "+f".repeat(32),
            "0f".repeat(31),
            "é".repeat(32),
        ] {
            let changed = original.replace(&"0f".repeat(32), &replacement);
            assert!(matches!(
                decode(changed.as_bytes(), "Runner".into()),
                Err(CloudTrustError::Format)
            ));
        }
    }
    #[test]
    fn public_setup_import_keeps_full_assignment_and_refuses_private_or_moved_fields() {
        let fixture = fixture();
        let setup = decode(fixture.as_bytes(), "Runner".into()).unwrap();
        assert_eq!(
            setup.runner().binding().runner(),
            RunnerId::from_bytes([2; 16])
        );
        assert_eq!(setup.runner().pins().proof, [5; 32]);
        assert!(matches!(
            decode(
                format!("{fixture}state = \"private\"\n").as_bytes(),
                "Runner".into()
            ),
            Err(CloudTrustError::Format)
        ));
        assert!(matches!(
            decode(
                fixture
                    .replace("raw_hub_id = \"instance\"", "raw_hub_id = \"other\"")
                    .as_bytes(),
                "Runner".into()
            ),
            Err(CloudTrustError::Binding)
        ));
        assert!(matches!(
            decode(
                fixture
                    .replace("runner_key_epoch = 1", "runner_key_epoch = 0")
                    .as_bytes(),
                "Runner".into()
            ),
            Err(CloudTrustError::Format)
        ));
        assert!(matches!(
            decode(&[b' '; MAX_BYTES + 1], "Runner".into()),
            Err(CloudTrustError::Capacity)
        ));
    }
}
