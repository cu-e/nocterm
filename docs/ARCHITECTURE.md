# Architecture

The application is the composition root. `src/main.rs` loads paths, settings and
design tokens, installs SSH/local transports and the vault service, and registers independent workspace
features. The [dependency map](architecture/dependencies.md) is generated from
Cargo metadata; `cargo xtask architecture` enforces its declared layers and
rejects runtime GUI dependencies below the UI layer. The
[architecture and security audit](ARCHITECTURE_SECURITY_AUDIT.md) records findings,
regression evidence and residual constraints.

`nocterm-vault` declares the device-unlock provider contract and owns envelope,
binding and cancellation rules. The composition root injects
`nocterm-device-unlock`; native adapters remain outside the vault and its UI.
An optional `nocterm-vault-broker` Linux system service enforces fingerprint
verification for user-bound memory-only keys. See [device unlock](DEVICE_UNLOCK.md)
for the platform boundaries and explicit installation.

`nocterm-core` provides paths and atomic, comment-preserving TOML persistence.
`nocterm-settings` owns the configuration schema, defaults, ranges and storage.
`nocterm-design` owns typed visual tokens and theme overrides. These foundation
crates have no UI or transport dependencies.

`nocterm-session` defines transport-independent targets, authentication,
commands, events, prompts and the `RemoteFs` contract. `nocterm-ssh` implements
that contract using russh and lazily opened SFTP channels on a dedicated Tokio
runtime. `nocterm-vt` turns a byte stream into a terminal grid using
alacritty_terminal and encodes keyboard, mouse and paste input. These domain
crates do not know the workspace or GPUI. `nocterm-local` adapts portable-pty to
the same session events and launch contract. Integration supplies bounded OSC 7
cwd and prompt markers without parsing rendered prompt text.

Input and output channels are bounded. An independent broadcast close signal
interrupts blocked SSH I/O and local PTY writes/output delivery. Unix local PTY
reads/writes are nonblocking and independently cancellable, including when a
background job retains its slave descriptor. Local closure terminates the shell
and current foreground process groups and reaps the owned child; it does not
enumerate and kill arbitrary detached jobs. Explicit input refusal
is surfaced in the terminal instead of silently discarding text.
HTTP CONNECT/SOCKS5 tunnel creation belongs to the SSH adapter, inside the same
connection timeout and target host-key policy as a direct connection.

`nocterm-ui` projects tokens/settings into the standard GPUI Kit theme and
provides shared icons, terminal styling, standalone drag previews and operational
notices. Optional toolkit button metrics are projected from design tokens once;
views retain standard component variants. Notices use the toolkit's window-local
notification list with stable operation keys and recovery actions, published by
operation events rather than rendering. Drag previews explicitly carry their
theme and typography because GPUI renders them outside the application root.
`nocterm-workspace` owns window
layout, tabs and sidebar switches through a native DockArea/DockSkin. The
toolkit owns the single pane tree and drag previews; an adapter exposes Items as
dock Panels. Presentation aliases are separate from Item/session titles. Shared
actions operate the same dock for keyboard split/reorder/focus. Tab context-menu
closures snapshot the clicked Item and resolve its current pane order at execution;
closing adjacent/other tabs cannot cross pane boundaries. The full-width workspace
footer owns section switches and status views. Removing the last bottom Item also
removes its Dock instead of merely emptying the tab list. Hiding the local dock
detaches its panel without closing the Item; showing it restores the same Item
and dock height. Bottom local
terminal focus preserves the last central remote context. An `Item` supplies tab content, focus and an
optional `SessionContext`; a `Panel` supplies sidebar content. Features register
actions rather than making the shell depend on them. `SessionSpec` and a session
opener connect requests from the connections feature to the terminal feature.
Cached session contexts avoid reading an Item reentrantly during its own render;
matching connected contexts also support transfer retry while a utility tab is active.

