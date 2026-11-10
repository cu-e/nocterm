# Adapter layer

Part of the [architecture overview](../ARCHITECTURE.md).

Adapters implement domain contracts with real I/O and depend only on the
domain and foundation: `nocterm-ssh`, `nocterm-local`, `nocterm-localfs`,
`nocterm-transfer-runtime`, `nocterm-acp`, `nocterm-agent-tools`, `nocterm-device-unlock`,
`nocterm-vault-store`, `nocterm-agent-storage`, `nocterm-image-codec`, `nocterm-connections-io`, `nocterm-project-storage`, `nocterm-project-runtime`, `nocterm-project-environment`, `nocterm-project-container-fs`, `nocterm-directory-io` and `nocterm-snippets-io`. The composition root chooses
and injects them
([app layer](app.md)).

## Workspace catalog

`nocterm-project-storage` reads and publishes workspace folder definitions through
the existing private atomic-file primitive and supplies identity/time metadata.
The app injects it into the catalog service; it owns no background writer.

`nocterm-project-runtime` executes built-in workspace preparation through a
captured host executor. Docker and Podman inventories have independent outcomes;
all later commands retain the chosen engine and full container ID. Preparation
passes the actual owned creation receipt and original bounded command context to
its injected creation installer, then clones Git. Command scopes and observations
retain the original timer, cancellation and progress. Recovery alone creates its
existing deliberate fresh uncancelled context.

`nocterm-project-environment` supports fresh creation installation and explicit
existing-environment changes through separate contracts. It probes the actual OS,
architecture, libc, account and prerequisites rather than inferring them from the
host or image reference. Distribution defaults remain explicit; managed Python,
Rust, Bun, official Node archives and checksum-verified Adoptium JDKs retain their
user-owned paths in fresh workspace shells. Installer version values travel as
positional arguments. Unsupported platform/version combinations fail explicitly;
existing environments are preserved after partial installation or cancellation.
Only the actual owned creation path permits initializing an empty Arch package
database with a full synchronization/update. Existing-environment management reports
the missing administrator prerequisite instead. Environment-owned workspace session
and request activation exports fixed managed paths without sourcing a mutable fragment.

Runtime's bounded pull-output decoding emits observed transfer values
or phase-only progress when totals are unavailable.
`NativeWorkspaceContainerActions` separately implements explicitly confirmed lifecycle
actions. Bounded inspection validates the original full ID. Stop is idempotent for
an already stopped container; Delete stops a running one before exact-ID `rm`,
without force, volume removal, images, pruning or name fallback. Failures retain
possible partial effects; no resource recreation attempts to roll them back
([ADR 0078](../adr/0078-explicit-workspace-container-removal.md)).
The separate registry-search adapter uses Docker Hub search or Podman configured
registries through that same captured executor, bounds results/bytes/time, cancels
obsolete work and returns typed quota cooldowns without pulling images. Custom
container creation uses the chosen engine’s cached image reference or ID and pulls
only when missing; curated images retain their explicit pull. Rollback checks the
exact resource identity, project label and a fresh creation-attempt label. A lost create response triggers bounded reconciliation on
the original executor; unresolved outcomes retain a typed, recoverable attempt
reference. Automatic rollback never deletes ordinary folders or attached containers.

`nocterm-project-container-fs` wraps normal host transport with scoped execution and
no host filesystem projection
([ADR 0071](../adr/0071-provider-workspace-locations.md)). Its process-backed
`RemoteFs` adapter uses that same original executor and exact engine/resource for
container files. It passes paths as arguments to fixed recipes, probes required
utilities, bounds queues/commands and streams file contents. Upload publication
retains an identity-checked private same-parent stage; Create collisions preserve
the sealed stage for a name retry. Downloads compare the opened descriptor with
the admitted regular file. Container filesystem commands run from `/` independently
of the workspace directory. Unsupported images have no host fallback and install
no helper or SSH daemon. The adapter's path-race and mutation limits are recorded
in [ADR 0076](../adr/0076-container-filesystem-capability.md). The three project adapters
have no dependencies on one another, including development dependencies; app owns
real cross-adapter composition tests
([ADR 0080](../adr/0080-project-adapter-ownership.md)).

`nocterm-directory-io` projects supplied local and remote filesystem capabilities
into read-only home and directory listing operations. It reuses existing traversal,
limits and typed errors, opens no connection and never substitutes local access
for a missing remote source
([ADR 0072](../adr/0072-shared-directory-selection.md)).

