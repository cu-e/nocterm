# Domain layer

Part of the [architecture overview](../ARCHITECTURE.md).

Domain crates hold the rules and contracts of one subject and depend only
on other domain crates and the foundation, never on GPUI: `nocterm-session`, `nocterm-terminal-model`, `nocterm-explorer`,
`nocterm-vt`, `nocterm-themes`, `nocterm-ai`, `nocterm-cloud-model`, `nocterm-cloud-conversation-model`, `nocterm-monitor`,
`nocterm-containers`, `nocterm-snippets`, `nocterm-transfers` and
`nocterm-vault`, `nocterm-ssh-keys`, `nocterm-plugin-model`, `nocterm-window-layout`, `nocterm-project-model`, `nocterm-project-providers` and `nocterm-connections-model`. Adapters implement their contracts ([adapter layer](adapter.md));
features present them ([feature layer](feature.md)).

## Workspace folders

`nocterm-project-model` owns stable workspace/context identities, checked names,
ordinary host folders, provider resource envelopes and the versioned catalog.
Provider references have independent schema versions and bounded inert data;
unknown providers retain their references and history without acquiring a session.
Import provenance grants no execution authority. Existing catalogs migrate to
ordinary host folders without changing identity.

`nocterm-project-providers` owns checked built-in container locations and the
host-backed preparation contracts. Container identity includes the original host,
ordered jump route, Docker/Podman engine, full container ID and folder. Actual
command execution belongs to the runtime adapter. Pure exact-container request
construction preserves arguments, stdin and explicit workspace-cwd choice without I/O. Checked ephemeral language/version
selections reject incompatible version syntax and duplicate languages before I/O.
`WorkspaceEnvironment` separately admits observed platform capabilities and explicit
tool changes against a checked host or already scoped executor. Neutral environment
facts and transfer progress remain ephemeral; `WorkspaceSessionFactory` activates
managed paths independently of container routing
([ADR 0075](../adr/0075-observed-workspace-environment-management.md)).
`EnvironmentCommands` and its factory preserve original bounded command authority
across scopes and observations. The separate `CreatedWorkspaceEnvironment` requires
an actual owned receipt; ordinary installation accepts no fresh-creation flag.
`WorkspaceCommandActivation` supplies managed exports before container wrapping;
all implementations are explicitly composed adapters
([ADR 0080](../adr/0080-project-adapter-ownership.md)).
The separate `ImageSearch` port accepts the captured host executor, engine,
checked query and cancellation without expanding the preparation contract; registry DTOs
and typed rate-limit cooldowns do not enter the catalog. Workspace identity selects chat
history independently of visible tabs and provider availability
([ADR 0071](../adr/0071-provider-workspace-locations.md)).
The same provider domain owns `[workspaces]` defaults. Its container directory is
a structurally validated absolute POSIX path with `/home` as the initial policy;
it neither creates directories nor rewrites saved workspace locations.

`WorkspaceContainerActions` is a separate explicit Stop/Delete contract. A checked
source pairs the saved container location with a matching original host and ordered
route; its resource key ignores folder and saved profile aliases. Keep requires no
provider call. Typed failures retain native causes and acknowledged stop effects
([ADR 0078](../adr/0078-explicit-workspace-container-removal.md)).

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

`ConnectionOpenOptions` carries a captured `ConnectionSource` separately from
route facts. A synchronous guard checks the original owner; a resumed operation
rechecks it before each poll and after completion. A transport that cannot retain
it rejects constrained opens. `SessionDriver::close` wakes independent native
waiters on cancellation or producer termination. `ConnectionFailure` preserves
native typed causes and an actionable kind without protocol types in this domain.

`nocterm-session` defines transport-independent targets, authentication,
commands, events, prompts and the `RemoteFs` contract. Launch and session-override
validation returns typed field/constraint errors without filesystem access or
value expansion; native transports reject invalid launches before starting I/O.
`TextEncodingError` carries the configured `Charset` without retaining input text.
`ExecOutput::collect_bounded` reports typed output overflow separately from an
unknown command exit; the existing `collect` retains its bounded-prefix behavior.
`fs::read_bounded` collects a small document from an already admitted
`RemoteDownload`, validating requested chunk sizes and total bytes. Callers retain
the stream and their own close-error policy. Neither helper opens a transport or
adds native I/O to the domain.

