# Domain layer

Part of the [architecture overview](../ARCHITECTURE.md).

Domain crates hold the rules and contracts of one subject and depend only
on other domain crates and the foundation, never on GPUI: `nocterm-session`, `nocterm-terminal-model`, `nocterm-explorer`,
`nocterm-vt`, `nocterm-themes`, `nocterm-ai`, `nocterm-monitor`,
`nocterm-containers`, `nocterm-snippets`, `nocterm-transfers` and
`nocterm-vault` and `nocterm-connections-model`. Adapters implement their contracts ([adapter layer](adapter.md));
features present them ([feature layer](feature.md)).

## Connection formats and rules

`nocterm-connections-model` owns saved profiles, recent destinations, detected
facts and nested `ProfileDraft` validation. Collection transitions preserve
checked edit expectations, remembered folders, user ordering, credential
matching and recent deduplication. Native identifiers, timestamps and editor
user/home defaults are supplied through values or narrow ports. Validation
preserves typed credential, JSON, launch and session-option causes.
Profile encoding errors retain original typed TOML serializer causes through
foundation `SharedError`; clones share the native diagnostic. Checked conflict
classification and both 64 KiB mutation/profile limits remain domain rules.
`ConnectionRepository` describes blocking document calls without naming a file
implementation; GPUI publication and observation scheduling stay outside the domain.
`RecentUse` supplies semantic session-use inputs without workspace types or timestamps.
`ObservationEpochs` tracks only live profile IDs. Supplied generations advance
incarnation, exact target and manual icon/country stamps at each candidate transition;
equal final payloads retain intermediate history. `ProfilePublication` projects immutable
live authority without serialized stamps or full document copies.
Checked `OsId` and release aliases/marker eligibility are pure rules.
`RemoteOsDetector` supplies typed acquisition results over a held session capability;
legacy detected-fact strings retain their serialized representation.

## Sessions and terminal emulation

`nocterm-session` defines transport-independent targets, authentication,
commands, events, prompts and the `RemoteFs` contract. Launch and session-override
validation returns typed field/constraint errors without filesystem access or
value expansion; native transports reject invalid launches before starting I/O.
`TextEncodingError` carries the configured `Charset` without retaining input text.
Text conversion stays in the terminal feature, outside the session domain.
Local filesystem diagnostics retain native I/O and recursive partial-removal causes
through foundation `SharedError`; clones share each original allocation. Domain
constructors preserve semantic NotFound/PermissionDenied/AlreadyExists conversion
and its deliberate lack of a native source. Publication warnings retain their native
kind while the already committed file remains a successful outcome.
`nocterm-vt` turns a byte stream into a terminal grid using
alacritty_terminal and encodes keyboard, mouse and paste input. These domain
crates do not know the workspace or GPUI.

Recording diagnostics preserve native I/O through immutable foundation `SharedError`.
Domain constructors retain operation category, path and physical append offset;
equality continues to compare those semantic values rather than cause identity.
Native adapters and the shared scripted port retain the same physical-prefix and
seal ordering, including failure outcomes that do not claim durability.

The optional session `test-support` feature provides `InMemoryFs`, `FakeExec`,
`ScriptedTransport` and `FakeCredentialStore` to consumers through dev-dependencies.
They perform no real I/O or background scheduling. Tests drive the real session and
execution channel ends and release pending filesystem/credential operations through
`Gate`, so cancellation and back-pressure remain part of the port behavior. The
filesystem uses one coherent tree, rejects link traversal and publishes staged
uploads only on successful publication; `publish_as` preserves an acknowledged stage
after a known name collision so Rename retries reuse bytes. A canceled or dropped
uncommitted upload leaves its destination intact.

## Keyboard encoding

