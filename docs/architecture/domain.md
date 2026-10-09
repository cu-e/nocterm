# Domain layer

Part of the [architecture overview](../ARCHITECTURE.md).

Domain crates hold the rules and contracts of one subject and depend only
on other domain crates and the foundation, never on GPUI: `nocterm-session`,
`nocterm-vt`, `nocterm-themes`, `nocterm-ai`, `nocterm-monitor`,
`nocterm-containers`, `nocterm-snippets`, `nocterm-transfers` and
`nocterm-vault`. Adapters implement their contracts ([adapter layer](adapter.md));
features present them ([feature layer](feature.md)).

## Sessions and terminal emulation

`nocterm-session` defines transport-independent targets, authentication,
commands, events, prompts and the `RemoteFs` contract.
`nocterm-vt` turns a byte stream into a terminal grid using
alacritty_terminal and encodes keyboard, mouse and paste input. These domain
crates do not know the workspace or GPUI.

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
explicit error when a limit is exceeded. Regex batches run off the UI thread;
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
rename. `nocterm-ui` resolves light/dark selections from SettingsStore and the
catalogue into effective Design tokens, retaining the base tokens for Nocterm
Default. Imported component palettes start from empty configuration so the
component fallback chain derives unmapped colors. Settings UI injects its registry
client and runs explicit search/install/remove work on the background executor;
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

## Containers

`nocterm-containers` (domain) speaks the Docker and
Podman command lines over `HostExec` — detection (exit 127 or
`ExecError::NotFound` means not installed), listings parsed from either JSON
dialect, the event stream and actions — and knows nothing of GPUI.

## Transfers

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

## Vault

`nocterm-vault` owns a versioned authenticated encrypted envelope and a bounded
worker; `nocterm-vault-ui` exposes create/unlock/lock/rotation, auto-lock and a
credential provider contract. Profiles store opaque IDs. Authentication asks this
provider only for matching password/key-passphrase bindings; explicit Remember
commits after Connected and excludes MFA. Epoch checks invalidate queued results
on lock. See [security/lifecycle notes](../VAULT.md).
