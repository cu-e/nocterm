# Architecture

The application is the composition root. `src/main.rs` loads paths, settings and
design tokens, installs SSH/local transports and the vault service, and registers independent workspace
features. The [dependency map](architecture/dependencies.md) is generated from
Cargo metadata; `cargo xtask architecture` enforces its declared layers.

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
actions operate the same dock for keyboard split/reorder/focus. Bottom local
terminal focus preserves the last central remote context. An `Item` supplies tab content, focus and an
optional `SessionContext`; a `Panel` supplies sidebar content. Features register
actions rather than making the shell depend on them. `SessionSpec` and a session
opener connect requests from the connections feature to the terminal feature.
Cached session contexts avoid reading an Item reentrantly during its own render;
matching connected contexts also support transfer retry while a utility tab is active.

Features depend on shared contracts and never on other features or the SSH
adapter. `nocterm-terminal` owns the terminal model/view and observes session
and settings changes. `nocterm-connections` owns persisted profiles and recents,
the grouped sidebar, profile editor and quick-connect menu. `nocterm-settings-ui`
contributes a single settings Item; it validates a draft, saves it, and publishes
changes through SettingsStore only after a successful write. `nocterm-files`
contributes an Explorer Panel which follows `ActiveSessionChanged`. The remote
half uses home/listing requests; the local half owns navigation, selection and a
cancellable background logical-size scan. Request tasks are dropped when
superseded, and generations reject stale replies. The local cwd bridge goes
through Workspace's `LocalTerminal` contract, without Files depending on Terminal.

Typed session options inherit global TERM, charset, proxy and output-recording
defaults, with explicit profile/quick-connect overrides. A shared options form in
`nocterm-ui` keeps settings and connection features independent. Terminal owns
incremental decoding/encoding at the text boundary; terminal replies and mouse
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