Keyboard input is negotiated through the emulator rather than application-name
heuristics. `Modes::keyboard` exposes kitty's five progressive flags and xterm's
modifyOtherKeys level. The encoder separates legacy, kitty and xterm forms,
returning encoded input, deferred composed text, or an ignored event. Without
negotiation, Shift+Enter remains CR; kitty flags 7 distinguish it as
`CSI 13;2u`, while plain Enter stays CR. Enhanced F3 uses `CSI 13~`, avoiding the
cursor-position-report collision. Kitty takes priority when both protocols are
requested. Modifier levels and query replies follow
[kitty's keyboard protocol](https://sw.kovidgoyal.net/kitty/keyboard-protocol/)
and [xterm's controls](https://invisible-island.net/xterm/ctlseqs/ctlseqs.html).
Kitty effective state and a bounded 4096-entry saved stack are independent on
each screen; the xterm modifier resource is global and resets on RIS. A narrow
keyboard negotiation epoch also identifies transitions that return to identical
flags within one output chunk; ordinary text and queries preserve the epoch.

## Search

VT's literal Unicode search uses bounded KMP scan steps over grid cells and their
combining characters, with memory proportional to the query rather than the
number of matches or history size. It supports case sensitivity and Unicode
whole-word boundaries, preserves soft wraps and can match an explicit hard line
break. Regex mode joins soft wraps but matches only within each complete logical
line. It limits each line to 64 KiB, caps compiled regex size and returns an
explicit typed `SearchError` when a limit is exceeded. Regex compilation retains
its native cause; cancellation and obsolete generations cannot publish matches.
Regex batches run off the UI thread;
Terminal schedules scan slices and invalidates matches when output, geometry or
screen changes. A rapidly changing buffer can defer a complete result, but
stale highlights are cleared and input remains responsive. Search highlights
and clipboard selection are independent.

## Line metadata

Terminal line metadata is attached inside the patched emulator Cell representation
and follows grid moves/reflow/history. `nocterm-vt` projects logical IDs and UTC
observation timestamps separately from text. TerminalView allocates a gutter
outside PTY columns/selection and hides it in alternate screen. See the
[maintained patch](../../vendor/alacritty_terminal/NOCTERM.md).

## Themes

`nocterm-themes` is a GUI-independent domain crate. It parses original Zed JSON,
loads user files and installed extension manifests, maps palettes, and provides a
bounded registry client and archive installer. The installer accepts only direct
regular `themes/*.json` files and publishes a validated pack through a staged
rename. `nocterm-ui` resolves light/dark selections from its supplied SettingsHandle and the
catalogue into effective Design tokens, retaining the base tokens for Nocterm
Default. Imported component palettes start from empty configuration so the
component fallback chain derives unmapped colors. The UI Appearance page retains the theme
service and runs explicit search/install/remove work on the background executor;
startup loads the local catalogue without network requests.

## Running programs on a host

`nocterm-session` also defines `HostExec`, a contract for running one program
on a host with its standard output streamed back; the SSH adapter runs it on an
exec channel of the live connection and `nocterm-local` as a child process.
`nocterm-monitor` builds one long-running collection script per watch (POSIX
`sh` reading `/proc` and `/sys` on Linux, PowerShell with CIM on Windows),
splits its output into frames, parses them into raw counters and turns
successive readings into loads, rates and bounded histories. It knows neither
GPUI nor the workspace.

`ProgramOpener` is a distinct pure session contract for external local editors. Its
validated request preserves literal arguments and native cwd; success acknowledges
spawn. The adapter owns children, not the domain or the view. The same dev-only
`FakeExec` implements this port with bounded scripted answers and deterministic gates.

## Containers

`nocterm-containers` (domain) speaks the Docker and
Podman command lines over `HostExec` — detection (exit 127 or
`ExecError::NotFound` means not installed), listings parsed from either JSON
dialect, the event stream and actions — and knows nothing of GPUI.
`ListingError` distinguishes native JSON failures (with line and column), non-object
entries and missing container/image identifiers. `ContainersError` preserves those
parse sources and startup `ExecError` sources separately from process exit status
and its bounded 4-KiB stderr tail. Malformed successful output remains a retryable
listing failure; absence and unsupported execution remain separate outcomes.

## Transfers

`nocterm-transfers` owns bounded requests, collision decisions, typed diagnostics,
progress transitions and the `TransferService` port. It has no runtime, native file
operations, clocks or entropy. Queue, discovery and stream limits have distinct
checked owners. Progress advances only for acknowledged bytes; terminal completion
requires discovery and worker cleanup. Cancellation does not prematurely evict live
batches. The optional dev-only `test-support` feature exposes an explicitly driven
port fake, without scheduling or streaming. Execution belongs to the transfer
adapter ([ADR 0018](../adr/0018-transfer-runtime-capabilities.md)). Runtime errors
use core shared cause ownership through domain constructors, so standard source
traversal retains their original concrete classification. Snapshot clones retain
the same diagnostic identity; adapter code gains no core dependency for construction.

## Vault

`nocterm-vault` owns the versioned authenticated envelope, bounded codecs,
credential binding rules and pure prepared/accepted state transitions. Entropy is
supplied as values by adapters. A prepared write leaves state unchanged; acceptance
is infallible after publication. `VaultAccess` and `DeviceUnlockProvider` declare
UI/service and native authentication contracts without workers or filesystem I/O.
`nocterm-vault-store` implements storage and lifecycle in the adapter layer;
`nocterm-vault-ui` receives an explicit `Arc<dyn VaultAccess>`. Profiles store opaque
IDs constructed from supplied bytes in `nocterm-session`. Authentication uses only
matching password/key-passphrase bindings; Remember commits after Connected and
excludes MFA. See [security/lifecycle notes](../VAULT.md) and
[ADR 0016](../adr/0016-vault-storage-boundary.md).

Transfer progress distinguishes completed files from acknowledged destination directory
preparations. A prepared directory may already have existed; the count records successful
adapter create/open acknowledgement, not a new-file or new-directory count. Its acknowledgement
survives later failure or cancellation without advancing file or byte counters.

## Agent history

`nocterm-ai` owns pure version-one transcript codecs, structural/collection
validation, catalog summary rules and the `ChatRepository`/`AgentStateRepository`
contracts. Filesystem history/preferences implementation belongs to
`nocterm-agent-storage`; the domain exports no forwarding filesystem facade.
SavedChat and SharedChat hold neutral `nocterm_ai::model` records whose serialization
is the version-one transcript JSON; no agent protocol SDK type appears in the domain
([ADR0058](../adr/0058-neutral-agent-protocol-boundary.md)).
Chat identity allocation and wall-clock observations are supplied through the
ChatMetadataSource contract; the transcript reducer accepts supplied timestamps
([ADR0040](../adr/0040-native-chat-metadata.md)). New storage contracts use the existing typed records.
See [ADR0022](../adr/0022-agent-storage-owners.md).

Account plan usage is read through AI's `AccountUsageSource` contract. `UsageAgent`
contains launch identity without environment secrets; typed `UsageReadError` retains
native causes as values. Domain `Limits` and transcript estimates remain pure;
provider log discovery and parsing belong to the agent-storage adapter.

AI workspace preparation is a pure `WorkdirResolver` contract with typed
`WorkdirError` values. `SandboxPolicy` validates supplied lexical/canonical path
facts and produces ordered semantic mounts without native lookup or argv.
`SandboxProbe` returns availability for presentation; that fact does not authorize
a later launch. Native filesystem/home/PATH resolution belongs to the ACP adapter.

## Agent transcript and configuration rules

`nocterm-ai::thread::ThreadState` reduces conversation entries, timestamps, plan,
usage, token counts and title. Per-chat options, modes, command revisions and
reconciliation belong to the feature's private `ChatConfig`. Existing
`session_config` helpers retain pure choice and category rules; protocol-backed
version-one values remain migration debt at their established seams.

Version-one `SavedChat::for_identity` constructs framing from supplied values without
allocating a replacement UUID or querying time. Its saved-window boundary is shared
with feature semantic comparisons. Genuine new/fork allocation uses the injected
native ChatMetadataSource. The transcript reducer receives the same timestamp as the accepted document transition;
the history wire format and protocol-backed records retain their current representation.

## Agent image contracts

AI owns pure image header preflight, checked format/dimension facts, exact prompt
encoding and collection limits. ImageInspector supplies validated original bytes;
prompt admission checks their Arc identity and facts against its own preflight.
ImageThumbnailer supplies bounded PNG previews, with pure checked dimensions and
output-size contracts. Native decoding and PNG encoding belong to an adapter.

## Display and Explorer settings

`nocterm-terminal-model` owns the terminal display settings schema, ranges and
sanitization without GPUI, emulation or native sessions. `nocterm-explorer` owns
Explorer indexing settings, ordered opener matching and literal argument substitution;
filesystem traversal and native program requests stay in the Files feature and its
injected adapters. Monitor and Vault schemas belong to their existing domain crates.
These schemas retain their established serialized names and public schema identities.
The foundation settings schema builder supplies ordered property/default machinery;
policy descriptions, constraints and defaults remain with each domain owner.

## Snippet contracts

The snippet domain also declares blocking SnippetRepository and supplied SnippetIds
capabilities. Storage errors retain the original native source behind a typed read/write
operation; no native paths, entropy acquisition or I/O enters the domain. Record and
library serialization, validation and exact expected-record mutations remain pure.