Incremental text conversion belongs to `nocterm-vt`; the session domain only
names the charset and typed conversion refusal.
`Event::Restore` is the screen a host holds when a session is attached again: the owner
paints it on a fresh emulator and never treats it as output of the program
([ADR 0095](../adr/0095-attached-sessions-restore-and-detach.md)).
Local filesystem diagnostics retain native I/O and recursive partial-removal causes
through foundation `SharedError`; clones share each original allocation. Domain
constructors preserve semantic NotFound/PermissionDenied/AlreadyExists conversion
and its deliberate lack of a native source. Publication warnings retain their native
kind while the already committed file remains a successful outcome.
Jump hosts cross this boundary only as resolved values: `JumpHost` carries target and
authentication and `ResolvedJump` adds an optional saved credential, `ConnectRequest.jumps`
lists them in order up to `MAX_JUMP_HOSTS`, `Prompt.hop` (`PromptHop`) names the
hop a prompt belongs to, and `ConnectStage` names the hop being reached. `HostAddress`
(destination plus jump-host targets) identifies a machine as reached: the workspace
matches sessions, transfer jobs and program hosts by it, not by target alone, and
`nocterm-connections-model::credential_scope` limits a remembered credential to the
connection (and its recents) it was typed for when that connection has jump hosts.
`nocterm-connections-model::replica` defines the `ProfileReplica` port (a feed of changes
and versioned writes of one sealed object per profile), its values, the persisted agreed
state and the pure rules that reconcile local, remote and agreed profiles
([ADR 0101](../adr/0101-profiles-synchronize-through-a-replica-beside-the-local-document.md)).
`nocterm-connections-model::jump_route` stores a profile's
ordered jump entries (saved connections or literal addresses) and resolves them
purely at open time, with typed errors for deleted connections, cycles and routes over
the limit ([ADR 0060](../adr/0060-ssh-jump-host-routes.md)).
A route can start at a hub instead of a socket: `ProxyConfig::Hub` names the hub by an
opaque address (and optionally one of its nodes) and `validate` bounds both, without
dialling. `RelayDialer` is the port the application implements to turn that address
into a `RelayStream`: two bounded frame channels (`RELAY_FRAMES` frames of at most
`RELAY_FRAME_BYTES`) so a side that stops reading stalls the other instead of growing
a buffer, and dropping an end closes both directions. `RelayError` types why a stream
was not opened (not signed in, refused, node unavailable, unreachable, closed). No hub
type appears in the session domain
([ADR 0100](../adr/0100-the-hub-is-a-route-step.md)).
`nocterm-vt` turns a byte stream into a terminal grid using
alacritty_terminal and encodes keyboard, mouse and paste input. It also owns
`TextCodec`, the incremental charset decoder/encoder, and `ShellIntegration`,
the bounded OSC directory and prompt observer. Shell observations grant no session
or input authority; the caller still validates its captured session and admits
input before recording it with the observer. These domain crates do not know
the workspace or GPUI.

Recording diagnostics preserve native I/O through immutable foundation `SharedError`.
Domain constructors retain operation category, path and physical append offset;
equality continues to compare those semantic values rather than cause identity.
Native adapters and the shared scripted port retain the same physical-prefix and
seal ordering, including failure outcomes that do not claim durability.

The optional session `test-support` feature provides `InMemoryFs`, `FakeExec`,
`ScriptedTransport`, `ScriptedRelay` and `FakeCredentialStore` to consumers through dev-dependencies.
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
The Linux container platform instead builds own-cgroup v1/v2 CPU/memory recipes.
It resolves process membership and controller mounts, parses bounded facts and
derives CPU as percent of one CPU. Memory retains an optional observed limit;
missing host-only facts stay absent. This reuses the watch's supplied executor and
frame stream, adding no native filesystem or process acquisition to the domain.

`ProgramOpener` is a distinct pure session contract for external local editors. Its
validated request preserves literal arguments and native cwd; success acknowledges
spawn. The adapter owns children, not the domain or the view. The same dev-only
`FakeExec` implements this port with bounded scripted answers and deterministic gates.

