# 0102. SSH keys are kept in the vault; the editor receives them through `VaultSshKeys`

Status: accepted

## Context

A profile could name a private key only as a path on this machine, so a key did not follow the
user to another device and the vault, which already protects passwords and key passphrases
([ADR 0016](0016-vault-storage-boundary.md)), could not hold the key itself. Termius keeps keys in
its vault and lets a host refer to one. The rules of an SSH key (formats, algorithms, public half,
fingerprint) belong to neither `nocterm-session` nor `nocterm-vault`, and a later hub agent must sign
with the same keys without moving code.

## Decision

**The rules of a key are a domain crate.** `nocterm-ssh-keys` owns import of OpenSSH private keys
(Ed25519, ECDSA P-256/384/521, RSA of at least 2048 bits; DSA and `sk-*` keys are refused),
removal of an encryption passphrase, the canonical public key text (`<algorithm> <base64>`, no
comment), the SHA-256 fingerprint and the `VaultSshKeys` port. It performs no I/O and draws no
entropy: generation takes its seed from the caller. Its `ssh-key` version is the one `russh`
resolves, so the stored text is read by the adapter without conversion. Private material is held in
`Secret` and is never formatted.

**A key is a vault credential.** `CredentialBinding::PrivateKey { public_key }` binds the record to
the canonical public half; the secret is the unencrypted OpenSSH private key with LF line endings.
It is stored unencrypted because the vault envelope is the protection. A profile refers to it with
`Auth::VaultKey { credential, public_key }` (`method = "vault_key"`); the profile carries no secret,
and a key chosen in the editor is a reference, not a copy.

**Signing in asks the vault, never a file.** The terminal answers `SecretRequest::PrivateKey` only
from the credential store by the credential id, waits for the vault to unlock and answers nothing
when the record is missing. The adapter compares the public half of the record with the profile
before it signs, and skips a record that was replaced. RSA signs with SHA-256 or SHA-512 only;
a server that offers only `ssh-rsa` (SHA-1) is skipped. A `PrivateKey` request is not storable, so
Remember never creates or overwrites one, and it is not shown to agents.

**The editor gets keys through a narrow port.** `VaultSshKeys` lists, generates and imports keys.
`src/vault_keys.rs` implements it over `VaultAccess` with entropy from the OS, and bootstrap opens
the vault before the connections feature so the port can be injected; windows without a vault use
`NoVaultSshKeys`. "Move to vault" and "Import" read the file within bounds, store the key and
switch the profile to the vault key.

## Consequences

- Keys can be shared by any feature that holds the port and by the hub agent, which adds signing
  to the same crate.
- A key file on disk stays valid as before (`Auth::KeyFile`); importing never deletes it.
- The vault format gains one binding kind (`private_key`), described in [VAULT.md](../VAULT.md).
