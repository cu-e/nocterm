//! The canonical text of a public key.
use crate::KeyImportError;
use ssh_key::{HashAlg, PublicKey};
use std::fmt;

/// A public key as `"<algorithm> <base64>"`, without a comment.
///
/// Two keys are the same key exactly when their texts are equal, wherever the
/// text came from (an `authorized_keys` line, a `.pub` file, a vault record).
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PublicKeyText {
    text: String,
    fingerprint: String,
}

impl PublicKeyText {
    /// Reads an OpenSSH public key line and drops its comment.
    pub fn parse(text: &str) -> Result<Self, KeyImportError> {
        let key = PublicKey::from_openssh(text.trim()).map_err(KeyImportError::Malformed)?;
        Self::of(&key)
    }

    pub(crate) fn of(key: &PublicKey) -> Result<Self, KeyImportError> {
        let mut key = key.clone();
        key.set_comment("");
        let line = key.to_openssh().map_err(KeyImportError::Malformed)?;
        Ok(Self {
            text: line.trim_end().to_owned(),
            fingerprint: key.fingerprint(HashAlg::Sha256).to_string(),
        })
    }

    pub fn as_str(&self) -> &str {
        &self.text
    }

    /// The `SHA256:…` fingerprint `ssh-keygen -lf` prints.
    pub fn fingerprint(&self) -> &str {
        &self.fingerprint
    }
}

impl AsRef<str> for PublicKeyText {
    fn as_ref(&self) -> &str {
        &self.text
    }
}

impl fmt::Display for PublicKeyText {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.text)
    }
}

impl fmt::Debug for PublicKeyText {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "PublicKeyText({})", self.text)
    }
}