`command_jobs::{CommandJobs, CommandJobsFactory}` supplies chat-owned command
registries and immutable outcome snapshots without exposing an adapter or scheduler.
Creating a registry or admitting a command starts no program; the host polls the
returned runner. The dev-only
`ScriptedCommandJobsFactory` records accepted requests and publishes explicitly
scripted outcomes without calling `HostExec` or reading transport output. Actual
execution and retention behavior belong to the adapter's conformance tests.

## Containers

`nocterm-containers` (domain) speaks the Docker and
Podman command lines over `HostExec` — detection (exit 127 or
`ExecError::NotFound` means not installed), listings parsed from either JSON
dialect, the event stream and actions — and knows nothing of GPUI.
`EventReader` turns the event stream into `Changes`: which of the two listings an event
made stale. Commands run inside a container (health checks), attached terminals and
copies change neither, an unchanged health status changes nothing, and an unreadable line
changes both ([ADR 0131](../adr/0131-one-container-watch-per-host-connection.md)).
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

A request is an upload, a download or a relay between two remote hosts
(`Request::Relay`, `TransferDirection::Relay`). `Request::between` pairs two
`TransferEndpoint`s (`Local` or a remote host with its `RemoteFs`) into the right kind,
refuses two local endpoints and a destination equal to or inside a source on one host,
and validates the result against the default discovery limits. Consumers that only
start, follow and cancel transfers use the narrow `TransferRequests` port, which
the transfer runtime implements beside `TransferService`; `TransferId` parses from its display form.
`OwnedTransfers<B>` is the pure per-owner book over that port: at most four active and
sixteen retained transfers (`OwnedLimits`), no access to ids the owner did not submit,
final snapshots kept after the shared runtime trims its history, and cancellation of
active transfers whose binding the owner no longer keeps. It reports `OwnedTransferError`
(`TooManyActive`, `NotOwned`, `Expired`, or the underlying `TransferError`)
([ADR 0128](../adr/0128-agents-transfer-files-through-the-shared-transfer-runtime.md)).

## SSH keys

`nocterm-ssh-keys` owns the rules of an SSH key without I/O or entropy: import of OpenSSH
private keys with the accepted algorithms, the canonical public text, the fingerprint and
the `VaultSshKeys` port the connection editor uses to list, create and import vault keys.
Its `ssh-key` version is the one `russh` resolves. See
[ADR 0102](../adr/0102-ssh-keys-are-kept-in-the-vault.md).

Bounded `InitialHostProof`, `UserauthLogin` and `LoginSignature` belong to this
key-format domain. The complete Ed25519 userauth parser rejects other purposes,
algorithms and suffixes; the pinned host must have signed the initial session ID.
`LoginSigningKey` exposes only `sign_login` for this checked action, with no key
getter or generic signing method. Native SSH observes initial KEX; consent policy
belongs to cloud-model ([ADR 0112](../adr/0112-purpose-bound-ssh-login-and-captured-sources.md)).

## Vault

`nocterm-vault` owns the versioned authenticated envelope, bounded codecs,
credential binding rules, the pure `CredentialBinding::for_request` mapping shared by
native adapters and presentation, and pure prepared/accepted state transitions. Entropy is
supplied as values by adapters. A prepared write leaves state unchanged; acceptance
is infallible after publication. `VaultAccess` and `DeviceUnlockProvider` declare
UI/service and native authentication contracts without workers or filesystem I/O.
`nocterm-vault-store` implements storage and lifecycle in the adapter layer;
`nocterm-vault-ui` receives an explicit `Arc<dyn VaultAccess>`. Profiles store opaque
IDs constructed from supplied bytes in `nocterm-session`. Authentication uses only
matching password/key-passphrase bindings; Remember commits after Connected and
excludes MFA. A stored SSH key is the binding `PrivateKey { public_key }`, referenced from
a profile by `Auth::VaultKey`; `SecretRequest::PrivateKey` is never storable. See [security/lifecycle notes](../VAULT.md) and
[ADR 0016](../adr/0016-vault-storage-boundary.md).

Transfer progress distinguishes completed files from acknowledged destination directory
preparations. A prepared directory may already have existed; the count records successful
adapter create/open acknowledgement, not a new-file or new-directory count. Its acknowledgement
survives later failure or cancellation without advancing file or byte counters.

## Agent profiles

