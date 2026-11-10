use nocterm_session::{CredentialId, SecretRequest, Target};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
/// The specific requests a stored secret may answer. Interactive/MFA answers
/// have no representation here and can never be saved by this API.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "Wire", into = "Wire")]
pub enum CredentialBinding {
    /// A password, answering only the addresses it is attached to, in the order they were
    /// attached. One kept from a prompt answers its own address; a password saved under a
    /// name may answer several machines that share it, or none yet.
    Password {
        targets: Vec<Target>,
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

/// The stored form: a password attached to one address is written as `target`, as before
/// passwords could be shared, so every client reads it; any other as `targets`.
#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Wire {
    Password {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        target: Option<Target>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        targets: Option<Vec<Target>>,
    },
    KeyPassphrase {
        path: PathBuf,
    },
    PrivateKey {
        public_key: String,
    },
}

/// A stored binding names a password's addresses once, either way.
#[derive(Debug, thiserror::Error)]
#[error("a password binding names its addresses as `target` or `targets`, once")]
struct BindingFormatError;

impl TryFrom<Wire> for CredentialBinding {
    type Error = BindingFormatError;
    fn try_from(wire: Wire) -> Result<Self, Self::Error> {
        Ok(match wire {
            Wire::Password {
                target: Some(target),
                targets: None,
            } => Self::Password {
                targets: vec![target],
            },
            Wire::Password {
                target: None,
                targets: Some(targets),
            } => Self::password(targets),
            Wire::Password { .. } => return Err(BindingFormatError),
            Wire::KeyPassphrase { path } => Self::KeyPassphrase { path },
            Wire::PrivateKey { public_key } => Self::PrivateKey { public_key },
        })
    }
}

impl From<CredentialBinding> for Wire {
    fn from(binding: CredentialBinding) -> Self {
        match binding {
            CredentialBinding::Password { mut targets } if targets.len() == 1 => Self::Password {
                target: targets.pop(),
                targets: None,
            },
            CredentialBinding::Password { targets } => Self::Password {
                target: None,
                targets: Some(targets),
            },
            CredentialBinding::KeyPassphrase { path } => Self::KeyPassphrase { path },
            CredentialBinding::PrivateKey { public_key } => Self::PrivateKey { public_key },
        }
    }
}

impl CredentialBinding {
    /// A password answering `targets`, each once, in their first order.
    pub fn password(targets: impl IntoIterator<Item = Target>) -> Self {
        let mut unique: Vec<Target> = Vec::new();
        for target in targets {
            if !unique.contains(&target) {
                unique.push(target);
            }
        }
        Self::Password { targets: unique }
    }

    /// The vault key a request's answer is stored under. One-time answers have
    /// none, so the vault can never hold them; a key is created in the profile
    /// editor, never from a prompt.
    pub fn for_request(request: &SecretRequest) -> Option<CredentialBinding> {
        match request {
            SecretRequest::Password { target, .. } => Some(CredentialBinding::Password {
                targets: vec![target.clone()],
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

    /// Whether a record bound so answers a request bound as `request`: a password when the
    /// requested address is among its own, anything else only for the same binding.
    pub fn answers(&self, request: &CredentialBinding) -> bool {
        match (self, request) {
            (Self::Password { targets }, Self::Password { targets: asked }) => {
                !asked.is_empty() && asked.iter().all(|target| targets.contains(target))
            }
            (own, asked) => own == asked,
        }
    }

    /// Whether a record bound so may be bound as `other` while its secret stays: a password
    /// may answer other addresses; a key passphrase or a private key stays bound to its own.
    pub fn may_become(&self, other: &CredentialBinding) -> bool {
        match (self, other) {
            (Self::Password { .. }, Self::Password { .. }) => true,
            (own, other) => own == other,
        }
    }

    /// The addresses a password answers; none for any other record.
    pub fn targets(&self) -> &[Target] {
        match self {
            Self::Password { targets } => targets,
            Self::KeyPassphrase { .. } | Self::PrivateKey { .. } => &[],
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
            Some(CredentialBinding::Password {
                targets: vec![target]
            })
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

    #[test]
    fn a_password_attached_to_one_address_is_stored_as_before() {
        let one = CredentialBinding::password([Target::new("ann", "a.test", 22)]);
        let text = serde_json::to_value(&one).unwrap();
        assert_eq!(
            text,
            serde_json::json!({"kind": "password", "target": {"host": "a.test", "user": "ann"}})
        );
        assert_eq!(
            serde_json::from_value::<CredentialBinding>(text).unwrap(),
            one
        );
        let shared = CredentialBinding::password([
            Target::new("ann", "a.test", 22),
            Target::new("ann", "b.test", 2222),
            Target::new("ann", "a.test", 22),
        ]);
        let text = serde_json::to_value(&shared).unwrap();
        assert_eq!(text["targets"].as_array().unwrap().len(), 2);
        assert_eq!(
            serde_json::from_value::<CredentialBinding>(text).unwrap(),
            shared
        );
        let none = CredentialBinding::password([]);
        assert_eq!(
            serde_json::to_value(&none).unwrap(),
            serde_json::json!({"kind": "password", "targets": []})
        );
        for invalid in [
            serde_json::json!({"kind": "password"}),
            serde_json::json!({"kind": "password", "target": {"host": "a", "user": "u"}, "targets": []}),
            serde_json::json!({"kind": "password", "targets": [], "other": 1}),
        ] {
            assert!(serde_json::from_value::<CredentialBinding>(invalid).is_err());
        }
    }

    #[test]
    fn a_shared_password_answers_only_its_own_addresses() {
        let (a, b, c) = (
            Target::new("ann", "a.test", 22),
            Target::new("ann", "b.test", 22),
            Target::new("ann", "c.test", 22),
        );
        let shared = CredentialBinding::password([a.clone(), b.clone()]);
        let asked = |target: &Target| CredentialBinding::password([target.clone()]);
        assert!(shared.answers(&asked(&a)));
        assert!(shared.answers(&asked(&b)));
        assert!(!shared.answers(&asked(&c)));
        assert!(!CredentialBinding::password([]).answers(&asked(&a)));
        assert!(shared.may_become(&asked(&c)));
        let key = CredentialBinding::PrivateKey {
            public_key: "ssh-ed25519 A".into(),
        };
        assert!(!key.may_become(&CredentialBinding::PrivateKey {
            public_key: "ssh-ed25519 B".into()
        }));
        assert!(!shared.may_become(&key));
        assert!(!key.answers(&asked(&a)));
    }
}
