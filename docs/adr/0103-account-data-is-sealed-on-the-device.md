# 0103. Account data is sealed on the device; one chosen server keeps the vault and the profiles

Status: accepted

## Context

A hub account can follow the user to another device ([ADR 0101](0101-profiles-synchronize-through-a-replica-beside-the-local-document.md)
replicates profiles, [ADR 0102](0102-ssh-keys-are-kept-in-the-vault.md) keeps SSH keys in the
vault). What a server may see of that data is a security decision of its own: a vault on a
server the user does not run must not expose a credential to its operator, and a server that
forges or reorders records must not make the client overwrite local data silently.

## Decision

**The hub keeps ciphertext only.** The keys come from the account's login password (or from a
passphrase of its own for an account that signs in through a provider) and are derived on the
device by the hub contract's `nocterm-keys`. A random keyset is created on the first sign-in of
an account and stored sealed on the hub; a later device opens it with the same secret. Each
collection (the vault, the profiles) has its own key, so a record of one cannot be read as the
other. The device keeps no key on disk.

**Opening and recovery are explicit.** Sign-in answers with the status of the keys: open, created
(the recovery key is shown once and never stored), a passphrase needed, or the recovery key needed.
The recovery key is a plain `Secret`; a wrong one is `RecoveryKeyInvalid`, a keyset the hub damaged
is `KeysetDamaged`. Locking the vault and signing out zero the keys in memory.

**Only ciphertext on disk, besides public facts.** The adapter caches the sealed vault records in
a private file (mode 0600, replaced atomically through `nocterm_core::fs::write_private_atomic`) so
credentials read offline. An offline write is refused with `Offline`; nothing is queued in clear.
The same file keeps non-secret facts: how the unlock key is derived, whether the recovery key was
confirmed, and the pinned administrator keys below, which are public keys stored in the clear.

**Administrators are trusted on first use, with a fingerprint.** A hub names the account that
granted a group's collection key and publishes that account's public key, so it could substitute
its own and read what the group shares. The device therefore opens a grant only under an
administrator key it pinned after a person compared its fingerprint with the one the administrator
sees in `hubctl`. A group whose granter is not pinned is left out of the layout and reported as
pending, with the fingerprint to compare; a key that differs from the pinned one is reported the
same way, marked as a change, and replaces the pin only when confirmed again. Nothing confirms a
key silently. The application lists the pending keys and records the decision through the
`HubAdminKeys` port of `nocterm-hubs-model` (shown on the Servers page); confirming raises the keys
notice, so the profile feed and the vault read the groups again at once. Pins are kept in the
account cache, so they outlive a restart, and are erased with the account when the device signs out
of the server.

**A forgery is an error.** A record the hub presents that does not authenticate (changed
ciphertext, or a record moved to another collection or identity) is the hub's forgery: it is
reported as an error, the change feed does not move past it, and it never replaces or removes local
data.
Unknown collections and unknown record kinds are left exactly as they are: they are never listed,
rewritten or deleted ([ADR 0101](0101-profiles-synchronize-through-a-replica-beside-the-local-document.md)).
Organization collections are `Forbidden` until their keys exist.

**One server is the account.** `[hubs] account` names the server whose account keeps this device's
vault and profiles. The application holds a `SelectedVault` that delegates to the vault of this
device or the account's; choosing another server (or none) locks the vault it replaces and
attaches or detaches the profile replication, without a restart. The agreed replication state is a
private file of the state directory that goes through the shared persistence service. The adapter
does not know the application's settings: `HubLinks::open_account` returns the replica and the
vault, and `src/hub_account.rs` composes them.

## Consequences

- A server operator learns the size, the number and the times of records, not their content. It
  cannot make the device open a group's key under a key the person did not confirm.
- Until a person confirms an administrator's key, that organization's groups stay hidden; a changed
  key hides them again until it is confirmed.
- Losing both the password and the recovery key loses the account's vault and replicated profiles.
- A revoked device erases its cached ciphertext and zeroes its keys, as signing out does.
- A recovery key that was shown and not confirmed is flagged in the cache. While the flag stands
  and the keys are open, a new key can be issued: it replaces the recovery envelope with a keyset
  write conditional on the version, so the old key stops working.
- The hub contract owns the cryptography; this repository adds no primitive of its own.