Features depend on shared contracts and never on other features or the SSH
adapter. `nocterm-terminal` owns the terminal model/view and observes session
and settings changes. `nocterm-connections` owns persisted profiles and recents,
the grouped sidebar, profile editor and quick-connect menu. Its `ServerFacts`
global keeps what was detected about servers (system over SFTP, country through
GeoIP for public addresses) in the state directory's `servers.toml`, apart from
`connections.toml`. Flags are cached in memory and under `flags/`. Icons and flags
reach other features only as images in `ConnectionSummary`. Editor sections retain
one draft, with a scrollable active form, multiline description and fixed footer;
typed validation selects the section containing the invalid field. Folder membership
changes persist a complete profile before publishing new state; explicit folder
names keep empty groups available as drop targets. Group rename, ungroup and
delete are single queued mutations; removed profiles unlink their recents and
vault credentials are left untouched. `nocterm-settings-ui`
contributes a single settings Item with a page list. Every control saves itself:
switches and choices at once, text when typing pauses, on Enter and on blur.
Invalid text stays in its field with the error and is never written. Each save is
an edit queued with `edit_settings` and applied to the settings current when the
write runs, so quick successive edits and other windows never conflict; changes
are published through SettingsStore only after a successful write, and fields not
being edited follow changes made elsewhere. Persistent writes run through bounded
ordered background queues; whole-draft saves (`save_settings`) still carry a
revision and profile drafts carry the original snapshot, so another window cannot
silently lose its saved changes. Pages share `nocterm_ui::form` sections and rows
on the terminal's background. Recent snapshots
coalesce separately from connection opening. Native quit draining is bounded
best effort within GPUI's shutdown deadline. Workspace's `SettingsPage`/`SettingsPageSpec` contract lets the composition
root inject the lazy Vault page without feature-to-feature dependencies; the
Vault page has Overview, Credentials, Security and Options tabs, and switching
tabs, deactivation and closure clear typed master passwords. `nocterm-files`
contributes an Explorer Panel which follows `ActiveSessionChanged`. The remote
half uses home/listing requests; the local half owns navigation, selection and a
cancellable background logical-size scan. Request tasks are dropped when
superseded, and generations reject stale replies. The local cwd bridge goes
through Workspace's `LocalTerminal` contract, without Files depending on Terminal.

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

`nocterm-transfers` owns bounded batch discovery and four streaming workers,
32-KiB chunks, progress, cancellation and explicit retry. A drop pins Target,
RemoteFs and destination. Reservations coordinate same-name sources inside a
batch. Uploads pipeline at most eight acknowledged chunks. Downloads stream through
bounded remote readers and stage local files before atomic publication. On Unix,
local traversal/open/publication use directory capabilities and no-follow opens
for every ancestor. Remote unsafe names, links and special files are rejected or
skipped. Windows reparse checks do not provide the same active-ancestor-race guarantee.
The SFTP adapter stages
owned temporary files and publishes using negotiated rename capabilities, with
cleanup on cancellation/drop/graceful disconnect. Explorer contributes a progress
Item and always-visible status; the service outlives sidebar visibility.
SFTP v3 cannot atomically enforce no-follow identity between remote lstat/open;
server-side mutation remains a protocol limitation. Expected file sizes detect
short/growing sources, but same-size concurrent content mutation is not detected.
Each directory listing remains a Vec even though file streams and discovery queues
are bounded. Readers/writers belong to the SFTP session and close on disconnect.

`nocterm-vault` owns a versioned authenticated encrypted envelope and a bounded
worker; `nocterm-vault-ui` exposes create/unlock/lock/rotation, auto-lock and a
credential provider contract. Profiles store opaque IDs. Authentication asks this
provider only for matching password/key-passphrase bindings; explicit Remember
commits after Connected and excludes MFA. Epoch checks invalidate queued results
on lock. See [security/lifecycle notes](VAULT.md).

Terminal line metadata is attached inside the patched emulator Cell representation
and follows grid moves/reflow/history. `nocterm-vt` projects logical IDs and UTC
observation timestamps separately from text. TerminalView allocates a gutter
outside PTY columns/selection and hides it in alternate screen. See the
[maintained patch](../vendor/alacritty_terminal/NOCTERM.md).

## Extension points

The composition root supplies Session/Edit/Search/Window/Help descriptors to
Workspace's native `AppMenuBar`. `ItemCommand` routes feature operations without
Workspace depending on Terminal. Each window snapshots enabled/checked state
before opening; an open popup retains its originating Item. A focused bottom
terminal takes priority over the central Item. Native clipboard/edit actions
continue to dispatch to native text fields, including dialogs. The maintained
[GPUI Component patch](../vendor/gpui-component/NOCTERM.md) updates menu snapshots
without replacing native trigger entities or process-wide menu state.

VT's literal Unicode search uses bounded KMP scan steps over grid cells and their
combining characters, with memory proportional to the query rather than the
number of matches or history size. It supports case sensitivity and Unicode
whole-word boundaries, preserves soft wraps and can match an explicit hard line
break. Regex mode joins soft wraps but matches only within each complete logical
line. It limits each line to 64 KiB, caps compiled regex size and returns an
explicit error when a limit is exceeded. Regex batches run off the UI thread;
Terminal schedules scan slices and invalidates matches when output, geometry or
screen changes. A rapidly changing buffer can defer a complete result, but
stale highlights are cleared and input remains responsive. Search highlights
and clipboard selection are independent.

Session Settings reuses the validated session options editor for transient
overrides. Safe reconnect retires the previous session, event pump, prompts and
remote filesystem before starting a new epoch. The application window factory
reuses global services rather than initializing another vault or transfer queue.

