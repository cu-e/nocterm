//! Rules of SSH key material: import, normalization, public half and fingerprint.
//! Owns: OpenSSH private and public key formats, the accepted algorithms, the
//! canonical public key text, and the `VaultSshKeys` port the connection editor uses.
//! Owns: the `authorized_keys` access ports and line rules ([`authorized_keys`]), through which
//! the hub's agent key line is written to and removed from a saved server.
//! Does not own: consent, entropy, files, the vault, SSH connections or any view.
//! Ports provided: VaultSshKeys (implemented by the application over the vault);
//! AuthorizedKeysFile, AgentHosts and ConnectedHosts (implemented by `nocterm-ssh`).
//! Ports used: none. Settings: none. Globals: none.
//! Invariants: private key material is held in `Secret`/`Zeroizing` and never
//! formatted; an imported key is stored unencrypted only inside the vault;
//! generation takes its seed from the caller.
//!
//! The `ssh-key` version is pinned to the one `russh` resolves, so the text a
//! [`KeyMaterial`] produces is read back by the SSH adapter without conversion.

pub mod authorized_keys;
mod error;
mod material;
mod port;
mod public;

pub use error::{KeyImportError, VaultSshKeysError};
pub use material::{KeyMaterial, MIN_RSA_BITS, ed25519_public_key};
pub use port::{KeySource, NoVaultSshKeys, StoredKey, VaultSshKeys};
pub use public::PublicKeyText;

#[cfg(any(test, feature = "test-support"))]
pub mod testing;

#[cfg(test)]
mod tests;
