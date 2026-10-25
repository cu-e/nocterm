# Domain layer

Part of the [architecture overview](../ARCHITECTURE.md).

Domain crates hold the rules and contracts of one subject and depend only
on other domain crates and the foundation, never on GPUI: `nocterm-session`, `nocterm-terminal-model`, `nocterm-explorer`,
`nocterm-vt`, `nocterm-themes`, `nocterm-ai`, `nocterm-monitor`,
`nocterm-containers`, `nocterm-snippets`, `nocterm-transfers` and
`nocterm-vault`, `nocterm-plugin-model`, `nocterm-window-layout`, `nocterm-project-model`, `nocterm-project-providers` and `nocterm-connections-model`. Adapters implement their contracts ([adapter layer](adapter.md));
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
Jump hosts cross this boundary only as resolved values: `JumpHost` carries target and
authentication and `ResolvedJump` adds an optional saved credential, `ConnectRequest.jumps`
lists them in order up to `MAX_JUMP_HOSTS`, `Prompt.hop` (`PromptHop`) names the
hop a prompt belongs to, and `ConnectStage` names the hop being reached. `HostAddress`
(destination plus jump-host targets) identifies a machine as reached: the workspace
matches sessions, transfer jobs and program hosts by it, not by target alone, and
`nocterm-connections-model::credential_scope` limits a remembered credential to the
connection (and its recents) it was typed for when that connection has jump hosts.
`nocterm-connections-model::jump_route` stores a profile's
ordered jump entries (saved connections or literal addresses) and resolves them
purely at open time, with typed errors for deleted connections, cycles and routes over
the limit ([ADR 0060](../adr/0060-ssh-jump-host-routes.md)).
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
The Linux container platform instead builds own-cgroup v1/v2 CPU/memory recipes.
It resolves process membership and controller mounts, parses bounded facts and
derives CPU as percent of one CPU. Memory retains an optional observed limit;
missing host-only facts stay absent. This reuses the watch's supplied executor and
frame stream, adding no native filesystem or process acquisition to the domain.

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

## Agent profiles

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
Version one remains byte-stable and belongs to General. Version two requires an
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

Account plan usage is read through AI's `AccountUsageSource` contract. `UsageAgent`
carries the agent id and its profile's `AccountUsage` kind, never its launch or
environment, and `UsageSources` routes it to the source registered for that kind.
Rate-limit records in usage-update metadata are parsed by `rate_limit_record` under
the key the profile's `UsageMeta` names. Typed `UsageReadError` retains native causes
as values. Domain `Limits` and transcript estimates remain pure; account requests,
log discovery and their grammars belong to the agent-usage adapter.

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
replacing it. It
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
`cargo xtask docs`. It also owns the code-package format rules (kind detection from the module
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

`nocterm-window-layout` owns the shell's layout as plain data and pure rules: the three
zones of the status bar, panel groups with their members and separators, the tray of
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
[ADR 0084](../adr/0084-layout-presets-are-app-wide-settings.md) and
[ADR 0087](../adr/0087-status-bar-and-panels-editor.md).
