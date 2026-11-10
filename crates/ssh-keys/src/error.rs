//! Typed failures of key import and of the vault key port.
use nocterm_core::error::SharedCause;

/// Why key text could not become usable key material.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum KeyImportError {
    #[error("The text is not an OpenSSH key.")]
    Malformed(#[source] ssh_key::Error),
    #[error("The key is protected by a passphrase.")]
    PassphraseRequired,
    #[error("The passphrase is incorrect.")]
    WrongPassphrase,
    #[error("{algorithm} keys are not supported.")]
    Unsupported { algorithm: String },
    #[error("RSA keys of {bits} bits are too weak; use at least 2048.")]
    TooWeak { bits: u32 },
}

/// Why the vault could not list, create or import a key.
#[derive(Clone, Debug, thiserror::Error)]
pub enum VaultSshKeysError {
    #[error("The vault is locked.")]
    Locked,
    #[error("The vault is offline.")]
    Offline,
    #[error("No vault is available.")]
    Unavailable,
    #[error(transparent)]
    Key(#[from] KeyImportError),
    #[error("Could not read the key file.")]
    Read(#[source] SharedCause),
    #[error("Could not get randomness for a new key.")]
    Entropy(#[source] SharedCause),
    #[error("The vault refused the key.")]
    Vault(#[source] SharedCause),
}
