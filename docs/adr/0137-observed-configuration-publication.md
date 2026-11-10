# 0137. Configuration publication checks the observed document

Status: accepted

## Context

Atomic replacement protects file completeness and the shared persistence worker orders one
owner's edits. Neither detects another process publishing a newer settings or profile document.
A stale complete snapshot could erase those changes.

## Decision

`nocterm-core::persist::RevisionedFile` owns one path and its last observed or successfully
published revision. `SettingsFile` and `FileConnectionRepository` use it for settings and
profiles. Clones share the observation; separately constructed owners have independent bases.
Reading records the revision of the same bytes returned to the decoder.

A writer acquires a nonblocking OS lock on a persistent sibling of the resolved configuration
target, reads the current file, compares the revision, then retains the lock through private
atomic publication. Configuration symlinks retain their existing write-through semantics;
retargeting an observed symlink conflicts. Lock files are regular private files and are never
unlinked as normal cleanup, because replacing their inode would break interprocess exclusion.
Lock contention and native failures preserve their causes. A stale owned document returns
`PersistError::Conflict` without changing either the file or its observed revision.

Settings compare the top-level tables their document owns and preserve current unowned or
unreadable tables. Profiles compare their whole document. Revisions retain SHA-256 table
fingerprints, not another copy of the document. Formatting changes inside an owned table can
also conservatively conflict. Reloading observes a new base; a failed write does not silently
retry against an externally advanced base. Recents and observed facts remain rebuildable
snapshots.

The sibling lock coordinates cooperating Nocterm processes. It cannot make arbitrary external
editors participate: an external write detected before admission conflicts, but an editor that
races the final rename can still be lost. Trusted ancestor policy and pre/postcommit semantics
remain those of [ADR0015](0015-private-file-publication.md).

## Alternatives rejected

- Locking the destination inode: atomic replacement gives the next writer another inode.
- Silently rebasing every field: these owners do not retain enough user edit intent to choose
  a correct merge of conflicting tables or profiles.
- Making the whole application single instance: it changes window-launch behavior and still
  does not coordinate external editors.

## Consequences

Conflicts become visible persistence failures rather than successful stale overwrites.
The lock is bounded native work on the existing disk worker; it adds no GUI tasks, process-wide
registry, retry loop or second document authority. A held owner must load before editing
existing state. A save without a preceding load deliberately adopts the current file as its
initial base, as a direct replacement API previously did.