The command palette (`workspace::command_palette`, Ctrl+Shift+P, Cmd+Shift+P on
macOS) lists no commands of its own. When it opens it asks GPUI for the actions
available where focus is — the focused view's, its ancestors' and application
handlers — keeps nocterm's namespaces (`workspace`, `terminal`, `connections`,
`files`, `agent`, `vault`), names them from the action (`workspace::OpenSettings`
→ "Workspace: Open Settings"), searches their `actions!` documentation as well
and shows the shortcut. The chosen action is dispatched from the view that had
focus, exactly as its shortcut would be, so a feature makes a command available
in the palette by declaring and handling an action, nothing more.

Widths the user drags resizable columns to, and layout choices, are kept by
`nocterm_ui::LayoutMemory`, a global keyed by name and saved to `layout.json` in
the state directory. The workspace body lays out only its visible columns —
sidebar, tabs, side panel, or mirrored when `SwapSides` put the side panel on
the left — with one resize state per arrangement, and remembers widths per
column rather than per position: a hidden column kept in the resizable group
would hold its stale width and stop its neighbours from growing. The side panel
learns its side through `RightPanel::set_docked_left`; the agent panel puts its
history column on the outer side and remembers that column's width. A side
panel that opens a column of its own asks the workspace to widen it with
`RightPanelEvent::Widen`. Sidebar panels are shown and hidden by their feature's
action (`files::ToggleExplorer`, `connections::ToggleServers`) through
`Workspace::toggle_panel_of`, so the workspace binds no feature's panel.

Key bindings are data. `nocterm-keymap` (UI layer) owns the keymap format, the
merge of the application's defaults (`assets/keymap.toml`) with the user's
`keymap.toml`, which lists only differences (`"none"` unbinds), and installs the
merge in GPUI. Its bindings carry a metadata mark, so a change replaces exactly
them without a restart and leaves the component library's own bindings alone.
It also names actions for people (`humanize`) and decides which are user
commands (`offered`), for the command palette and the keymap page alike.
`nocterm-keymap-ui` is the Keymap settings page, a guest page like the vault's:
a searchable table that records new shortcuts through a keystroke interceptor
(so even bound keys are captured) and edits only through `nocterm-keymap`.

The vault feature handles `workspace::UnlockVault` and `workspace::LockVault`
application-wide, so the palette and a terminal's sign-in prompt can unlock the
vault in a small dialog without opening Settings and without depending on the
vault feature.

Add a new Item for another kind of tab, a Panel for another sidebar section or a
registered action for a command. Observe the terminal model or session contract
for integrations. Add another Transport implementation to support another kind
of session. Theme changes belong in the token source or its user override.
IDE features, collaboration, Vim mode and an extension runtime remain future
consumers of these boundaries.

The ACP AI panel uses three layers: `nocterm-ai` declares runtime-neutral agent
contracts and bounded context/tool rules; `nocterm-acp` supplies subprocess and
authenticated local bridge adapters; `nocterm-agent` supplies GPUI lifecycle and
chat UI. App injects the adapter. Workspace exposes allowlisted `TerminalAccess`
and `ConnectionDirectory` seams so the agent feature never imports other
features. The right panel lives outside the tab dock and remains available with
no tabs; maximize uses the working area while preserving the footer.
`ConnectionDirectory` methods take the workspace as a weak handle and are called
outside workspace updates, because opening a connection updates the workspace.

Workspace also keeps sessions without a tab. `open_background_session` asks an
installed background opener (Terminal's) for an Item it holds outside the dock;
such sessions appear in `terminals()` marked `background`, can be moved into a
tab with `show_background_session`, and end with `close_background_session`. The
agent opens them for attached saved servers that have no session; a chat closes
those it opened when it closes or no longer attaches the server.

AI is gated by `Settings.ai.enabled`; switching it off tears down prompts,
permissions, tool registrations, processes and panel visibility across windows.
Chats are saved one file each in a private state directory and reopened through
ACP session resume/load; fork uses `session/fork`. All three are sent only when
advertised. Favorites have a separate state file. Usage limits come from Claude's
`usage_update` metadata and Codex's own session logs, read through an injected
directory. See
[AI agents](AI_AGENTS.md) for protocol, limits and privacy boundaries. On Linux,
agent processes can run isolated under bubblewrap: `nocterm_ai::sandbox` derives
the mount policy (read-only root, writable workspace and agent state, hidden
credential stores and nocterm's own directories, separate PID namespace), the
ACP adapter applies it and fails closed when bubblewrap is missing. A changed
isolation setting restarts running agents.

## Generated documentation

`cargo xtask docs` derives field descriptions/defaults from schemars schemas
and serialized defaults, action descriptions from parsed `actions!` declarations,
shortcuts from the keymap, and crate dependencies from Cargo metadata. Generated
files are deterministic and contain no timestamps. `cargo xtask docs --check`
compares them without modifying files. CI also runs format, build, lint, tests
and rustdoc. Behavioral architecture notes in this file remain authored text;
the dependency graph and references update from code.
