# Credential vault

The portable vault is `vault.bin` in the application configuration directory. It
requires no OS keychain. Back up the encrypted file and retain the master password
separately; a forgotten master password cannot be recovered. No cloud provider,
account system or plaintext export is implemented. A future storage provider must
handle authenticated ciphertext and preserve conflict detection, without receiving
secrets or decrypted metadata.

Open Settings → Vault, or Credential Vault in the `+` menu (which selects the same
Settings page). Create requires confirmation; unlock,
explicit lock, automatic lock, credential deletion and changing the master password
are supported. Default auto-lock is configured in Settings. Activity is vault
operations, not arbitrary keyboard activity. Profiles/recents store opaque IDs;
credential labels, bindings and secrets live inside the encrypted payload.
Leaving the Vault page or closing Settings clears its password fields and undo
history without locking the service. Lock is explicit or automatic; other draft
settings survive page changes. Refresh reloads credential labels saved by other
features since the page was opened.

Remember is an explicit authentication choice. Only successful SSH authentication
stores a password or private-key passphrase, bound to its target or key path.
Interactive/MFA answers have no vault record type. A saved credential is used for
the initial matching request; rejected credentials return to manual entry instead
of being silently retried. Unlocking can supply the still-pending initial request.

## Format and persistence

Version 1 derives a 32-byte wrapping key with Argon2id (64 MiB, three passes, four
lanes, fresh 16-byte salt). It wraps a random 32-byte data key with
XChaCha20-Poly1305. The data key encrypts a bounded binary payload with fresh
24-byte nonces. The fixed header authenticates format, KDF/cipher identifiers,
parameters and lengths; the payload also authenticates the wrapped key. OS
randomness is required. Unsupported KDF parameters, malformed lengths and excessive
sizes are rejected before expensive allocation. Maximum payload is 1 MiB / 1024
credentials. Password changes replace the wrapping material without changing IDs.

Writes use a private atomic temporary file, a sibling `vault.bin.lock` exclusive
lock and a revision comparison. A competing writer must lock/unlock to reload
before saving. Failed writes preserve the previous file and in-memory state.
Unix vault/temporary files have mode 0600; Windows relies on the user's configuration
directory ACLs. Moving ciphertext between machines preserves the envelope;
keep its companion lock available while an instance is using that directory.

## Security boundaries

The single bounded worker keeps KDF and disk I/O off the UI thread. Lock immediately
invalidates the service epoch and queued/in-flight results. An already-running
Argon2 operation cannot be interrupted; its keys and buffers are erased when that
bounded operation finishes. Domain secrets and KDF buffers use secrecy/zeroize;
Debug output is redacted. Only encrypted revisions are retained for comparison.

GPUI's password widget uses shared strings and undo storage. Inputs and undo history
are cleared on submission/lock, but the widget cannot guarantee erasure of every
previous allocation. This is not protection against a compromised running process,
a debugger, OS swap or physical memory access. The application does not claim an
independent cryptographic audit. Review of this implementation is separate from
audit claims about the underlying RustCrypto primitives.

Tests cover wrong passwords, header/ciphertext tampering, hostile lengths/KDFs,
private Unix permissions, failed/cancelled writes, concurrent writers, rotation,
auto-lock and authentication/MFA boundaries. Cross-platform file ACL handling and
an external security audit remain additional verification work.
