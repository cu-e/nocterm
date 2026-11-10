# 0076. Container files use a scoped session filesystem capability

Status: accepted

Extends [0071](0071-provider-workspace-locations.md) and
[0018](0018-transfer-runtime-capabilities.md).

## Context

Launching a container shell must not expose its host's filesystem as the
container's. Removing the host filesystem without supplying a replacement also
leaves Explorer and file transfers unavailable. The selected resource may run
locally or through an existing SSH connection. Requiring an SSH daemon or installed
helper inside that resource adds a separate installation and credential lifecycle.

## Decision

`nocterm-project-runtime` adapts the existing `RemoteFs`, `RemoteUpload` and
`RemoteDownload` contracts through the original `HostExec` captured by the container
session. Initial and nested terminal sessions attach this scoped capability with
the same engine and full container ID. A missing execution capability leaves file
access unavailable; the underlying host filesystem is never substituted.

Fixed shell recipes receive paths as positional arguments. Commands use `/` as
their working directory, independent of the workspace directory, and retain the
container's configured account. The adapter probes compatible Linux `/proc`,
shell and GNU/BusyBox utilities rather than inferring support from an image name.
It installs nothing. Listings, response sizes, command deadlines, queues and open
streams are bounded. Names and metadata use a NUL-delimited representation; invalid
or unrepresentable responses fail rather than becoming partial successful results.

Downloads retain one process output stream and honor bounded caller reads. They
reject known final links and nonregular objects, then compare the opened regular
descriptor's identity before emitting contents. Closing or dropping cancels the
process. Upload chunks are acknowledged before progress and are not buffered as
whole files or spooled onto the host. A private same-parent stage records parent,
staging-directory and file identities. Mutation traversal rejects symlink
components and compares the directory reached with the admitted identity.

Publication seals the stage. Create uses atomic no-clobber hard-link publication;
a confirmed collision retains the sealed stage for `publish_as` within its
original parent. Replace requires an explicitly authorized regular destination
and uses atomic same-filesystem rename. Failed or uncertain acknowledgments poison
the stage. A bounded owner handles abandoned-stage cleanup, checking identity before
removing only its own temporary objects.

The Files and Explorer features continue consuming session contracts. No
provider-specific file protocol, broad workspace port, new global, SSH daemon or
helper-installation API is introduced.

## Limits and consequences

This is a Linux process-backed capability, not a universal filesystem protocol.
Minimal or incompatible images report unsupported access. Directory/link rename
and cross-filesystem rename are refused; regular-file rename uses no-clobber link
publication followed by source unlink, so a failure between those operations can
leave both names. Container permissions apply without implicit escalation.

Shell recipes cannot provide all native held-directory guarantees across separate
commands. If an admitted parent is moved or replaced, later writes, publication
and cleanup fail closed; a relocated private stage may remain for manual cleanup.
If the initial stage-creation response is lost entirely, its identities are unknown;
the adapter refuses name-only deletion and may leave that private stage behind.
Metadata and permission operations retain a concurrent remote path race, as with
path-based SFTP. Inode checks do not detect same-inode content mutation. A raced
special-file open is deadline-bounded. These limits do not permit host fallback,
following known final links for data transfer, or publishing an uncertain stage.

Future providers can supply the same session filesystem contracts through their
own adapters. Their credentials, resource discovery and protocol implementation
remain separate from the built-in container mechanism.

## Alternatives

- Reusing the host SFTP capability routes operations to the wrong filesystem.
- Requiring SFTP inside every container adds an SSH server and authentication.
- Installing a persistent helper strengthens handle semantics but mutates attached
  resources and adds a binary distribution/installation lifecycle.
- Buffering whole files or interpolating filenames into shell source loses the
  existing transfer bounds and safe argument representation.