## SSH and local sessions

`nocterm-ssh` implements
that contract using russh and lazily opened SFTP channels on a dedicated Tokio
runtime. Authentication with a vault key reads the record through the credential store,
checks its public half against the profile and signs without SHA-1 for RSA
([ADR 0102](../adr/0102-ssh-keys-are-kept-in-the-vault.md)).
`nocterm-local` adapts portable-pty to
the same session events and launch contract. Integration supplies bounded OSC 7
cwd and prompt markers without parsing rendered prompt text. It also implements the
foundation `WallClock` as `SystemClock`, which reads the system time and the local time zone
in effect at that instant ([ADR 0088](../adr/0088-wall-clock-contract-and-the-clock-plugin.md)).

## User plugin packages

`nocterm-plugin-packages` implements the `PluginPackages` port of `nocterm-plugin-api`. It reads a
package file (a TOML file of at most 256 KiB, or a code package below) without following links through
`nocterm_core::fs::read_regular_bounded`, hashes the exact bytes it then parses, validates the
manifest and publishes immutable private files per validated `user-` plugin id and hash
(`<root>/<id>.<hash>.toml`). Records select one exact version; old versions remain until
the matching records receipt succeeds. Loads also accept legacy id-only files and recheck
the requested hash. Its I/O is limited to those `nocterm_core::fs`
calls and metadata and removal under the package root ([ADR 0089](../adr/0089-user-plugin-contract.md)).
A code package is one `.wasm` module, at most 2 MiB, that carries the manifest in its custom
section `nocterm.manifest`; the store keeps `<id>.<hash>.wasm` or `<id>.<hash>.toml` and the hash
covers the whole file ([ADR 0091](../adr/0091-user-plugin-code-runs-in-process-on-wasmi.md)).
`SessionStore` is the store of a developer session: it reads a file with the same bounded read but
publishes, loads (re-checking the hash) and removes packages in memory, so no package directory is
ever created. See [ADR0141](../adr/0141-versioned-plugin-publication.md).

`nocterm-plugin-wasm` implements the `PluginRuntime` port on wasmi for code packages. It reads no
file: the bytes come from the package port. It validates and compiles a module when a plugin is
instantiated, caches at most 16 compiled modules, runs each call under a fuel budget and a memory
limit, allows at most 16 live instances, bounds every read of guest memory, and maps a trap, a
missed deadline or a memory fault to a `RuntimeError` that the host counts. Guests import
nothing; their log lines go to an injected `GuestLog`. The crate has no thread and no global.

`nocterm-plugin-net` implements the `PluginNet` port for the `net.request` capability. The host
builds each call from the plugin's settings: the origin the user typed and, from a secret setting,
the credential header. The adapter parses the address again and refuses one that is not at that
origin, follows no redirect, keeps no cookies, sends JSON with a plugin user agent, waits at most
15 seconds and reads at most 256 KiB of the answer. A failure is a status-0 answer, not an error.
Calls block, so the host runs them on a background thread ([ADR 0153](../adr/0153-toolbars-and-user-plugin-requests-timers-and-terminals.md)).

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
A hub proxy (`ProxyConfig::Hub`) takes its first stream from the injected
`RelayDialer` port instead of a socket: `SshTransport::with_relay` supplies it, and
without one such a session fails to connect. The adapter bridges the stream's two
bounded frame channels (`RELAY_FRAMES` frames of at most `RELAY_FRAME_BYTES`) to the
async stream the handshake needs, so a relay that stops reading stalls the writer
instead of growing a buffer. Closing either side ends both directions. The relay
carries bytes only: the host key checked and the sign-in are the destination's.

The adapter also establishes jump-host routes ([ADR 0060](../adr/0060-ssh-jump-host-routes.md)).
`ConnectRequest.jumps` lists resolved hops; each is reached through a `direct-tcpip`
forward opened on the previous connection, and the configured proxy carries only the
first connection. Every hop verifies its own host key and completes authentication
before a forward is requested through it, with prompts tagged by their hop. The
connection timeout applies per hop, a failure is attributed to its hop, routes over
the limit are refused before any I/O, and closing the session closes the chain from the
destination outward.

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
directory: Unix holds descriptors and uses `*at` calls; Windows holds a handle to every
ancestor without delete sharing and refuses reparse points
([ADR 0162](../adr/0162-windows-holds-directory-handles-for-strict-local-capabilities.md)).
Walks and removals have entry/depth limits; cancellation can leave an explicitly
reported partial removal. Other portable targets check reparse/link state but do not
claim held-handle race guarantees. Staged publication distinguishes
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

