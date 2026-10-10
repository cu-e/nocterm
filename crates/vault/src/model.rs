use nocterm_session::{CredentialId, Target};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
/// The specific request a stored secret may answer. Interactive/MFA answers
/// have no representation here and can never be saved by this API.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum CredentialBinding {
    Password { target: Target },
    KeyPassphrase { path: PathBuf },
}

#[derive(Clone, Debug)]
pub struct CredentialInfo {
    pub id: CredentialId,
    pub label: String,
    pub binding: CredentialBinding,
}