`nocterm-ai::tools::output` owns the shared terminal, command and live-observation
report DTOs. Desktop and headless tools normalize their terminal snapshots into
that format; command reports share explicit redaction policy and retain the actual
exit, byte count and truncation. The panel decodes the same DTOs while retaining
its labels, unknown-field display and historical provider-envelope fallbacks.
These reports are observations, never execution or cancellation capabilities.

`nocterm-ai::agents` describes how a known agent deviates from plain ACP as an
`AgentProfile`: its pinned launch, the home directories it writes, how it reports
plan limits and its brand mark. Deviations are named by shape, never by brand.
`AgentCatalog` validates profiles (unique valid ids, a command, permitted inherited
environment, home-relative state) and finds the profile of a launch by id, then by
an adapter marker in its command. `AvailableAgents::resolve` applies the `ai`
settings to the catalog and returns typed `AgentConfigWarning`s; `validate_agent`
takes the catalog to know which agents may leave their command unset. The domain
names no concrete agent: `nocterm-agent-providers` holds one module per built-in
agent and `catalog()`, which the application injects.

## Agent history

`nocterm-ai` owns pure transcript codecs, structural/collection validation,
catalog summary rules and the `ChatRepository`/`AgentStateRepository` contracts.
Versions one and two belong only to ThisComputer. Version one remains byte-stable
and belongs to General. Version two requires an
explicit `ContextId` and a canonical envelope whose authoritative catalog header
fits within a 16 KiB prefix. Its bounded display metadata and content/title
projection precede the payload; full hydration validates their agreement.
Prefix-only discovery can offer that declared header even when a damaged or
disagreeing payload cannot hydrate. Repository catalog, load and delete calls
capture their expected scope; saving derives it from the document. Filesystem history/preferences implementation belongs to
`nocterm-agent-storage`; the domain exports no forwarding filesystem facade.
SavedChat and SharedChat hold neutral `nocterm_ai::model` records whose serialization
preserves the version-one transcript JSON. SharedChat also supplies a conservative
structural ownership cost for full retained media and its separate shared queue
without allocating an encoded transcript buffer; no agent protocol SDK type appears in the domain
([ADR0058](../adr/0058-neutral-agent-protocol-boundary.md)).
Chat identity allocation and wall-clock observations are supplied through the
ChatMetadataSource contract; the transcript reducer accepts supplied timestamps
([ADR0040](../adr/0040-native-chat-metadata.md)). New storage contracts use the existing typed records.
See [ADR0022](../adr/0022-agent-storage-owners.md).

Account plan limits come only from agent usage events. `rate_limit_record` parses
usage-update metadata under the key the profile's `UsageMeta` names. Domain
`Limits` merges each reported window independently; missing or malformed records
leave prior reports intact. Context, token totals and transcript estimates remain
independent of account limits. There is no native account-source port or provider
credential/log discovery in the application
([ADR 0105](../adr/0105-provider-usage-through-acp-only.md)).

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
Explorer indexing settings, ordered opener matching, literal argument substitution,
pure directory completion rules and read-only directory contracts. Local paths
retain native identity and remote paths use POSIX semantics. Filesystem traversal
and native program requests stay in injected adapters; Files owns their browser
presentation. Shared directory selection is described in
[ADR 0072](../adr/0072-shared-directory-selection.md).
Monitor and Vault schemas belong to their existing domain crates.
These schemas retain their established serialized names and public schema identities.
The foundation settings schema builder supplies ordered property/default machinery;
policy descriptions, constraints and defaults remain with each domain owner.

## Snippet contracts

The snippet domain also declares blocking SnippetRepository and supplied SnippetIds
capabilities. Storage errors retain the original native source behind a typed read/write
operation; no native paths, entropy acquisition or I/O enters the domain. Record and
library serialization, validation and exact expected-record mutations remain pure.

## Plugins and contributions

`nocterm-plugin-model` owns what a plugin is and what the window shell can host:
ids (their syntax is checked by the catalog), the `SystemPlugin` and `SystemContribution` descriptors that feature
crates declare, placement hints (zone, order, group, automatic placement), group specs,
icons and width classes, and the checked `PluginCatalog`. Core contributions are
functions that are not plugins and are never disabled (the shell's, and those of connections
and files); every other contribution belongs to a system plugin. The catalog refuses malformed or repeated ids, contributions outside
their plugin's namespace, undeclared groups, a group hint on a widget, a set of groups
without exactly one primary group, panel traits on a widget (or missing on a panel) and
repeated order values in a zone, with typed errors. It has no GPUI types, no
I/O and no internal dependencies. See [ADR 0082](../adr/0082-system-plugins-and-core-contributions.md).

