use nocterm_session::{CredentialId, Target};
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

/// The organization group a credential belongs to, as the signed-in member may use it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CredentialGroup {
    /// What the user calls the organization.
    pub name: String,
    /// Whether the member may change or delete the credential; a guest only uses it.
    pub writable: bool,
}

#[derive(Clone, Debug)]
pub struct CredentialInfo {
    pub id: CredentialId,
    pub label: String,
    pub binding: CredentialBinding,
    /// `None` for a credential in the user's own vault.
    pub group: Option<CredentialGroup>,
}