Downloads and relays walk remote sources through one bounded DFS. Its destination
retains either a held `LocalDirectory` or the admitted remote filesystem and parent
path, with separate native-download and remote-publication name policies. Their
shared double-buffer pipeline joins both pending operations before returning a
failure, so a local writer regains its stage before cleanup.

A relay request walks its source tree on the source `RemoteFs`, prepares directories on
the destination and streams each file from a `RemoteDownload` into a staged
`RemoteUpload` with double buffering: the next read batch is in flight while the previous
write batch is acknowledged, so a worker holds at most two batches of buffers. Staging,
reservation, collision policy and publication are shared with uploads. `read_batch` is an
additive `RemoteDownload` method (sequential by default); the SFTP adapter keeps up to
eight READs in flight and returns the in-order prefix up to the first short or
end-of-file reply, and downloads use it too. The runtime also implements the narrow `TransferRequests` port for agents
([ADR 0128](../adr/0128-agents-transfer-files-through-the-shared-transfer-runtime.md)).

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

New Local transcripts use held `scoped-v3/<environment-key>/general` or
`scoped-v3/<environment-key>/projects/<project-id>` descendants. The authoritative
bounded header and body must match the requested environment and context before
payload reads or mutation. Older namespaces are readable only by ThisComputer;
obsolete sidecars are ignored and cannot select membership. See [ADR 0110](../adr/0110-local-history-environment-partitions.md).

`nocterm-agent-storage` implements the AI domain's `ChatRepository` and
`AgentStateRepository`. It owns bounded transcript reads, private atomic JSON/TOML
publication and catalog scanning. The transcript is authoritative; obsolete `.meta` files
are neither read nor written. Version-two documents remain under
`scoped-v2/general/<chat-id>` or `scoped-v2/projects/<project-id>/<chat-id>`;
legacy flat discovery cannot enter Project history. The captured scope and UUID
select one bucket, so identical chat IDs in different scopes are independent.
Discovery reads bounded authoritative headers, then applies the 200-row limit within that
scope. Ordinary v2 catalog reads and overwrite checks read at most the 16 KiB prefix rather
than the full transcript. Arbitrarily ordered valid General v1 files keep their bounded full
scan, including late metadata and first-user fallback titles; no migration rewrites v1 bytes.
A scoped General document takes precedence over a legacy duplicate,
including when damaged; deletion validates both and removes legacy first so a
failed removal cannot resurrect its stale fallback.

The app supplies `LocalFs`; generated descendants, reads, private staging,
publication and removal use retained `LocalDirectory` capabilities. The configured
private storage root has trusted ancestry and alone may be canonicalized. Project
buckets require supplied held identity, with typed refusal on a portable path
implementation. General retains that adapter's documented active-ancestor race
limitation. Generated links and final source links are refused. An archived-draft
update merges only its draft and timestamp into the original scoped transcript
under the service's shared serial gate.

The app injects these repositories; service and feature crates have no adapter
dependency. Repository futures run filesystem work when polled on the background
executor. The adapter owns no writer loop, per-chat gate or shutdown timer.
See [ADR0022](../adr/0022-agent-storage-owners.md).

## Agent usage boundary

The ACP adapter maps the connected agent's context usage, token totals and usage
metadata into AI's neutral records. No adapter in Nocterm reads provider sign-in
tokens or session logs, or requests private provider account endpoints. The agent
owns its authentication, requests and state files
([ADR 0105](../adr/0105-provider-usage-through-acp-only.md)).

## Agent workspace and sandbox

`nocterm-acp::NativeWorkdirResolver` creates the explicit private default workspace
through the shared foundation helper and canonicalizes user-selected roots. One
private path resolver supplies facts to AI's pure policy and revalidates them before
launch. Workspace and sandbox-spawn diagnoses retain the original typed native
I/O cause through foundation sharing and narrow domain constructors. Clones keep
the same original allocation. Existing credential aliases resolve to the same paths used for mounting;
non-missing native failures refuse the operation with typed I/O causes.