Its `schema` module is the declarative settings schema of a plugin: groups of fields of five
kinds (toggle, choice, multi-choice, bounded number and single-line text), with no colour,
secret or multi-line kind. `SettingsSchema::validate` bounds the counts and lengths, checks
that keys, options, ranges and defaults are well formed and that a group names one of the
plugin's own contributions; the catalog calls it and refuses a plugin with a schema but no
`settings_section`. `apply` turns a `FieldEdit` (set a value, toggle one option, or step a number) into the
new value of a field from the value current at write time, so quick edits compose, with a
typed `FieldError` whose text is meant for the field. Text is stripped of control
characters, line breaks and every `Default_Ignorable_Code_Point`, keeps at most two
nonspacing or enclosing marks per base character, and is data: never interpreted or
masked. `bounded` applies the same rules to values read from a stored document. The schema
performs no I/O and knows nothing of storage. A plugin may also name the full settings page it keeps
(`settings_page`), which the application registers nested under the Plugins page or, for a
destination of its own such as the AI page, at the top level. See
[ADR 0085](../adr/0085-one-plugin-model-with-separate-trust-tiers.md) and
[ADR 0086](../adr/0086-plugin-enablement-and-the-plugins-page.md).

`nocterm-plugin-enablement` owns the `[plugins]` settings section: the ids of the plugins
switched off. The list is sanitized (valid ids once each, at most 64; ids this build does not
know are kept, so a returning plugin finds its state) and `effective_disabled` returns
the ids that are really off, never a required plugin and never a core contribution.
`read_error` reports a table that does not read, so a writer can refuse instead of
replacing it. The section also holds the `user_plugins` switch: whether a build with the
`user-plugins` feature loads user plugins at the start (off by default, left out of the file
while off; [ADR 0093](../adr/0093-user-plugins-start-off-and-the-api-stays-compatible.md)). It
reads `nocterm-plugin-model` and `nocterm-settings`, has no GPUI types and no I/O
([ADR 0086](../adr/0086-plugin-enablement-and-the-plugins-page.md)).

`nocterm-plugin-model` is `#![no_std]` with `alloc`, so I/O is unreachable by construction. It also
knows the user tier: `user-` plugin ids, borrowed `PluginRef` and `ContributionRef` views over system and user plugins, the clamp that
bounds where a user plugin may place a contribution, and the `renamed_from` and `resolve` rules
that move a stored setting value to its renamed key. Its schema carries a `version`.

