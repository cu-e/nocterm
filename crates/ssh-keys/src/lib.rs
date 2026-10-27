//! Rules of SSH key material: import, normalization, public half and fingerprint.
//! Owns: OpenSSH private and public key formats, the accepted algorithms, the
//! canonical public key text, and the `VaultSshKeys` port the connection editor uses.
//! Does not own: entropy, files, the vault, SSH connections or any view.
//! Ports provided: VaultSshKeys (implemented by the application over the vault).
//! Ports used: none. Settings: none. Globals: none.
//! Invariants: private key material is held in `Secret`/`Zeroizing` and never
//! formatted; an imported key is stored unencrypted only inside the vault;
//! generation takes its seed from the caller.
//!
//! The `ssh-key` version is pinned to the one `russh` resolves, so the text a
//! [`KeyMaterial`] produces is read back by the SSH adapter without conversion.

mod error;
mod material;
mod port;
mod public;

pub use error::{KeyImportError, VaultSshKeysError};
pub use material::{KeyMaterial, MIN_RSA_BITS};
pub use port::{KeySource, NoVaultSshKeys, StoredKey, VaultSshKeys};
pub use public::PublicKeyText;

#[cfg(any(test, feature = "test-support"))]
pub mod testing;

#[cfg(test)]
mod tests;
