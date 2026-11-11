# 0162. Windows holds directory ancestry and guards atomic publication

Status: accepted. Extends the Windows implementation of [ADR 0017](0017-local-filesystem-capabilities.md).

## Context

New v3 chat namespaces require held native directories, including General
([ADR 0110](0110-local-history-environment-partitions.md)). A portable path identity
cannot provide that authority. Windows formerly provided only that weaker identity,
so new chat writes were refused even when the disk and agent were available.

Windows sharing flags can prevent renaming or deleting an opened directory, but
do not prevent attribute-only access. An empty directory can become a reparse point
through an attribute handle. Merely opening a directory without delete sharing is
therefore insufficient for subsequent publication by pathname.

## Decision

`nocterm-localfs` implements Windows `LocalDirectory` through safe native API
wrappers. Strict acquisition opens one component relative to its parent, refuses
reparse points on the opened object, and retains every ancestor without delete
sharing. Acquisition checks that a newly pinned child still identifies the selected
entry. Ancestry is bounded to 256 components. Display paths never replace these
handles for reads, child acquisition, metadata, enumeration or removal.

Native identity comes from the retained object's volume and file ID. A full
128-bit ID and the filesystem's shorter fallback occupy distinct identity spaces
in session's pure `DirectoryIdentity` descriptor. Portable identity remains
portable and does not satisfy v3 storage requirements.

Each Windows stage also creates a random guard child relative to the held leaf,
retains it without delete sharing, and checks every retained ancestor for reparse
state after pinning the guard. This rejects conversion that happened before an
ancestor acquired its pinned child. The chain cannot now be renamed or emptied, and
the pinned guard prevents the leaf from becoming empty. Existing private temporary
file creation and atomic replacement can then safely use the protected pathname.
The guard lasts until temporary cleanup or committed publication finishes. Native
handle deletion removes the guard itself; a cleanup failure after commit is a
publication warning, preserving [ADR 0015](0015-private-file-publication.md).

The adapter retains the whole capability chain for cursors and stages. Cursors
reopen an independent handle so enumeration positions never interfere. All native
work stays behind the existing synchronous session ports, off the UI executor.
No direct `unsafe` code or weakened workspace checks are introduced.

The storage adapter keeps its existing environment/context paths, header ownership
checks and publication acknowledgement. Chat error presentation distinguishes
unsupported native storage, full disk/quota, denied access and other write errors;
all retain queued prompts and the original typed cause.

## Alternatives

Relabeling Portable identity would assert authority that does not exist. Falling
back to old General transcripts would change v3 namespace and context semantics.
Sending queued prompts before successful persistence would discard the durability
boundary. None is an acceptable substitute for Windows capabilities.

Direct Windows FFI inside the adapter conflicts with the workspace's unsafe-code
prohibition. Path-based capability libraries alone also leave the empty-directory
reparse case unresolved. Safe relative operations, pinned ancestry and a temporary
publication guard preserve the existing ports and publication primitive.

## Consequences

Windows supplies native capability semantics for device/account and General/Project
v3 namespaces. Held directories cannot be renamed or deleted until their owners
release them; this intentionally differs from Unix handles surviving such moves.
Read-only browsing can still resolve a requested alias before strict acquisition.
Handle memory grows with bounded ancestry. Publication checks that bounded ancestry
once more and adds one small transient guard file. A process crash can leave an
empty guard or private candidate; these
are not history records and never select chat ownership.

Production LocalFilesystem integration fixtures cover round trips, queue recovery,
namespace isolation, rejection of foreign records and atomic replacement. Windows
CI runs those fixtures and native capability tests. These checks do not establish
GUI rendering, a real Codex conversation or power-loss durability.
