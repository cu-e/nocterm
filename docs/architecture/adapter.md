# Adapter layer

Part of the [architecture overview](../ARCHITECTURE.md).

Adapters implement domain contracts with real I/O and depend only on the
domain and foundation: `nocterm-ssh`, `nocterm-local`, `nocterm-localfs`,
`nocterm-transfer-runtime`, `nocterm-acp`, `nocterm-device-unlock`,
`nocterm-vault-store`, `nocterm-agent-storage`, `nocterm-image-codec` and `nocterm-connections-io`. The composition root chooses
and injects them
([app layer](app.md)).

## SSH and local sessions

`nocterm-ssh` implements
that contract using russh and lazily opened SFTP channels on a dedicated Tokio
runtime.
`nocterm-local` adapts portable-pty to
the same session events and launch contract. Integration supplies bounded OSC 7
cwd and prompt markers without parsing rendered prompt text.

## Bounded I/O

Input and output channels are bounded. An independent broadcast close signal
interrupts blocked SSH I/O and local PTY writes/output delivery. Unix local PTY
reads/writes are nonblocking and independently cancellable, including when a
background job retains its slave descriptor. Local closure terminates the shell
and current foreground process groups and reaps the owned child; it does not
enumerate and kill arbitrary detached jobs. Explicit input refusal
is surfaced in the terminal instead of silently discarding text.
HTTP CONNECT/SOCKS5 tunnel creation belongs to the SSH adapter, inside the same
connection timeout and target host-key policy as a direct connection.

Structured local commands through `HostExec::exec` start on a worker when their
future is polled. The worker owns cleanup independently of output delivery or
the shell's lifetime. Unix exit observation leaves the leader unreaped until
its process group is stopped, preventing reuse of the group ID; deliberately
detached process groups are outside that ownership. Windows commands start
suspended and are attached to a Job Object before execution. Dropping the
output or startup future cancels the owned command.

## External local programs

`nocterm-local` implements the separate `ProgramOpener` port for configured external
editors. One opener is injected into Explorer and shared across application windows.
Its 32 admitted workers start and then reap direct children independently of launch
receipt or view lifetime. Literal arguments and native working directories reach
`Command` without shell expansion; standard streams are null. Worker/spawn failure
releases admission, natural reaping frees it, and uncertain wait failure retires a
slot. Unlike PTY/exec cancellation, accepted external editors are neither killed on
receipt cancellation nor included in application shutdown draining. See
[ADR 0025](../adr/0025-native-program-opener.md).

## Device unlock

`nocterm-vault` declares the device-unlock provider contract and owns envelope,
binding and cancellation rules. The composition root injects
`nocterm-device-unlock` into `nocterm-vault-store`; native adapters remain outside
the pure vault and its UI.
An optional `nocterm-vault-broker` Linux system service enforces fingerprint
verification for user-bound memory-only keys. See [device unlock](../DEVICE_UNLOCK.md)
for the platform boundaries and explicit installation.

## Vault storage

`nocterm-vault-store` implements `VaultAccess` with bounded serial background work.
It owns filesystem reads, revision comparison under a sibling exclusive lock,
OS entropy, auto-lock epochs and native registration orchestration. Publication
uses `nocterm_core::fs::AtomicPublication`; the lock remains held through rename,
and pure state is accepted only after success. Device registration is checked
against the current envelope and cancellation epoch before publication or key
installation. Only the app composes this adapter with native authentication.

## Native filesystem capabilities

`nocterm-localfs` implements session's synchronous `LocalFs` and `LocalMutations`
ports. Application composition supplies them to Explorer; feature code dispatches
blocking calls on the background executor. Native names remain `OsString`/`PathBuf`
through operations; lossy strings are presentation only.

Read-only browsing may follow directory links and accepts linked parent navigation.
Strict traversal, source streams and mutations reject linked ancestors and pin each
Unix directory. Walks and removals have entry/depth limits; cancellation can leave
an explicitly reported partial removal. Portable targets check reparse/link state
but do not claim Unix held-handle race guarantees. Staged publication distinguishes
failure before commit from a durability warning after successful publication.
Successful publication relinquishes its temporary name before returning; Drop
cannot remove a later owner of that name. Explicit cancellation confirms removal
through the held directory or an already absent candidate and returns typed cleanup
failures; Drop remains best effort. A failed temporary-link cleanup is a
committed warning, never permission to repeat the publication. See
[ADR 0017](../adr/0017-local-filesystem-capabilities.md) for the capability boundary.

## Transfer execution

