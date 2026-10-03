# Architecture

The application is the composition root. `src/main.rs` loads paths, settings and
design tokens, installs SSH/local transports and the vault service, and registers independent workspace
features. The [dependency map](architecture/dependencies.md) is generated from
Cargo metadata; `cargo xtask architecture` enforces its declared layers and
rejects runtime GUI dependencies below the UI layer. The
[architecture and security audit](ARCHITECTURE_SECURITY_AUDIT.md) records findings,
regression evidence and residual constraints.

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
provides shared icons and terminal styling. `nocterm-workspace` owns window
layout, tabs and sidebar switches through a native DockArea/DockSkin. The
toolkit owns the single pane tree and drag previews; an adapter exposes Items as
dock Panels. Presentation aliases are separate from Item/session titles. Shared
actions operate the same dock for keyboard split/reorder/focus. Tab context-menu
closures snapshot the clicked Item and resolve its current pane order at execution;
closing adjacent/other tabs cannot cross pane boundaries. The full-width workspace
footer owns section switches and status views. Removing the last bottom Item also
removes its Dock instead of merely emptying the tab list. Bottom local
terminal focus preserves the last central remote context. An `Item` supplies tab content, focus and an
optional `SessionContext`; a `Panel` supplies sidebar content. Features register
actions rather than making the shell depend on them. `SessionSpec` and a session
opener connect requests from the connections feature to the terminal feature.
Cached session contexts avoid reading an Item reentrantly during its own render;
matching connected contexts also support transfer retry while a utility tab is active.

Features depend on shared contracts and never on other features or the SSH
adapter. `nocterm-terminal` owns the terminal model/view and observes session
and settings changes. `nocterm-connections` owns persisted profiles and recents,
the grouped sidebar, profile editor and quick-connect menu. Folder membership
changes persist a complete profile before publishing new state; explicit folder
names keep empty groups available as drop targets. `nocterm-settings-ui`
contributes a single settings Item with native page tabs; it validates a draft,
saves it, and publishes changes through SettingsStore only after a successful
write. Persistent writes run through bounded ordered background queues;
settings drafts carry a revision and profile drafts carry the original snapshot,
so another window cannot silently lose its saved changes. Recent snapshots
coalesce separately from connection opening. Native quit draining is bounded
best effort within GPUI's shutdown deadline. Workspace's `SettingsPage`/`SettingsPageSpec` contract lets the composition
root inject the lazy Vault page without feature-to-feature dependencies. Page
deactivation/closure clears transient secrets. `nocterm-files`
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
number of matches or history size. Soft wraps preserve text continuity; hard
line breaks prevent cross-line matches. Terminal schedules scan slices and
invalidates matches when output, geometry or screen changes. A rapidly changing
buffer can defer a complete result, but stale highlights are cleared and input
remains responsive. Search highlights and clipboard selection are independent.

Session Settings reuses the validated session options editor for transient
overrides. Safe reconnect retires the previous session, event pump, prompts and
remote filesystem before starting a new epoch. The application window factory
reuses global services rather than initializing another vault or transfer queue.

Add a new Item for another kind of tab, a Panel for another sidebar section or a
registered action for a command. Observe the terminal model or session contract
for integrations. Add another Transport implementation to support another kind
of session. Theme changes belong in the token source or its user override.
IDE features, AI, collaboration, Vim mode and an extension runtime are future
consumers of these boundaries; no implementations are included now.

## Generated documentation

`cargo xtask docs` derives field descriptions/defaults from schemars schemas
and serialized defaults, action descriptions from parsed `actions!` declarations,
shortcuts from the keymap, and crate dependencies from Cargo metadata. Generated
files are deterministic and contain no timestamps. `cargo xtask docs --check`
compares them without modifying files. CI also runs format, build, lint, tests
and rustdoc. Behavioral architecture notes in this file remain authored text;
the dependency graph and references update from code.
