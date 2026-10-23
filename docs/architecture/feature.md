# Feature layer

Part of the [architecture overview](../ARCHITECTURE.md).

Features depend on shared contracts and never on other features or the SSH
adapter. Feature crates use the service, UI, domain and foundation
layers: `nocterm-terminal`, `nocterm-connections`, `nocterm-settings-ui`,
`nocterm-files`, `nocterm-snippets-ui`, `nocterm-monitor-ui`,
`nocterm-containers-ui`, `nocterm-vault-ui`, `nocterm-keymap-ui`,
`nocterm-agent` and `nocterm-agent-settings-ui`.

## Terminal

`nocterm-terminal` owns the terminal model/view and observes session
and settings changes. Private model owners separate session/prompt lifecycle,
transport input and live leases, screen/search presentation, credential candidates
and output recording. The view keeps independent frame-cache, keyboard/composition,
cursor-clock and pointer owners. Selection and scrolling use named model actions;
consumers only receive immutable emulator and painted-frame projections. See [ADR 0038](../adr/0038-private-terminal-state-owners.md). Its renderer decorates otherwise unstyled default-color
output with semantic roles for timestamps, validated IP addresses, versions,
process/user identifiers and important messages. ANSI colors and program styles,
selection, search results and block cursors take precedence. This presentation
never enters clipboard text, recordings or terminal transport. A per-view cache
uses output generation, viewport size, scroll offset and screen identity; blink
and selection repaint reuse roles, while palette colors resolve at paint time.
Only visible rows are examined, joining soft wraps on either screen. Matches
touching a clipped soft-wrap boundary are skipped without reading off-screen
text; complete fields elsewhere in that visible group still receive decoration. Work is
bounded to 64 KiB of text and 65536 examined cells per frame, 4 KiB per logical
group, 512 field matches per group and 4096 retained compact spans. Oversized
groups are left undecorated. Disabling `terminal.semantic_highlighting` releases
the cache and skips parsing; the Terminal settings switch is enabled by default.
Theme ANSI colors are used only when their normal or bright variant meets 4.5:1
contrast against the terminal background, otherwise the original foreground is
kept. Built-in field captures follow the bounded line decoration approach used
by [iTerm2 triggers](https://iterm2.com/triggers.html) and the semantic log fields
in [lnav formats](https://docs.lnav.org/en/latest/formats.html).

Typed session options inherit global TERM, charset, proxy and output-recording
defaults, with explicit profile/quick-connect overrides. A shared options form in
`nocterm-ui` keeps settings and connection features independent. Terminal owns
incremental decoding/encoding at the text boundary. Combining marks retain the
base cell style. OSC 52 clipboard writes are denied by default; opt-in requires
the actual screen to have focus in the active window. Clipboard reads remain
denied. Terminal replies and mouse
protocol bytes bypass conversion. Its output recorder uses a bounded worker,
private capped files, visible failures and an application-quit drain. Neither
authentication answers nor the input stream enter the recorder.

Session Settings reuses the validated session options editor for transient
overrides. Safe reconnect retires the previous session, event pump, prompts and
remote filesystem before starting a new epoch. Credential association failures
retain their original owned concrete cause. A failed history link leaves the
authenticated credential saved; late completions cannot replace a newer prompt.

Terminal input preserves typed charset failures and distinct command, snippet,
live ownership and queue refusals through the workspace seam. Explorer path drops
use a private `FileDropError`; validation precedes one native paste submission.
Agent observation retains terminal causes during asynchronous work until its tool
result. Snippet execution retains target selection and typed failures until the
notice, and never retries a paste after submission if focusing its tab fails.

Terminal implements separate workspace state, text, input, live command, execution,
user sign-in, reconnect and snippet-paste ports through weak Item/model handles.
Agent approvals capture the original session at intake; reusable grants expire on
epoch changes. Reconnect waiters hold their accepted receipt and cannot follow a
later replacement. User card callbacks retain the original prompt token and input;
identical-looking replacement prompts receive a fresh empty field. Background opening
checks live chat, registration and attachment authority before creating a session.
Its private owner captures the original receipt through workspace independently of
chat lifetime; failed handoff closes only that same still-background session, preserving
replacement connections and tabs the user promoted.

## Terminal keyboard input

The view forwards repeat and release events only for accepted terminal input,
tracking at most 128 held keys. Deferred text acquires physical ownership only
when its commit succeeds. Application bindings, platform keys, focus changes,
composition, reconnects and negotiation changes discard stale ownership; key
release does not scroll or clear selection. GPUI text preference leaves AltGr
and dead-key composition to the input method without duplicating characters.
Text-only commits use kitty's unknown key 0 when report-all is requested, with
Unicode text codepoints only when associated-text reporting is enabled. GPUI
supplies logical keys and produced text, but no reliable physical base-layout
key, keypad identity, modifier handedness or lock state. These are never guessed:
known shifted values can be reported, generic modifier names are ignored, and
optional alternate identities are supported by the VT API for hosts that have
them. Platform-key combinations remain application shortcuts.

## Connections, settings and Explorer

`nocterm-connections` owns the grouped sidebar, profile editor, editor inputs,
session-intent conversion and quick-connect menu. `Connections` aliases the actual
`nocterm-connections-store::ConnectionStore` service entity. Pure document formats
and checked transitions belong to `nocterm-connections-model`; the app supplies
preloaded documents and repository writers. Editor, reload and quick-connect
use injected identifiers/defaults retained by `Installed`; recent-use intents
receive their timestamp from the service's injected clock. `Installed` retains the
actual `FactsService` entity. `FactsPresentation` projects immutable bounded flag
bytes to one cached GPUI image per country, with embedded fallback after a terminal
negative result. Raw fact persistence, detection admission and native job reservations
belong to the GPUI-only service. `ObservationDeps` arrives explicitly from bootstrap;
no view performs native acquisition. Weak rotating source projections supply original
Item/session receipts and retry refused work by re-reading live capabilities after
capacity events, without retaining a waiting request/FS payload. Icons and flags
reach other features only as images in `ConnectionSummary`. The empty workspace
projects its six most recent saved profiles through
`ConnectionDirectory`, using current names and the same icon and country policy as
the sidebar. Quick destinations and deleted profiles are excluded. Selecting a
recent server defers opening through the normal authentication path until the
workspace borrow ends. Editor sections own their address/metadata, authentication, jump hosts, launch and appearance
controls and subscriptions. They project one pure domain draft, with a scrollable
active form, multiline description and fixed footer;
typed validation selects the section containing the invalid field. The Jump Hosts
section edits the ordered route from saved connections or `user@host[:port]`
addresses and reports an unresolvable route when saving. Opening a connection
resolves its route from the current profiles in the connections feature, posts a
notice when it cannot, and hands only resolved hops to the session spec. The
terminal labels hop prompts with the hop's address, selects a saved credential only
for the hop it belongs to and never remembers answers to hop prompts
([ADR 0060](../adr/0060-ssh-jump-host-routes.md)). Deleting a connection that has jump
hosts removes its recents rather than leaving plain destinations that skip the route. Folder membership
changes persist a complete profile before publishing new state; explicit folder
names keep empty groups available as drop targets. Group rename, ungroup and
delete are single queued mutations; removed profiles unlink their recents and
vault credentials are left untouched. Profiles use durable checked edits through
`nocterm-store`; recents and detected facts remain optimistic and coalesce. The
injected shutdown coordinator captures profiles/recents together, saves independent
uses before waiting for profile outcomes, and applies only physically successful
prefix/final effects. The service supplies typed publication errors; the domain supplies codecs;
no separate writer loop or quit handler remains. Unknown top-level profile tables
and comments on retained keys round-trip through the existing atomic writer.
`nocterm-settings-ui`
contributes a single settings Item with a page list. The app supplies a checked ordered catalog with
explicit eager/lazy registration and a required default; validation invokes no factories. Each Item
constructs eager pages in order, reuses lazy pages and closes only constructed pages. The ordinary
and startup catalogs retain separate application policies
([ADR0057](../adr/0057-checked-settings-catalog.md)). Every control saves itself:
switches and choices at once, text when typing pauses, on Enter and on blur.
Invalid text stays in its field with the error and is never written. Feature-owned
bindings retain the actual validators, labels and section identities. Shared UI
`SettingsTextField` owns private native input, debounce and checked receipt state;
its callback validates the actual ordered candidate atomically. Publication failure
preserves the literal and shows its error; the same text can be retried with Enter.
Each distinct close/page-deactivation flush is admitted before field membership is
released, including behind a pending write. FieldPresentation retains the narrow
parent redraw observation with its handle; removal retires both. Changes publish
through the original SettingsHandle only after a successful write. Newer drafts are
not overwritten by older completion or external updates; pristine fields follow
published changes without a feedback write. Persistent writes run through bounded ordered
background queues; profile drafts carry the original snapshot, so another window
cannot silently lose its saved changes. Pages share `nocterm_ui::form` sections and rows
on the terminal's background. Recent snapshots
coalesce separately from connection opening. Native quit draining is bounded
best effort within GPUI's shutdown deadline. UI's `SettingsPage`/`SettingsPageSpec` contract lets the composition
root inject the lazy Vault page without feature-to-feature dependencies; the
Vault page has Overview, Credentials, Security and Options tabs, and switching
tabs, deactivation and closure clear typed master passwords. `nocterm-files`
contributes an Explorer Panel which follows `ActiveSessionChanged`. The remote
half uses home/listing requests; both halves own their navigation, selection and
background tasks in private `LocalPane`/`RemotePane` entities. Local I/O is delegated
to injected session filesystem capabilities. Request tasks are dropped when
superseded, and generations reject stale replies. The local cwd bridge goes
through Workspace's `LocalTerminal` contract, without Files depending on Terminal.

## Explorer

Explorer receives explicit `Deps` through `install` and `register`; application
composition retains `Installed` and supplies it to each workspace. Transfers and
its status view retain the supplied TransferService port, so Show Transfers also works before
Explorer is opened. The coordinator handles transfer intents and queue snapshots;
pane entities own their views and emit typed intents without mutating siblings.
Local home discovery, listing, completion, statistics and mutations run through
`LocalFs`/`LocalMutations` on background executors. `ListingState` exposes the
presentation of navigation through a public query, rather than exposing pane state.
Configured local program dispatch uses the injected `ProgramOpener`; the local adapter
owns process spawning and child reaping.

Explorer uses the shared `nocterm-ui` directory input and navigation controls;
the workspace creation picker uses the same completion implementation. Read-only
sources are supplied explicitly through the Explorer domain contracts
([ADR 0072](../adr/0072-shared-directory-selection.md)).
Suggestions use a viewport-constrained popup anchored below the input,
flipping above it when needed, so minimum-height split panes retain usable rows.
Tab and Shift+Tab complete directories from one cached listing; Up and Down
cycle the plain-text popup without descending into a single match.
Editing, blur, navigation and session changes invalidate pending generations.
Enter resolves a literal absolute, relative or home path through the existing
browser loader, retaining the draft on failure. With the popup open, successful
navigation preserves editing and synchronizes its absolute path while retaining
the current focus; a second Enter without the popup closes editing. A shared
Workspace `FileDrag`
contract preserves local paths or pinned remote filesystem/host data. Explorer
uses it for transfers; Terminal quotes its literal paths for the receiving shell
and submits one checked native paste without Enter, independently of the source
host. Wrapper programs can carry a typed receiving-shell syntax through
`ProgramSpec`; Container Shell marks its known Bash/sh launch as POSIX, while log
tabs retain unsupported-program handling. No feature depends on its sibling crate.

Explorer context menus snapshot typed local/remote targets. A mutation dialog runs
I/O on the background executor; completion refreshes only a matching navigation
generation and filesystem instance. New metadata/rename/remove/permission methods
on RemoteFs advertise capabilities and default to Unsupported, preserving custom
transports. Unix local operations traverse retained directory descriptors with
no-follow opens; deletion unlinks links and walks real directories. Recursive
deletion has cancellation points and depth/entry limits, without rollback. Windows
uses a safe atomic no-replace move and bounded path-based deletion; reparse-point
and ancestor checks reject existing links but cannot eliminate active ancestor
replacement races. Native Windows execution remains a separate verification step. SFTP
mutations remain path-based: v3 cannot atomically pin identity between LSTAT and
READDIR/SETSTAT/REMOVE, so concurrent server-side path replacement remains a
protocol limitation. Removing a normal symbolic link never traverses its target.

## Snippets

`nocterm-snippets` owns the serialized snippet library, validation and exact profile/group
matching without GUI dependencies. `nocterm-snippets-ui` registers the sidebar before
containers and projects the active profile through workspace contracts. Snippets for
that profile or its group appear first; all remaining snippets occupy a collapsible
section. Group identities are exact connection group names; unavailable bindings
remain visible in the editor and never turn into global snippets. After renaming a
connection group, reattach its snippets to the new name in the editor. The native
GPUI Kit Editor supplies bundled Shell, JSON, Python, YAML and TOML syntax grammars.
Library writes run in one bounded background queue, rebase checked draft/delete
snapshots on the last persisted state, and publish only after an atomic save succeeds.
Invalid library files remain read-only; their shared load diagnostics retain the
original non-Clone error and transparent native cause through core `SharedError`.
Persistence errors appear in the panel and
editor. The Snippet and Attachments pages keep one draft with a fixed footer;
attachments show searchable server folders with independent direct-profile and
whole-group bindings. Copy preserves code verbatim. Run and a row double-click
resolve the exact last-focused visible user terminal, including the bottom shell,
and share a user paste contract separate from agent execution. Successful submission
returns keyboard focus to that same terminal. The terminal applies
its current bracketed-paste mode and session charset, then appends one Enter after
the paste closing marker. It sends the complete sequence atomically and refuses
authentication, disconnected or alternate-screen states. Shell integration is not
required; SSH and local shells use the same terminal path.

The snippets feature returns one cloneable `Installed` over its exact `SnippetsHandle`,
supplied `SettingsHandle` and draft identity capability. Bootstrap retains successful installation and supplies it to
every workspace/action/editor. Separate installations remain independent; no snippet
global or transitive settings lookup selects the save target. Shutdown registration
failure omits this feature rather than publishing an unregistered writer. SnippetDraft
owns the actual name/description/code controls and original checked-save expectation;
AttachmentPicker owns search, folding and section focus. Save completion advances the
expected record only on success and can close only its original focused dialog. The
viewless library lives in `nocterm-snippets-store`; native document and UUID work
are supplied by `nocterm-snippets-io`. See
[ADR0055](../adr/0055-snippet-storage-boundaries.md), which retains the editor authority
and control ownership from [ADR0050](../adr/0050-retained-snippet-installation.md).

## Monitor

`nocterm-monitor-ui` consumes the workspace's dedicated `MonitorSource` projection.
The selected inner workspace session takes precedence; missing or unsupported
workspace authority never falls back to the unrelated outer session or local runner.
Ordinary local contexts retain the injected local policy. Cache identity includes
the original surface and checked resource; watch generations reject stale detection
and readings across switches or reconnects. It keeps bounded samplers and runs
exactly one script: the status
bar's metrics at the slow interval while the details are closed, and the union
with the details' metrics at the fast interval while they are open. It adds a
leading footer view through the workspace and supplies its Settings page as a
`SettingsPageSpec`. Local host execution is an optional `HostExec` supplied at
installation and retained by its model. Container monitoring selects pure
own-cgroup collection through its already scoped executor, with CPU explicitly
relative to one CPU and memory current/optional limit. Host-only readings remain
absent; unobservable limits never become a host memory percentage
([ADR 0077](../adr/0077-selected-workspace-monitoring.md)).

## Workspaces

Workspace removal uses a compact native confirmation. Folder and unknown-provider
locations remove metadata only. Docker/Podman locations offer mutually exclusive
Keep (default), Stop and Delete choices with their consequences. Stop/Delete retain
the original unscoped host and terminal epoch, require a live matching capability,
and affect other workspaces sharing that container. Busy controls prevent duplicate
submits and dismissal; errors stay inline. Known completed effects remain visible
when catalog publication fails, and Retry removal publishes metadata only. Images,
volumes, bind-mounted host folders and chat history are retained; Delete loses the
container writable layer. The feature never closes or retargets open surfaces
([ADR 0078](../adr/0078-explicit-workspace-container-removal.md)).

`nocterm-projects` presents the saved workspace library and a branching creation
wizard. Its private navigation owner projects Connection, Project and conditional
Tools steps without duplicating retained input entities or setup mode. Connection
validates only resource selection, Project validates name/directory/repository,
and Tools appears only for new containers. Next and Back do not prepare resources;
final Create retains full validation and the existing preparation/publication
lifecycle. Step changes move focus and Back preserves drafts. The body fits normal
720p windows; only small windows use body scrolling, with actions fixed outside.
Direct
connection uses the shared directory picker when the captured source exposes a
filesystem. Existing container selection combines independently acquired Docker
and Podman results; creation selects the engine, image and supported tools.
The form retains its original host authority while another tab is selected.
Without a selected server, an injected local executor enables all three modes on
This computer without opening a terminal. Active remote workspaces and selected
disconnected or stale remote terminals refuse creation instead of falling back to
local capabilities. Container rows preserve full engine/ID selection independently
of truncated names, image labels and status summaries.
An independent `ContainerPicker` owns captured inventory discovery, filtering and
exact selection. Search matches names, full IDs, images and engine without I/O or
changing a filtered-out selection. The fixed shared header retains the original
server. Image sources use a single full-width bounded result viewport.

Projects retains Workspace for outer ProjectView admission, Panel/action
registration, focused source capture, General Item enumeration and connection
directory projections. Its ProjectTerminal contracts and context errors are
imported directly from workspace-ports; container Engine/Image/Container remain
domain DTOs ([ADR0079](../adr/0079-project-terminal-contracts-below-shell.md)).

Projects retains its supplied `SettingsHandle` and provides the lazy Workspaces
settings page. Shared checked text receipts edit the domain-owned
`WorkspacesSettings` section. A creation window snapshots the container directory
default and resolves empty Attach/Create paths to it; saved locations and existing
drafts are unchanged by later setting publications.

Independent ImagePicker and LanguageChooser entities own image and version drafts.
Immediate local filtering shares no failure state with debounced registry search.
Search retains the source and engine, invalidates obsolete generations and uses a
bounded 16-entry, 90-second cache; recognizable rate limits impose a 60-second
cooldown without automatic retry. Complete manual references remain usable.
Language rows preserve custom version drafts across enablement, render the Rust
mark with theme foreground and project checked selections into preparation. The
shared chooser also serves explicit existing-environment tool changes. Custom
images are checked against their actual platform before installation. Image and
inventory lists have their own bounds within the relevant step
([ADR 0074](../adr/0074-captured-host-image-search-and-language-versions.md)).

Workspace settings retains exact original surface/session authority and separately
projects observed environment facts, installed versions and tool-change status.
Metadata Save does not install languages. Disconnected remote targets remain
descriptive until ordinary connection recovery; a scoped executor is never treated
as an original host executor. Library rows show the server caption without an OS
icon and project a compact Docker/Podman badge from the checked locator. The whole row
uses the Containers hover treatment; settings and removal actions appear only
while that row is hovered. Footer progress
shows observed layer bytes or an indeterminate phase, independently of action labels
([ADR 0075](../adr/0075-observed-workspace-environment-management.md)).

Preparation is separate from durable catalog publication. The view presents
progress and typed failures without owning process execution or a second writer.
Saved locations resolve through an explicitly supplied provider registry; an
unknown provider keeps its workspace and history but cannot acquire a terminal.
Container locations expose no host filesystem fallback. Inner terminals retain
the normal terminal lifecycle and their original workspace surface
([ADR 0071](../adr/0071-provider-workspace-locations.md)).

## Containers

`nocterm-containers-ui` lists the active host's containers in a sidebar panel
whose switcher shows the running count, re-listing whenever the engine reports
a change. A container's log and a shell inside it are `ProgramSpec`s opened
through `Workspace::open_program`; the terminal runs them with
`ProgramTransport` over `HostExec::terminal`, so no feature opens a
connection of its own and the tab follows as the same host.
Destructive confirmation captures the host executor, engine, object IDs and
connection epoch when the dialog opens. Switching hosts or reconnecting
invalidates that confirmation; completion refreshes only its original context.
Its optional local executor is supplied at registration; another window cannot
replace it through workspace service lookup.

## Vault

The vault feature receives `Deps::new(Arc<dyn VaultAccess>)`; `install`
returns an owned `Installed` handle retained by the composition root and reused
by every window. Settings page factories and action handlers capture that handle,
so views and credential bridges do not read a vault global or import adapters.
Installation retains the supplied settings service as well as the vault service
and its published-policy subscription. Auto-lock changes read that exact published
owner; page factories and the startup prompt retain the same authority. Each view
owns its cancellable status subscription. The feature handles
`workspace::UnlockVault` and `workspace::LockVault`
application-wide, so the palette and a terminal's sign-in prompt can unlock the
vault in a small dialog without opening Settings and without depending on the
vault feature.

## Agent

AI is gated by `Settings.ai.enabled`; switching it off tears down prompts,
permissions, tool registrations, processes and panel visibility across windows.
Chats are saved one file each in a private state directory and reopened through
the agent's session resume/load; fork uses `session/fork`. All three are sent only when
advertised. Favorites have a separate state file. Usage limits come from Claude's
`usage_update` metadata and Codex's session logs through an injected
`AccountUsageSource`, as each agent's profile says. See [AI agents](../AI_AGENTS.md) for protocol, limits and
privacy boundaries. On Linux, `nocterm_ai::sandbox` validates supplied path facts
and derives the ordered mount policy; the ACP adapter resolves native paths and
launches bubblewrap. Startup uses the injected `WorkdirResolver` to prepare the
canonical directory shared by the process and session. Fresh launch validation
rejects changed or protected directories and unavailable bubblewrap without an
unisolated fallback. A changed isolation setting restarts running agents.

The AI agents settings page lives in `nocterm-agent-settings-ui`. Composition gives it
the `SettingsHandle`, the installation's `AgentsHandle` and the `SandboxProbe`; it
edits the `ai` section only through AI's validation rules. It reports availability while keeping the isolation controls visible; launch
checks availability again rather than trusting the displayed result.

A chat's input — the draft, the attachments and images for the next prompt
and the queue of prompts waiting to be sent — is its `Composer`, kept apart
from the conversation and the session ([ADR 0009](../adr/0009-thread-composer.md)).
Its state is private. Prompt admission prepares a candidate queue, validates structural
and storage budgets without mutation, then commits the queue, draft and image changes
as one transition. Editing retains the original context until the edit ends.

A private `SessionDriver` owns the lease, session phase, visibility, pending restore,
reported agent information and outstanding-operation counter. Open, authentication,
prompt and cancellation operations return their captured ticket, connection, session
and registration authority to named coordinator actions. Late completion cannot
mutate a replacement session; an undelivered open closes its original agent session.
Holding work remains visible until its original guard ends, including after lease release.

A private `ToolRuntime` owns approval admission, fixed turn context, structured command
jobs, live observations, terminal handoffs and tool-display state. Its nested owners
return snapshots and named effects rather than mutable state. Fixed prompt attachments
do not follow later composer defaults. Jobs retain their original executor identity;
cancellation preserves bounded readable outcomes. Terminal sign-in waits retain their
prompt epoch, reconnect cleanup retains its original receipt, and background cleanup
preserves terminals promoted into ordinary tabs. Existing document, configuration,
composer and archive owners remain separate from these responsibilities.

A private reconnect dispatch owner moves accepted calls and their original busy
operation guards into a receipt-scoped sequence. Each waiter keeps its own timing;
readiness cannot overtake an earlier joined call. One ready head is consumed at a
time, with original session, registration, workspace and live window checked again
immediately before dispatch. Cancellation drains responders through recorded tool
completion. Ordering ends at synchronous launch rather than process completion.
This covers calls that joined the same accepted reconnect receipt; later directly
admitted calls after connection retain their existing behavior. No new acceptance
cap or absolute pending-memory bound is imposed.

`ThreadNotice` owns status text and failure emphasis, while `ThreadBindings` retains
the originating window and subscription lifetimes. Four `AgentThread` coordinator
files route lifecycle, incoming events/completions, durable input/history actions
and outgoing dispatch. Finite operations own captured request authority, timing
and delivery state; weak chat updates invoke named probes or completions rather
than leaf algorithms. Turn context consumes supplied workspace observations;
native terminal dispatch uses the original accepted capability and session.
See [ADR 0044](../adr/0044-private-thread-coordination.md).

A private `ChatConfig` owns per-chat desired choices, reported options and modes,
slash commands and their revision, prompt model labels and bounded pre-open controls.
It accepts asynchronous replies only for its current session ticket. Opening reports
retain desired choices until reconciliation finishes; each available choice is tried
once per ticket and reevaluated after dependent option replies. Only matching-session
pre-open controls are applied. The coordinator in `session.rs` performs requests and
persistence effects, saving accepted choices before dispatch. Configuration is separate
from the transcript reducer, and restart snapshots retain preview data while excluding
pending controls and attempts ([ADR 0034](../adr/0034-chat-config-composer-owners.md)).

A private `ChatDocument` owns identity, metadata, transcript, semantic revisions,
publication diagnostics and cached persistence budgets. Composer and configuration
remain separate owners. Accepted durable row or metadata transitions advance the
revision before timestamping; snapshots, clean browsing and capture do not. A revision
records accepted activity, so changing a saved value and changing it back can remain
dirty. Matching updates and view-only plan, usage or token changes remain clean.

The document compares affected saved rows before mutation. Only the codec's bounded
transcript window is durable; normalization supplies concrete patches to the existing
tool-display metadata key. Literal drafts and queued payloads share their immutable
storage while metadata projections retain no second transcript/media baseline.
Snapshot framing uses the existing identity and timestamp. The transcript reducer
receives timestamps from its coordinator. Each panel
and chat retains explicit host collaborators through ChatHost, including a
ChatMetadataSource injected by root and shared by all windows. New/fork identities
and timestamps are supplied to pure document transitions; version-one
protocol-backed history retains its current representation
([ADR0040](../adr/0040-native-chat-metadata.md)).

Deferred normal offers coalesce while preserving an immutable candidate and its
semantic revision. Capture offers pending work without consuming it. Admission to
the shared writer and physical publication have distinct acknowledgements; retry
retains the same revision/snapshot, and old failures cannot pause newer desired work.
Normal panel removal captures before releasing the document, retaining the owner when
admission fails. Archive hydration applies an already accepted draft overlay with its
original revision and timestamp; eviction requires settled durable state.

Disabled presentation is released only after the current immutable candidate is
admitted or the document is settled; rejected admission retains its input. Physical
failure still retries that original admitted snapshot. Normal archive eviction
keeps its stricter settled-state gate. See [ADR0037](../adr/0037-chat-document-semantic-revisions.md).

Archived chat hydration belongs to a private feature `Archive` owner
([ADR0031](../adr/0031-archive-hydration-owner.md)). It queues typed open, fork,
rename and pin intentions and accepts a result only for its local load generation
and the coordinator's current session/document ownership. Literal draft edits
replace the saved draft and timestamp together. The runtime loading permit remains
held through hydration and resident trimming; the panel releases it through a
named command. Untouched archives yield no quit snapshot, while edited archives
yield a draft-only patch through the shared storage service.

Provider permissions and terminal bridge approvals belong to a chat's private
`ApprovalInbox`. Its provider queue owns each responder until one valid choice
or cancellation; automatic provider policy selects only an unambiguous `AllowOnce`
option and never grants terminal authority. Provider and bridge cards share a
monotonic token sequence, preserved when queues drain. Panel cards carry immutable
projections and dispatch to the original weak chat owner and token, so stale actions
cannot approve a different queue entry or a different chat.

The coordinator supplies fresh session and registration gates for each transition,
records bridge activity before its response or execution, and retains terminal
resolution, executor identity and live-command lease checks. Terminal grants are
cleared on attachment removal and session release; unsafe live input always needs
an explicit choice. Session release cancels queued approvals while retaining the
conversation and document-owned execution resources.

Configured local Explorer editors use the injected `ProgramOpener`; feature code
performs no native process spawn or wait. Launch errors use an App-owned task and
remain scoped to the original window even after the pane is removed. Configured
remote editors retain the host-session route. Default-application/file-manager
operations continue through GPUI's platform open/reveal surface.

Explorer refreshes the affected destination after a finished transfer that acknowledged a
file publication or destination directory preparation, including transfers that later fail
or cancel. Empty-directory copies therefore refresh without being counted as completed files.

Agent presentation receives immutable `Installed`: the actual RuntimeHandle, image
capabilities, shared layout/settings handles, metadata and optional terminal auth. Five private feature owners retain
the actual controls and their subscriptions/tasks: MessageList, ComposerView,
HistoryColumn, PanelLayout and MenuView. Original chat/input/search/catalog/list
identities guard deferred completion. MessageList reuses one ImagePipeline for
preview cache generations and remeasures rows only for accepted delivery. Attachment
preparation preserves chat/lifecycle/composer revision checks and admits a complete
validated candidate collection atomically. The feature never decodes image data.
Panel coordination retains twelve private fields across four implementation files.
ChatList binds selection and per-chat observation to the original entity; removal
commits only after existing document admission. SignInCards retains original-token
secret inputs, PanelBindings installation lifetimes, and PanelNotices the actual
cancellable 33-ms redraw task and approval fingerprint. Stateless renderers consume
immutable projections; finite preview/image operations return typed results to named
completions. See [ADR 0049](../adr/0049-bounded-panel-coordination.md).

The connections service schedules injected `RemoteOsDetector` work and owns
probe admission, once-per-session decisions and publication/completion authority.
The feature projects original session receipts and presents checked identities
with its OS glyph/color catalog and manual-icon precedence. Release parsing and
remote read/close work belong to domain rules and the native adapter.
See [ADR 0048](../adr/0048-guarded-connection-facts.md).

Terminal models retain immutable `Installed` dependencies: remote transport,
optional local-session factory, supplied default shell, optional credential
provider and recording registry, and the actual `UiReady` settings handle.
`install` requires complete providers; `register` installs a private factory in
each workspace. Reconnects, background tabs and isolated command tabs use their
original installation. Missing local providers produce the existing unavailable
session result; explicit command and program transports take precedence. Model
and view subscriptions observe published settings through weak entity handles,
without discovering a settings or transport global. See [ADR 0042](../adr/0042-terminal-installed-dependencies.md).


Connection views now receive a retained `Installed` handle with their exact
profile/facts entities, editor inputs, explicit settings and observer lifetimes.
Directories, editors, menus and group actions capture those owners; connection
and facts globals are removed. Session intents remain feature conversions over
the unchanged workspace session factory. Country policy uses the supplied
SettingsHandle. Both profile/recent and raw-fact publication belong to the supplied
service entities. Feature state owns image presentation and original Item/session
source projections; admission/completion authority is checked by the service.
See [ADR0043](../adr/0043-retained-connections-installation.md).

`TerminalCapabilities` provides optional `ConnectionFactSession` alongside its other
narrow consumer ports. The terminal's existing weak Access owner reads one actual
session/ExpectedSession snapshot. Connection source authority compares captured epoch,
full target and original FS directly through this receipt, independent of window
membership/cursor lifetime. Closing the original Item calls its normal disconnect
transition; promoting/transferring the same live Item keeps authority. Unsupported
Items expose no port; no broad Item/ItemHandle/TerminalState API was expanded. The
shared workspace test-support SessionProbe supplies the same receipt and named
close/connection/epoch transitions; its mutable state remains private.

Connection projection callbacks keep weak connection/facts handles and upgrade them
only at execution; App-deferred work does not retain Installed through teardown.
Editor country-input subscriptions capture the narrow weak raw-facts service rather
than the installation, preserving owner-owned subscription cleanup without nested
unsubscribe during callback destruction. UI owners retain the installation itself.


The keymap page receives its actual KeymapHandle from the root's lazy page factory.
Its private search, recording and per-view message remain window-local; a retained
weak observation subscription refreshes shared desired bindings and typed native
save errors across windows. Deactivating a page releases interception; closing also releases
observation. It edits only through named handle operations, without a global owner
or mutable model projection. New windows reuse the root installation rather than
reloading the file. See [ADR0051](../adr/0051-retained-keymap-installation.md).
Agent panels and threads retain one `Installed` through their original weak-workspace
ChatHost. Runtime commands, deferred preferences/usage and terminal authentication
use that same installation, while policy is read from its supplied SettingsHandle
at the existing admission/completion boundary. The installation builds one
`AgentsHandle` from the supplied catalog; it registers the AI schema before agents
are resolved. Installation retains a weak-runtime settings subscription;
per-window subscriptions retain weak views and original window handles. There is
no runtime/auth or shared-UI AI policy compatibility lookup. Shutdown registration
status remains separately typed and does not discard a usable installation. See
[ADR0052](../adr/0052-retained-agent-installation.md).

Settings tab installation retains the supplied SettingsHandle. Feature-owned
pages retain their supplied settings and capabilities; Agent retains SandboxProbe.
Builtin fields, page controls, section diagnostics and settings observation
use that exact owner; window/page factories clone the installation. Per-window
observation keeps a weak view and originating window. Appearance picker callbacks
retain the same supplied policy through deferred opening and completion. Text fields
keep their 600 ms debounce and Enter/blur/deactivate/close triggers. Shared
SettingsTextField submits checked edits synchronously, observes physical receipts and
preserves newer draft generations when older completions arrive. Parent presentation
subscriptions render receipt diagnostics without requiring a settings publication.
NumberInput and Vault auto-lock controls retain their existing detached persistence
behavior. Settings consumers retain their supplied service handles. See
[ADR0053](../adr/0053-retained-settings-ui-installation.md) and
[ADR0054](../adr/0054-checked-settings-text-receipts.md).
Explorer panes and its settings page retain their original installation's settings
owner. Indexing reads policy at the existing scan request phase, and a local pane
captures initial configured cwd before resolving home while retaining the original
navigation guard. Opening a context menu snapshots its label; selecting its file
open action reads the latest published policy from that same installation during
deferred execution. The pinned file target, native program opener and original
workspace/window stay fixed. A failed settings publication cannot change that rule.

Monitor installation registers its schema on the supplied settings owner before
models/pages read it. Per-window models retain that handle with the exact supplied
host, and weak publication subscriptions refresh the existing plan. Summary/details
read through their model; page editors use the same supplied policy. One watch,
metric union, host/session identity, eight retained histories and ten-second retry
behavior remain the model's responsibility. Other settings consumers retain their own supplied handles.
VaultView keeps its request generation, service and native-device authority private.
A private VaultForm owns the actual masked password/confirmation and auto-lock inputs
and local numeric validation. A confirmed submit clears both native password controls
before constructing the service future. Tab/lock/deactivate/close transitions use that
same control owner; late completion still checks the original view generation. Form
state stores no extra plaintext copy or policy cache. Auto-lock retains its existing
Enter/blur submission and detached physical-failure behavior.

## Owner settings pages

Terminal supplies its terminal, local-shell and SSH pages. Agent supplies the AI
page through its retained installation, including the pure SandboxProbe display
port. Shared UI owns Appearance widgets and the theme browser. Settings UI hosts
these supplied factories and feature guest pages; it does not own their controls,
policy validators or native dependencies. Builtin controls are created eagerly with
the Item, while guest pages retain their lazy creation and reuse. All builtin pages
flush before selection and before the current guest deactivates. A per-Item UI
feedback handle preserves the shared save diagnostic without exposing the host view.
The app retains workspace action registration for appearance pickers, preventing a
UI-to-workspace dependency cycle.

Live shell/search facts, remote listing generations, scan presentation and completion cursors
remain private owners with immutable queries. Captured container/snippet operations retain
their original host and paste authority. Agent jobs assemble their own original-executor
runner and expose readable immutable outcomes; sign-in cards retain the original prompt.