The native sandbox launcher checks bubblewrap against the current sanitized agent
environment on each launch, regardless of settings probe evidence. It refuses a
changed canonical cwd, protected roots, unsupported platforms and missing or
unexecutable bubblewrap. It projects ordered policy mounts into native flags while
process grouping and cleanup retain their existing owner.

`nocterm-acp::mapping` is the only place where the agent protocol SDK is mapped to and
from the neutral `nocterm_ai::model` records; no other package may depend on the SDK
([ADR0058](../adr/0058-neutral-agent-protocol-boundary.md)).

ACP native taskkill diagnostics retain their original I/O error through foundation
SharedError, including native classification and shared allocation identity. The public
process shutdown port retains its existing stringified AgentError diagnostic.

## Connection files and native values

`nocterm-connections-io` implements the connection repository with the shared
foundation codecs. Profiles use `RevisionedFile::save_preserving` with observed-revision
conflict detection and a persistent sibling publication lock, so configuration symlinks,
comments and retained top-level tables keep their existing semantics; recents
and facts use rebuildable snapshot files. A malformed profile load retains a
typed read-only diagnostic; informational documents fall back to empty values.
The adapter supplies user/home defaults, UUID entropy and Unix timestamps.
Application startup preloads documents and defaults on its background executor;
bootstrap installs only prepared documents and writer ports. The existing
memory-only composition constructs no native repository or HTTP clients.
One lazy HTTP client per observation bundle shares connection pooling for country and flag
requests. Native DNS has four physical workers and four queued requests, with an eight-second
caller deadline. Expired queued work is skipped. An uncancellable system lookup retains its
worker until actual return, isolating optional country acquisition from other fact jobs.
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

## Snippet documents and identities

`nocterm-snippets-io` implements the domain SnippetRepository and SnippetIds ports.
Composition supplies the snippets path. Native load/save retain core persist's exact
TOML and atomic publication behavior, including rewriting comments on successful save.
The adapter preserves the original PersistError behind typed operation errors, and
performs no validation/fallback, queue or GUI work. Native UUIDv4 acquisition occurs
only when the editor requests a new draft. See
[ADR0055](../adr/0055-snippet-storage-boundaries.md).

## Hub connections

`nocterm-hub-io` is the only crate that speaks the hub protocol. It uses the hub contract
(`nocterm-hub-client`, `nocterm-hub-model`, `nocterm-hub-proto`, `nocterm-keys`), a public
repository outside this workspace that is pinned by commit, and implements the `HubTrust`,
`HubSignIn` and `AccountDevices` ports of `nocterm-hubs-model` on its own small Tokio runtime,
like `nocterm-ssh`. `tonic` and
`prost` values stop at its boundary. The device key and tokens of each hub are kept in one
mode-0600 file in a mode-0700 directory the application supplies; the contract rotates tokens
under an inter-process lock that the adapter takes on a sibling `.lock` file, so two processes
never spend a refresh token twice. A hub has one client while it is trusted the same way, so the
contract's own turn serializes refreshes inside the process, and a wait for the file lock does not
hold up a worker of the runtime. Writers of the file take the same lock. A revoked device erases its file. A failed connection is
explained by reading the certificate the hub presents and comparing it with the pinned one,
which separates a changed certificate from a hub that is not there. See
[ADR 0096](../adr/0096-hub-contract-outside-the-workspace.md).

`HubAccount::local_credentials` supplies a terminal credential store restricted to
one captured native account. Only an in-flight worker holds the original open
keys and client; each StoreOps call uses that client or fails. Cache reads and
publication verify the original key pointer and account under the Slot mutex,
so a late A result cannot read or overwrite B. Returned results check the source
again after worker completion. A stale captured store never revives at unlock;
a new attempt obtains a new source and credential store. Its SDK SessionStore
rejects adoption or publication of another account/device or explicit local
login under the shared file lock. A fallible random local login identity changes
at explicit sign-in, including the same account/device, and survives ordinary
refresh. File and pooled membership publication share the clients mutex, ordered
before the file lock. Captured revocation compares the original local file login;
an old response cannot erase replacement credentials in the publication window.
Already admitted remote A operations retain their existing cancellation limits.

Personal-vault refresh snapshots publish monotonically within a collection: an older
cursor cannot replace a newer cached deletion. Identical refreshes leave the cache file
untouched, and reading one credential copies only its encrypted object and envelope.
Account-cache encoding and acquisition share a 32 MiB file limit; a refused oversized
publication preserves the previous readable file.

