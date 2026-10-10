# 0015. Private snapshots share staged atomic publication

## Context

TOML persistence, vault envelopes, device-unlock registrations and SSH trust
replacement each staged, synced and renamed files. Some owners must validate a
revision or cancellation epoch after staging while retaining a mutation lock.
Terminal output is a stream and requires exclusive creation rather than snapshots.

## Decision

`nocterm_core::fs` owns `create_private`, `write_private_atomic` and
`AtomicPublication`. A snapshot uses an exclusively created temporary file in the
same directory, writes and syncs bytes and permissions before publication, then
renames over the exact destination. Dropping an unpublished snapshot cleans it up.
The owner retains its lock and validates revisions or cancellation between staging
and publication. A rename failure leaves the previous destination intact.

Private snapshots use Unix mode 0600. SSH trust replacement uses an explicit
permission-preserving staging constructor; it retains the previous mode without
claiming to copy ownership or access-control lists. Streams use exclusive creation,
retain their no-clobber names and own their existing append/drain lifetime.

A successful rename commits publication. Parent-directory sync is best effort
and cannot return an apparent precommit failure that would keep in-memory state
behind a file already replaced. Success does not guarantee power-loss durability.

The primitive replaces the final symlink; TOML `save` and `save_preserving`
intentionally resolve an existing configuration symlink before using it. Private
text and invalid-configuration backups replace the exact final path. Parent
creation policy remains explicit: `write_private_atomic` creates missing parents;
staging and streaming creation require an existing directory. Ancestors must be
trusted; link-safe capability traversal belongs to filesystem adapters.

## Alternatives rejected

- One snapshot writer that follows all symlinks: it could overwrite unrelated
  files through private-data or backup links.
- Always force mode 0600: existing SSH trust-file permission policy would change.
- Validate before staging: owner state could change during the write and sync.
- Return directory-sync errors after rename: callers could revert memory after
  the durable file already changed.

## Consequences

TOML persistence and SSH trust replacement share staging and publication while
retaining their schema, locking, limits and cancellation rules. Chat history
inherits the helper through `persist::save_text`; recording shares exclusive
creation and retains its bounded streaming worker. Vault adoption accompanies
the extraction of its storage adapter; its lock files and encrypted format
remain governed by the vault contract.