`nocterm-transfer-runtime` implements `TransferService` over injected `LocalFs` and
`RemoteFs` ports. It schedules bounded discovery and four workers, uses 32-KiB
chunks with at most eight acknowledged upload chunks per batch, and owns entropy,
cancellable reservations and cleanup. Local operations run through bounded blocking
slots; the adapter imports no concrete filesystem adapter or GPUI.

Sources open relative to held directories. Local destination reservations combine
directory identity and native name, and late Rename collisions reuse an already
synced stage. SSH uploads seal their acknowledged stage and retry publication under
one validated leaf in its original parent; failed/uncertain acknowledgements or
close cannot authorize a partial publish. Committed durability warnings remain
successful files. Native and SFTP limitations are described in
[ADR 0018](../adr/0018-transfer-runtime-capabilities.md).

Its drain port freezes admission synchronously and retains executor ownership
through joined cleanup. Application composition bridges that future into the
common shutdown coordinator; the adapter introduces no quit hook or separate budget.

## Agent history and preferences

`nocterm-agent-storage` implements the AI domain's `ChatRepository` and
`AgentStateRepository`. It owns bounded transcript reads, private atomic JSON/TOML
publication, catalog scanning and rebuildable metadata sidecars. The transcript
is authoritative: a sidecar failure cannot revoke a successful publication.
An archived-draft update merges only its draft and timestamp into the original
transcript under the service's shared serial gate.

The app injects these repositories; service and feature crates have no adapter
dependency. Repository futures run filesystem work when polled on the background
executor. The adapter owns no writer loop, per-chat gate or shutdown timer.
See [ADR0022](../adr/0022-agent-storage-owners.md).

## Agent account usage

`nocterm-agent-storage::CodexUsageReader` implements AI's `AccountUsageSource`.
Composition supplies its optional home directory; unsupported launches never touch
that path. The reader selects the newest lexical date and log, scans at most the
last 256 KiB and returns the last usable account report. Missing or malformed reports
produce no replacement; physical failures preserve typed I/O causes.
AI owns typed shared usage causes and category-specific constructors; native readers
retain the original I/O classification and deeper cause through standard traversal.
Clones share the original payload without changing account selection or tail limits.

## Agent workspace and sandbox

`nocterm-acp::NativeWorkdirResolver` creates the explicit private default workspace
through the shared foundation helper and canonicalizes user-selected roots. One
private path resolver supplies facts to AI's pure policy and revalidates them before
launch. Existing credential aliases resolve to the same paths used for mounting;
non-missing native failures refuse the operation with typed I/O causes.

The native sandbox launcher checks bubblewrap against the current sanitized agent
environment on each launch, regardless of settings probe evidence. It refuses a
changed canonical cwd, protected roots, unsupported platforms and missing or
unexecutable bubblewrap. It projects ordered policy mounts into native flags while
process grouping and cleanup retain their existing owner.

## Connection files and native values

`nocterm-connections-io` implements the connection repository with the shared
foundation codecs. Profiles use `save_preserving` so configuration symlinks,
comments and retained top-level tables keep their existing semantics; recents
and facts use rebuildable snapshot files. A malformed profile load retains a
typed read-only diagnostic; informational documents fall back to empty values.
The adapter supplies user/home defaults, UUID entropy and Unix timestamps.
Application startup preloads documents and defaults on its background executor;
bootstrap installs only prepared documents and writer ports. The existing
memory-only composition constructs no native repository or HTTP clients.
## Image codec

`nocterm-image-codec::NativeImageCodec` implements AI's image inspector and
thumbnailer without filesystem access, GPUI or its own workers. PNG, JPEG, GIF and
WebP decoding applies shared pure preflight before bounded decoder allocation:
4096 pixels per axis and 64 MiB. It validates full decode against header facts.
Previews preserve aspect and alpha, fit requested bounds up to 640 pixels, and
encode PNG through a sink capped at 4 MiB. Native errors retain typed causes.

Remote OS acquisition belongs to `nocterm-connections-io::NativeOsDetector`.
Its 16-KiB release reads use bounded chunks and explicit best-effort close on every
opened-reader result. Cancellation releases handles through Drop. Unknown native
failures retain typed sources, and inert composition issues no remote requests.
See [ADR 0039](../adr/0039-remote-os-observation-boundary.md).

Native chat identity and timestamp facts are supplied by agent-storage's
NativeChatMetadata through AI's ChatMetadataSource. New document allocation
combines one native UUID with its creation timestamp; snapshots and imports
supply their existing identity and time without consulting the adapter.