`nocterm-plugin-api` owns the contract of user plugins: the capability table with its field types,
`Snapshot`, `View` (a flat arena), `Effects` and `Event`, `sanitize_display`, the manifest and its
validation onto `SettingsSchema`, grants bound to `(PluginId, ContentHash)`, the template language
with `DeclarativeRuntime`, and the `PluginPackages` and `PluginRuntime` ports. It is `no_std`,
reads no file and runs no guest code; `crates/plugin-api/api-surface.json` is generated from it by
`cargo xtask docs`, and so are the manifest reference and `docs/plugins/manifest.schema.json`
from its typed manifest keys (`MANIFEST_KEYS`). A CI job holds the surface compatible with the
latest release tag ([guardrails](guardrails.md#plugin-api-compatibility)). It also owns the code-package format rules (kind detection from the module
magic and a bounded scan for the manifest's custom section) and `Runtimes`, which picks the
declarative interpreter or the injected code runtime by the manifest's `runtime`. See
[ADR 0089](../adr/0089-user-plugin-contract.md) and the
[isolation rules](guardrails.md#isolation-of-the-plugin-crates).

`nocterm-plugin-abi` owns the guest ABI of code plugins: the export names, the `ABI_VERSION`, the
resource limits, the call and reply wire types and their bounded postcard encoding. It is
`no_std`, reads no file and runs no code, so the SDK can use it on the guest side. Its wire types
embed the public types of `nocterm-plugin-api` (views, effects, events, errors, capability
values) and mirror only observations and setting values, so the golden file is what pins their
bytes.
`crates/plugin-abi/abi-golden.json` records its exports and canonical encodings and is kept fresh by
`cargo xtask docs` ([ADR 0091](../adr/0091-user-plugin-code-runs-in-process-on-wasmi.md)).

`nocterm-plugin-sdk` is the guest side of that ABI: the `Plugin` trait, `Cx` (the snapshot of one
call), a `ViewBuilder`, typed observation markers, effect constructors, the `export!` macro and,
behind the `testing` feature, `testing::Host`, which drives the real dispatch and encoding
natively and applies the host's rules to what comes back. It is `no_std` with `alloc`. The only
`unsafe` code is the expansion of `export!` in the author's crate, and only for `wasm32`. It has no
isolation row, because the structural bans would reject that macro; instead the policy lists it
as guest-only, and it must stay out of the closure of `nocterm`
([guardrails](guardrails.md#isolation-of-the-plugin-crates)). `examples/plugins/focus-timer` is a
plugin built on it, in a workspace of its own
([ADR 0092](../adr/0092-plugin-sdk-and-developer-sessions.md)).

`nocterm-window-layout` owns the shell's layout as plain data and pure rules: the three
zones of the status bar, panel groups with their members and separators (the catalog's,
groups the user made and named, and panels that stand alone as groups of one), the tray of
registered contributions that are not placed, and the panels that open with a layout.
`resolve` arranges the catalog's contributions as a preset asks and places the rest by
their hints; ids the catalog does not know are kept in the preset but never placed, a
contribution is placed once, separators never lead, trail or double, exactly one group
is primary (the catalog's; a preset's own `primary` flag is ignored) and a pinned widget is always placed. The built-in Default is the empty
preset and Swapped is its resolution with every group on the other side; neither is
stored. It also owns the `[layouts]` settings section: stored presets with `p<n>` ids and
checked names, `select`, which falls back to the earlier swapped flag without writing,
and `transition`, which tells a window whether a change of the document is a switch, an
edit or nothing. `resolve` also takes the disabled ids: they stay in the document but are
not rendered, not appended, not opened and listed by `ResolvedLayout::disabled()`, and
separators collapse around them. It reads `nocterm-plugin-model` and `nocterm-settings`, has no GPUI types
and no I/O. It also owns the pure rules of an edit: `edit(settings, legacy_swapped, catalog, target, op)`
returns the new settings and the preset to show, or a typed `EditError`. Editing a built-in layout in
use saves a copy named Custom and selects it, a pinned widget cannot leave the bar, and the result
never has what `resolve` would reshape. A `[layouts]` table that does not read is reported by
`LayoutSettings::read_error` so no edit overwrites it. See
[ADR 0083](../adr/0083-zoned-status-bar.md),
[ADR 0084](../adr/0084-layout-presets-are-app-wide-settings.md),
[ADR 0087](../adr/0087-status-bar-and-panels-editor.md) and
[ADR 0094](../adr/0094-panels-stand-alone-and-the-bar-preview-is-the-editor.md).

## Hubs

`nocterm-hubs-model` owns what the client needs to know about a hub without I/O: `HubAddress` (default port 7443, `https://` dropped, `http://`
refused), `Fingerprint` and `TrustPolicy`, the pure trust decision `decide` for a probed
certificate (pinned, public authority, first use that needs a confirmation, or a mismatch that
can only be removed), the `[hubs]` settings section (addresses, pinned fingerprints,
labels and an optional `HubColor` from a closed set of six theme-relative names; a colour this
version does not know leaves the server unmarked and never drops the record; never secrets) and the ports `HubTrust`, `HubSignIn` and `HubAdminKeys` (the administrator keys waiting for a person to compare their fingerprints). Port methods
return runtime-agnostic futures and fail with the typed, cloneable `HubError`, which keeps the
original cause. The `test-support` feature supplies `FakeHub`, one in-memory hub behind the
access ports. `nocterm-hub-io` implements the ports ([adapter layer](adapter.md), [ADR
0096](../adr/0096-hub-contract-outside-the-workspace.md)).

`source_identity` separately owns captured `HubIdentity`, `AccountIdentity` and
`AccountScope`, shared by Cloud and other domain consumers without a Cloud-to-AI
cycle. The supplied hub authority and account-key fingerprints participate in equality;
namespace text is bounded and literal. These descriptive constructors authenticate
nothing, perform no SDK/crypto work and define no wire serialization. They do not
replace the existing certificate fingerprint and address trust rules.

A window has no environment of its own ([ADR 0099](../adr/0099-windows-have-no-environments.md)), so the crate holds no record of open windows.


## Cloud conversations

`nocterm-cloud-conversation-model` is what a desktop window shows of one Cloud chat: the
neutral `CloudTranscript` (rows, runs, approvals, this device's pending commands) changed only
by `ConversationDelta`s (tool rows carry the core's title and typed `ToolOutcome`), the
`Freshness` the device core reports (`Opening` before the first round, `Current`,
`Unconfirmed` for a runner that did not answer, `Withheld`), the `ConversationAction`s a
person sends (a prompt may name a bound server), the chat's `ServerBinding` (each
`BoundServer` with its `ServerAccess` and whether a prompt may name it), the `ServerChoice`s it
can be bound to with typed `ServerRefusal`s, and the
`CloudConversation` port (with `servers`, `bind_servers` and `unbind_servers`) and its typed
`CloudConversationError`. It holds no protocol types and
does no I/O; `nocterm-hub-io` implements the port over the contract's device core and
`nocterm-cloud-chat` presents it. Its `test-support` `FakeConversation` scripts deltas,
freshness and command outcomes for consumers' tests
([ADR 0144](../adr/0144-the-device-side-runs-on-the-contract-device-core.md),
[ADR 0146](../adr/0146-device-transport-and-resumable-commands.md),
[ADR 0147](../adr/0147-desktop-answers-bound-chats-and-runs-prompts-on-bound-servers.md)).

## Cloud run contracts

`LoginConsentLedger` retains at most 32 device-issued SSH login consents for at
most 60 seconds, consumes the exact original binding before key loading and
rechecks expiry/clock ordering after awaited work. Missing state after restart
requires new consent. `SessionLoginAuthorizer` accepts only checked SSH proof
values; `LoginConsentAuthority` checks actual captured source, run/target and
credential revision. IDs and wire constructors do not authenticate a caller.

`nocterm-cloud-model` owns pure finite grants, run effect ordering, uncertain-mutation
fences and bounded account round-robin admission. Immutable authority retains the
original hub/account source, runner/key epoch, run/epoch, target, selected SSH
key/credential revision, session/epoch and grant/epoch. Grant origin records the
original issuer and durable command/control revision. Supplied time rollback fences
admission; read capability excludes terminal input, execution and connection creation.
A dispatched uncertain mutation blocks new writes until explicit original-intent
reconciliation, including after resume. The host preserves unresolved target effects
across distinct runs; the pure run owner neither performs nor retries I/O.

Its narrow host-controlled ports are `AgentEngine`/`AgentEngineFactory`,
`ScopedToolExecutor`, `RunJournalWriter` and `JournalReader`. They reuse neutral AI
content, tool-call and display records. Engine progress permits bounded text/thought
and optional usage only. Engine `Usage` describes one completed HTTP call after
its original observation is durably reconciled. `InferenceLedger::reported_usage`
sums those original observations independently of conservative reservation charges:
each missing field, including an unobserved call, makes only its cumulative field
unknown; checked addition rejects overflow. An empty ledger is unknown. The host
publishes normalized `CoordinatorEvent::Progress` usage in the same transaction
as each real native Budget reserve/observation record. A reservation therefore
publishes unknown totals without waiting for HTTP completion, and observation ACK
already includes the updated cumulative fact. Later SDK Usage validates that
original step's report and publishes no duplicate. Budget frames retain the full
original ledger from which restoration derives the same optional cumulative fact;
no synthetic native Progress body is created for this transition. At that consumer
port each usage event is a complete optional cumulative run snapshot: it replaces prior values, including clearing known values to unknown.
Consumers neither add snapshots nor combine fields from different snapshots. Zero
is known only when all contributing observations report zero. New active runs reset
usage; a queued prompt does not change the active run's totals. Host step/slot
identities survive opaque checkpoint restore;
provider call ids grant no authority. Checkpoints remain derived run/revision-bound
state. Exact JSON admission reuses AI history's allocation-free sizing owner.

Journal contracts bind expected transactional revision, writer epoch, admitted
command, records, checkpoint and ciphertext outbox in one durable native transaction.
`ChatIdentity` is account plus chat; runner authority in `StreamBinding` cannot create
an alternate partition for that same identity. Run revision, journal stream revision,
local record cursor, public event cursor and user-facing control revision are distinct.
Progress/internal budget commits do not stale user commands; two new commands against
one control revision conflict, while an identical retry returns its original receipt.
Bounded readers reject wrong-source records and gaps. `OutboxDeliveryId` is the
persisted transport deduplication identity; `OutboxEntry::id` remains the signed
record identity. `ResidentPublication` shares one actual allocation generation.
Public encoders reserve their output/native-envelope allowance through
`PublicationWorkBudget`; ownership follows the actual native write/upload task.
`JournalSnapshotPublication` separates stage, stored transport receipt and prefix
activation. `SnapshotCandidate` contains bounded metadata; the independently
charged native execution archive and actual signed public baseline remain separate.
`ResidentSnapshot` retains the returned archive's permit until its last owner drops.
No constructor authorizes dispatch, validates public cryptography or implies a hub
has stored a record ([ADR 0116](../adr/0116-signed-cloud-baseline-publication.md)). Value construction does not
prove persistence, authentication or native cleanup. See
[ADR 0107](../adr/0107-cloud-run-authority-and-host-controlled-ports.md).

Cloud engine context uses `EngineHistory`: bounded neutral user/assistant records
and complete historical tool-intent/result pairs supplied by the authoritative
journal. It excludes execution authority, provider SDK continuation types and
trusted display metadata. `TerminalCall::arguments` supplies one semantic argument
encoding from the AI catalog's DTOs; historical calls cannot dispatch themselves.
`nocterm-ai::local_environment` separates device/account ownership from project or
General membership. `ChatScope` combines those coordinates and `ChatKey` adds the
chat ID. `hubs-model::source_identity` supplies authenticated-source consumer ports;
its identity constructors validate shape, while native adapters authenticate and
invalidate the retained owners. See [ADR 0110](../adr/0110-local-history-environment-partitions.md).

The Cloud journal's `JournalCatalogBinding` fixes one known account and runner
identity/key epoch independently from any chat. `JournalCatalogCipher` binds that
identity and a positive catalog revision to bounded purpose-specific encrypted
catalog bytes. Composition must inject its implementation; DTO construction does
not verify cryptography, source authority or durability. The native adapter owns
complete physical row checks and explicit create/open lifecycle
([ADR 0113](../adr/0113-account-bound-journal-catalog.md)).

A chat summary is runner-derived display data. `ChatSummaryFacts` reduces journaled
observations (first prompt line, latest model, unfinished run statuses, open approvals,
deletion) to a `ChatActivity`; `ChatSummaryTracker` adds the revision ledger and the
cadence (one attempt per chat per 5 s, longer after failures, a deletion at once and last).
`ChatSummaryJournal` and `ChatSummaryDirectory` are the ports for its durable row and its
signed publication; the contract format stays in the adapter
([ADR 0143](../adr/0143-the-runner-publishes-chat-summaries-journal-v7.md)). The facts also
take a bound chat's `SummarySsh` (the binding's signed bytes and the runner's latest access
report), from which the adapter derives the summary's servers.

`ssh_access` holds the runner's rules for a chat's SSH binding (ADR 0145). Reach is selected:
`SshReach` lists the endpoints this runner may reach, each with its host key, which must equal
the binding's. Each `BoundTarget` carries the session access the adapter derived from the
signed binding, and `ChatSshAccess::select` names the admitted server a run's destination
selects by exact equality, with its credential while a certificate is active
([ADR 0147](../adr/0147-desktop-answers-bound-chats-and-runs-prompts-on-bound-servers.md)).

### Displayed account confirmations

`nocterm-hubs-model` owns `AdminKeyOffer`, `RecoveryKeyOffer` and `RecoveryKeyReissue`, opaque consumer capabilities supplied by adapters. Display DTOs remain descriptive. The offers preserve their original recipient and exact recovery action through deferred UI callbacks; typed stale errors cross the port. The domain performs no native source lookup or I/O. See [ADR 0114](../adr/0114-native-captured-account-operations.md).
