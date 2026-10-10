# 0016. Pure vault transitions and explicit storage adapter injection

## Context

The vault domain mixed cryptographic rules with filesystem publication, operating
system entropy, lock files, native registration orchestration and a background
worker. UI globals hid its collaborators. This prevented domain I/O checks from
being enforced and required UI tests to reach into private state.

## Decision

`nocterm-vault` owns authenticated format codecs, credential binding rules,
`VaultState` and prepared transactions. Explicit entropy values produce candidates
without changing accepted state. `PreparedWrite` carries encrypted bytes, expected
revision and replacement state; its acceptance is infallible after publication.
Prepared unlocks keep keys secret until the caller accepts them. Native release
and UI/service access are typed domain contracts.

`nocterm-vault-store` is an adapter. It obtains OS entropy, owns bounded file reads,
exclusive mutation locks, serial worker admission, epochs and native registration
orchestration. It compares revisions under the lock, stages via the shared private
publication primitive, checks cancellation before rename, and accepts state only
when publication succeeds. The lock spans publication. Device registration and
release remain bound to the current encrypted envelope. Cancellation invalidates
queued and in-flight results; native scopes still own their watchdog teardown.
The encrypted envelope and opaque credential ID wire formats remain unchanged.
`CredentialId::from_bytes` is pure; adapters supply entropy or explicit test bytes.

The app constructs the storage adapter and passes `Arc<dyn VaultAccess>` through
vault feature `Deps`. `install` returns an owned `Installed` handle. Settings page
factories, actions, credential bridges and window creation capture explicit handles.
Views own their status tasks. No vault service/status global or UI adapter dependency
is required. Shared scripted domain test support drives public UI behavior; native
adapter plus storage adapter integration tests belong in the app layer.

## Alternatives rejected

- A domain re-export of the storage implementation: it hides the same I/O rather
  than removing it and preserves incorrect dependencies.
- A replacement service global: it still hides collaborators and initialization.
- UI dev-dependencies on storage/native adapters: they violate the feature layer
  and conflate presentation tests with durable I/O integration.
- Mutating state before file publication: failures leave memory ahead of disk.

## Consequences

The domain can be tested without filesystem, worker or OS entropy. Adapter tests
retain publication, revision, Unix permissions, native binding, epoch and rollback
regressions. Crypto and I/O errors remain typed until presentation. Material
constructors require fresh entropy for production use; deterministic values are
restricted by composition to tests. Parent directory trust and Windows ACL limits
remain those of the private publication policy and existing vault security notes.
