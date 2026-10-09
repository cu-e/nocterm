# Feature layer

Part of the [architecture overview](../ARCHITECTURE.md).

Features depend on shared contracts and never on other features or the SSH
adapter. Feature crates use the service, UI, domain and foundation
layers: `nocterm-terminal`, `nocterm-connections`, `nocterm-settings-ui`,
`nocterm-files`, `nocterm-snippets-ui`, `nocterm-monitor-ui`,
`nocterm-containers-ui`, `nocterm-vault-ui`, `nocterm-keymap-ui` and
`nocterm-agent`.

## Terminal

`nocterm-terminal` owns the terminal model/view and observes session
and settings changes. Its renderer decorates otherwise unstyled default-color
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
remote filesystem before starting a new epoch.

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

`nocterm-connections` owns persisted profiles and recents,
the grouped sidebar, profile editor and quick-connect menu. Its `ServerFacts`
global keeps what was detected about servers (system over SFTP, country through
GeoIP for public addresses) in the state directory's `servers.toml`, apart from
`connections.toml`. Flags are cached in memory and under `flags/`. Icons and flags
reach other features only as images in `ConnectionSummary`. The empty workspace
projects its six most recent saved profiles through
`ConnectionDirectory`, using current names and the same icon and country policy as
the sidebar. Quick destinations and deleted profiles are excluded. Selecting a
recent server defers opening through the normal authentication path until the
workspace borrow ends. Editor sections retain one draft, with a scrollable active
form, multiline description and fixed footer;
typed validation selects the section containing the invalid field. Folder membership
changes persist a complete profile before publishing new state; explicit folder
names keep empty groups available as drop targets. Group rename, ungroup and
delete are single queued mutations; removed profiles unlink their recents and
vault credentials are left untouched. `nocterm-settings-ui`
contributes a single settings Item with a page list. Every control saves itself:
switches and choices at once, text when typing pauses, on Enter and on blur.
Invalid text stays in its field with the error and is never written. Each save is
an edit of one section (`update_setting`) queued and applied to the settings
current when the write runs, so quick successive edits and other windows never
conflict; changes are published through SettingsStore only after a successful
write, observers of other sections are not woken, and fields not being edited
follow changes made elsewhere. Persistent writes run through bounded ordered
background queues; profile drafts carry the original snapshot, so another window
cannot silently lose its saved changes. Pages share `nocterm_ui::form` sections and rows
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

## Explorer

Explorer location editors own native input and asynchronous, bounded directory
completion. Suggestions use a viewport-constrained popup anchored below the input,
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
Invalid library files remain read-only; persistence errors appear in the panel and
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

## Monitor

`nocterm-monitor-ui` follows the active session (a
session with exec is remote; anything else is this computer when allowed),
keeps a sampler per recently seen host and runs exactly one script: the status
bar's metrics at the slow interval while the details are closed, and the union
with the details' metrics at the fast interval while they are open. It adds a
leading footer view through the workspace and supplies its Settings page as a
`SettingsPageSpec`.

## Containers

`nocterm-containers-ui` lists the active host's containers in a sidebar panel
whose switcher shows the running count, re-listing whenever the engine reports
a change. A container's log and a shell inside it are `ProgramSpec`s opened
through `Workspace::open_program`; the terminal runs them with
`ProgramTransport` over `HostExec::terminal`, so no feature opens a
connection of its own and the tab follows as the same host.

## Vault

The vault feature handles `workspace::UnlockVault` and `workspace::LockVault`
application-wide, so the palette and a terminal's sign-in prompt can unlock the
vault in a small dialog without opening Settings and without depending on the
vault feature.

## Agent

AI is gated by `Settings.ai.enabled`; switching it off tears down prompts,
permissions, tool registrations, processes and panel visibility across windows.
Chats are saved one file each in a private state directory and reopened through
ACP session resume/load; fork uses `session/fork`. All three are sent only when
advertised. Favorites have a separate state file. Usage limits come from Claude's
`usage_update` metadata and Codex's own session logs, read through an injected
directory. See
[AI agents](../AI_AGENTS.md) for protocol, limits and privacy boundaries. On Linux,
agent processes can run isolated under bubblewrap: `nocterm_ai::sandbox` derives
the mount policy (read-only root, writable workspace and agent state, hidden
credential stores and nocterm's own directories, separate PID namespace), the
ACP adapter applies it and fails closed when bubblewrap is missing. A changed
isolation setting restarts running agents.

A chat's input — the draft, the attachments and images for the next prompt
and the queue of prompts waiting to be sent — is its `Composer`, kept apart
from the conversation and the session ([ADR 0009](../adr/0009-thread-composer.md)).