`HubAccount::local_source` captures the configured hub and the actual pooled account for
device-local history. The history partition is derived from the account id alone, so a locked
vault never blocks chats; an address or leaf certificate does not establish a stable
authority. The account handle keeps a small provider so capture can be retried. Each returned source keeps only weak key,
pool membership and owner references. Lock, sign-out and replacement login make
old captures stale, including an A→B→A round trip; ordinary refresh preserves the
membership only for the same account, device and trust. Pool eviction invalidates
captures before waiting for remote device revocation. General account notices
are independent from this validity check.

`pool::captured::CapturedClient` is shared by fixed credential-store and relay
consumers. It retains the original SDK client, weak native membership and optional
weak key owner. Refresh uses that client only. Retirement compares exact native
membership; cache/key erase additionally requires the original open-key pointer.
Without that key owner it may retire original membership but never erase another
account's cache. `HubAccount::local_relay` adds the captured source and account
partition. Capturing a relay binds hub and complete target before spawning;
workers and returned streams recheck the original owner after waits. A constrained
hub open requires an explicit captured relay collaborator and never selects the
ordinary desktop relay as a fallback.

`HubRelay` implements `RelayDialer` of `nocterm-session` over the same runtime and the same
signed-in clients ([ADR 0100](../adr/0100-the-hub-is-a-route-step.md)). It finds the hub by
address among the hubs this device is signed in to and never signs in on demand. After the hub
opens the connection, one task carries frames between the bounded channels of the `RelayStream`
and the hub's stream: the two runtimes share nothing else. A target that is silent does not
keep a closed session alive, and each direction ends on its own: dropping the sender is end of
file for the target, which may still answer. A refusal is told by the hub's code, never by its
text.

`HubAgentsLink` implements the `HubAgentDirectory`, `HubServers` and `HubChanges` ports of
`nocterm-hub-agents-model` over the hub's agent and server services; its hints come from
one `Watch` of the account per tracked hub (chat changes, and `AccountChanged` for agents,
providers and servers). `HubAgentConnector` implements
`nocterm_ai::AgentConnector` for agents whose launch names a hub
([ADR 0148](../adr/0148-hub-chats-run-on-the-hub-and-use-the-agent-chat-ui.md)): a new
session is shown with the agent's model and mode and its first message creates the hub chat
under the id the session already has, a restore reads its history and watches it, a prompt uploads its
attachments, sends the message and resolves at the end of the first run that started after
it, cancel stops the run, approvals answer the chat's pending approval, and the model option
sets the chat's model. Runs started from another device become neutral turn notices. A
`Watch` per open chat is the only stream; nothing polls. `HubChatRepository` decorates the
local `ChatRepository`: in a hub environment it lists, renames, pins and deletes the hub's chats
and keeps only which local documents started which hub chats in a private file; a hub chat's
transcript is never written locally.

## Agent command execution

`nocterm-agent-tools` supplies the session-domain `CommandJobsFactory` and `CommandJobs`
ports. Each chat gets an independent registry with four active and sixteen retained commands.
The adapter drains all stdout while retaining a 64KiB prefix, preserves typed startup
errors and the transport's bounded stderr tail, and holds each original executor.
The host schedules returned futures and supplies absolute deadlines; cancellation drops
the held transport output and preserves a readable outcome with an unknown exit.
The app injects the factory through the retained agent installation. Neither the
chat service nor its views depend on the adapter ([ADR 0106](../adr/0106-shared-terminal-tool-ownership.md)).

## Native hub account admission

`nocterm-hub-io` captures authenticated native membership before account work is queued. Opened replicas and terminal credential facades never select a replacement login; the general vault captures membership separately for unlock while keys are locked. Pool admission and Slot lifecycle receipts make key/cache and idle-timer publication conditional on their original owner. Reconnect/login/sign-out retain the original private-file login generation. Recovery offers bind the original recipient and displayed action. See [ADR 0114](../adr/0114-native-captured-account-operations.md).


Agent access edits of `authorized_keys` use the SFTP adapter's fixed no-clobber owner lock,
with no expiry or automatic orphan recovery. Different Nocterm sessions/devices coordinate
through that lock. Existing empty files use Replace; absence uses Create. Final comparison and
checked lock cleanup remain best effort against arbitrary external editors. Busy and
unsupported protocol failures retain typed causes; the agent key's line can still be copied
and added by hand.
See [ADR0139](../adr/0139-cooperative-authorized-keys-edits.md).
