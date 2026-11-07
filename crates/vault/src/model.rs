use nocterm_session::{CredentialId, SecretRequest, Target};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
/// The specific request a stored secret may answer. Interactive/MFA answers
/// have no representation here and can never be saved by this API.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum CredentialBinding {
    Password {
        target: Target,
    },
    KeyPassphrase {
        path: PathBuf,
    },
    /// A private key kept in the vault, bound to the canonical text of its public
    /// half (`"<algorithm> <base64>"`). A profile that names the record must name
    /// the same text, so a record never answers for a different key.
    PrivateKey {
        public_key: String,
    },
}

impl CredentialBinding {
    /// The vault key a request's answer is stored under. One-time answers have
    /// none, so the vault can never hold them; a key is created in the profile
    /// editor, never from a prompt.
    pub fn for_request(request: &SecretRequest) -> Option<CredentialBinding> {
        match request {
            SecretRequest::Password { target, .. } => Some(CredentialBinding::Password {
                target: target.clone(),
            }),
            SecretRequest::KeyPassphrase { path, .. } => {
                Some(CredentialBinding::KeyPassphrase { path: path.clone() })
            }
            SecretRequest::PrivateKey { public_key, .. } => Some(CredentialBinding::PrivateKey {
                public_key: public_key.clone(),
            }),
            SecretRequest::Interactive { .. } => None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct CredentialInfo {
    pub id: CredentialId,
    pub label: String,
    pub binding: CredentialBinding,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_binding_preserves_the_exact_target_path_or_public_key() {
        let target = Target::new("ann", "host.test", 2222);
        assert_eq!(
            CredentialBinding::for_request(&SecretRequest::Password {
                target: target.clone(),
                retry: true
            }),
            Some(CredentialBinding::Password { target })
        );
        let path = PathBuf::from("/keys/exact-key");
        assert_eq!(
            CredentialBinding::for_request(&SecretRequest::KeyPassphrase {
                path: path.clone(),
                retry: false
            }),
            Some(CredentialBinding::KeyPassphrase { path })
        );
        assert_eq!(
            CredentialBinding::for_request(&SecretRequest::PrivateKey {
                credential: CredentialId::from_bytes([1; 16]),
                public_key: "ssh-ed25519 exact".into()
            }),
            Some(CredentialBinding::PrivateKey {
                public_key: "ssh-ed25519 exact".into()
            })
        );
        assert_eq!(
            CredentialBinding::for_request(&SecretRequest::Interactive {
                prompt: "MFA".into(),
                echo: false
            }),
            None
        );
    }
}
