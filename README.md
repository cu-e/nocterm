# nocterm

An extensible desktop SSH client written in Rust with GPUI Kit. The current
workspace provides draggable split panes, terminal tabs with aliases, saved
connections, quick connect, recent destinations, a local shell, an Explorer and
streaming SFTP uploads and downloads. Passwords can be saved in a portable
encrypted vault.

The title bar provides Session, Edit, Search, Window and Help menus. Session
offers connection lifecycle, output recording, settings, profiles and window
commands. Edit routes clipboard operations to the focused terminal or text field;
Copy Connection Name includes the tab's alias. Window exposes the existing split,
pane and tab controls. Help → About shows the packaged release version.

Search → Find searches the screen and retained history. It starts with literal,
case-sensitive text; the search bar can switch to regular expressions, ignore
case or match whole words. Literal searches can span hard line breaks; regular
expressions search one logical line at a time, joining soft wraps. A regex line
over 64 KiB reports an error instead of silently skipping results. Find
Next/Previous wrap; Find Next Selected Text uses the current selection. Search
highlights do not replace the clipboard selection. On Linux
and Windows use `Ctrl+Shift+F` to open search, `F3`/`Shift+F3` to navigate and
`Ctrl+F3` to search the selection. macOS uses `Cmd+F` and `Cmd+F3`. Enter navigates
and Escape closes the search field without sending those keys to the shell.
Search yields between scan slices and runs regex matching in bounded background
work. Rapidly changing output can leave results pending until a consistent scan
completes.

Session → Preferences → Session Settings applies per-session options to the next
reconnect without changing its saved profile. Default Session Settings opens the
SSH defaults. New Window shares application services while keeping its own tabs,
focus and menus; Close Window closes only that window.

## License

Nocterm is source-available under the [PolyForm Perimeter License 1.0.1](LICENSE).
Use and modifications, including contributions, are permitted subject to its
terms. Providing others with a competing product based on this code is not
permitted, even if that product is free of charge.

Third-party dependencies, vendored code and assets retain their own licenses.
The vendored forks and Nocterm's patches to them use their declared upstream
licenses. Distributed installers include [third-party notices](THIRD_PARTY_NOTICES.txt);
see [packaging documentation](docs/PACKAGING.md) for updating them.

## Install

Release pages provide a Windows installer, `.deb` and `.rpm` packages and an
AppImage. The Linux packages also install and start the fingerprint vault
broker. See [packaging](docs/PACKAGING.md).

## Build and run

Install the Rust toolchain specified by `rust-toolchain.toml`. On Debian or
Ubuntu, install the native GUI dependencies and OpenSSH for integration tests:

```sh
sudo apt-get install build-essential pkg-config clang cmake libssl-dev \
  libfontconfig1-dev libfreetype6-dev libxkbcommon-dev libxkbcommon-x11-dev \
  libwayland-dev libxcb1-dev libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev \
  libvulkan-dev mesa-vulkan-drivers openssh-server openssh-client
scripts/setup.sh
cargo build
cargo run
```

A running X11 or Wayland desktop and a Vulkan-capable driver are needed to open
the application. On macOS, install Xcode command line tools. Windows requires
the MSVC toolchain and its C++ build tools. CI verifies the workspace on Linux and
compiles/links vault device adapters on macOS and Windows.

Open a connection from the `+` menu. Save a profile with New Connection, or enter
`user@host[:port]` in Quick Connect. First connections ask you to verify the host
key. Passwords and key passphrases are requested when needed. Open Settings → Vault
(also reachable from Credential Vault in the `+` menu) to create or unlock a vault,
then explicitly choose Remember in
the authentication prompt; only successful authentication saves a credential.
Interactive/MFA answers are never saved. See [vault security](docs/VAULT.md).

Settings → Vault also detects optional fingerprint/Touch ID/Windows Hello unlock.
Linux uses a root-owned broker that the `.deb` and `.rpm` packages install;
detection and password fallback work without it. See [device unlock setup](docs/DEVICE_UNLOCK.md).

Drag a tab to reorder it or onto a pane edge to split horizontally/vertically;
the dock previews the destination. Double-click its title to set an alias. Enter
accepts, Escape cancels, and an empty alias restores the original title. Aliases
belong to open tabs. Right-click a tab to close it, close other/left/right tabs,
close all central tabs, split its pane or open Settings. Close commands apply to
the clicked pane and never close the bottom local shell. Pane and bottom-terminal
dividers can be resized. The section, local terminal and AI panel buttons occupy a
full-width bottom bar, including when the sidebar is hidden. Closing the last
bottom terminal removes its empty pane and resize handle.

Drag saved connections onto folder headers or Ungrouped to change their group;
collapsed and empty folders remain drop targets. Moving a connection keeps its
authentication, launch options, description and credential ID.
Double-click a folder name to rename it inline (Enter or leaving the field saves,
Escape cancels); renaming to an existing folder name merges the two. Hovering a
folder header shows expand/collapse and delete buttons. Deleting a folder asks
whether to delete its connections or ungroup them; saved vault credentials and
open sessions are kept.
Saved servers show their operating system's icon in its usual colour, detected
over SFTP on connect from `/etc/os-release` and a few platform markers
(Proxmox, TrueNAS, macOS, the BSDs, Windows and others). A server whose system is
unknown keeps the plain server icon. A flag after the name shows the server's
country. Its public IP address is located once through `api.country.is`; private
and reserved addresses and host names are never sent. Flags come from
`flagcdn.com` and are cached in memory and on disk. Settings › Appearance › Detect server countries turns the
lookup off. The editor lets you choose the icon and its colour, or set a
two-letter country code; empty fields mean automatic.
The connection editor groups fields into Connection, Authentication, Session and
Launch pages, laid out like Settings: titled sections of labelled rows on the
terminal's background. Switching pages keeps the draft; descriptions accept
multiple lines. Operation notices appear at the side of the window with recovery actions.

Explorer shows the active remote directory above a persistent local browser.
Its compact rows use the `typography.explorer_size` design token. Right-click a
file/folder to rename, copy its name/path, delete or open Properties. Properties
show type, size, owner IDs, UTC modification time and editable POSIX permission
checkboxes when the filesystem supports them; symbolic-link permissions are
read-only. Rename refuses an existing destination. Delete permanently removes
selected entries, including folder contents, after confirmation. Stop interrupts
between operations; completed deletions cannot be undone. Open dialogs retain
their original computer, directory and server after navigating or changing tabs.
Drag selected local files/folders, or files from the OS, onto a remote directory.
Skip is the default collision policy; Rename and explicit Replace are available.
Drag remote files/folders onto a local directory, or select them and use Download
to save them in the open local directory. Transfers use four concurrent streaming
workers and temporary files; uploads pipeline up to eight acknowledged 32-KiB
chunks. The bottom Transfers status opens progress, errors, cancellation and retry.
Retry retains its server and destination even after switching tabs or directories.
Replace requires the server's atomic POSIX rename extension. Network loss can
leave an owned `.nocterm-*.part` file when the server cannot be reached for cleanup.

Toggle the local terminal with its bottom button, `Ctrl+J` or `Ctrl+backtick`
(`Cmd+J`, `Cmd+backtick` on macOS). Hiding keeps it running; Close local terminal ends it.
The local Explorer footer shows immediate file/folder counts and recursive logical
bytes, excluding symlinks and special files. Its two arrow buttons synchronize
Explorer → local shell and local shell → Explorer. Changing the shell directory
requires supported shell integration and a known empty prompt. Unsupported shells
or an active command report the limitation. Remote paths are never sent to this
local shell.

Settings are opened through Session → Preferences → Settings or `Ctrl+,` (`Cmd+,` on macOS). Switch
between Appearance, Terminal, Local shell, SSH, AI agents, Keymap and Vault pages. Changes
save themselves: switches at once, text when you pause typing, press Enter or leave
the field; invalid text stays in its field with an explanation and is not saved. Appearance, terminal font, cursor,
scrollback and optional line
gutters update in open terminals. Line numbers identify logical lines through
wrapping/reflow; timestamps use UTC and the local host's first observation of
output. Both are hidden in alternate-screen programs and excluded from copying.
Local and SSH shell program, argument array, cwd and environment take effect on
the next start/reconnect. Saved profiles can override the SSH launch defaults.
Automatic local integration supports Bash, Zsh, fish and PowerShell; custom
argument arrays preserve the exact invocation and disable automatic integration.

Global session defaults and individual profiles offer TERM presets/custom values,
UTF-8 or Windows-1251/KOI8-R/Windows-1252/GBK, and Direct, HTTP CONNECT or SOCKS5
routes. SOCKS5 can resolve names at the proxy. Proxy authentication
is outside this subset. A saved connection can reach its host through ordered
jump hosts, each a saved connection or a `user@host[:port]` address; every
jump host is verified and signed in to in turn, and a configured proxy carries
only the first connection. Selecting TERM announces server capabilities; it does
not switch to a separate terminal emulator. Charset conversion is incremental
and affects terminal text, never SFTP bytes. Unrepresentable input reports an error.
Saved connections also have a description.

Output recording is opt-in: start/stop it from a terminal or enable automatic
recording globally/per profile. Unique private UTF-8 logs contain received output,
including escape sequences, and have a configurable size limit. Authentication
answers and input are excluded; remote output can still contain sensitive text.
Recording status/errors stay visible. See the
[terminal architecture](docs/architecture/feature.md#terminal) for recording
boundaries and persistence behavior.

## Configuration

Files use the operating system's configuration/state directories (see
[paths](crates/core/src/paths.rs)). `settings.toml` stores user choices;
`theme.toml` overrides the built-in [design tokens](crates/design/tokens/default.toml).
Unset design values retain the standard GPUI Kit component appearance.
Settings → Appearance lets you choose separate light and dark color themes.
Install, update and uninstall themes from Zed's extension registry there, or put
original Zed theme JSON files in `<config>/themes/` and press Reload. Managed
extensions live in `<state>/themes/<extension-id>/`. Search queries go to zed.dev;
network requests happen only when you search, browse or install. Selections apply
immediately to the interface and every open terminal. The command palette also
offers **Change Theme** for the current appearance and **Change Color Scheme**
for System, Light or Dark; searching “theme” finds both. Zed icon themes, syntax
colors and transparency are not imported. When an imported theme is selected,
`theme.toml` color overrides apply only to Nocterm Default; its typography, shape
and layout overrides always apply.
`connections.toml` stores profiles, `recent.toml` stores recent targets and
`known_hosts` stores accepted keys. In the state directory, `servers.toml` keeps
detected systems and countries, `flags/` the downloaded flags, `agents.toml`
model favorites and the last agent used, `agent-chats/` saved AI chats and
`layout.json` the widths of resizable columns and which side the sidebar is on.
`keymap.toml` in the configuration directory holds your key binding changes. `vault.bin` stores encrypted credentials and
uses a sibling `.lock` file for concurrent-writer protection; profiles contain
only opaque credential IDs. Existing OpenSSH known hosts are also read.

Every command is also available from the command palette: Ctrl+Shift+P
(Cmd+Shift+P on macOS) searches the commands available where focus is and runs
the chosen one.

The window has the sidebar (Servers, Explorer) on one side of the tabs and the
AI panel on the other; both resize by dragging their edge and keep their width.
`Ctrl+Shift+C` shows Servers and `Ctrl+Shift+E` the Explorer; pressing it again,
or clicking the shown panel's button in the footer, hides the sidebar.
`Ctrl+Alt+B` shows or hides the AI panel (`Cmd` instead of `Ctrl` on macOS). The
command palette's Swap Sides command puts the two on each other's side. In a
focused terminal `Ctrl+Shift+C` copies, and the terminal no longer receives
`Ctrl+J`; rebind it in Settings → Keymap if the shell needs it.

Settings → Keymap lists every command with its shortcuts and description. Search
matches words in any of them and shortcuts in any modifier order; Find by keys
shows what a pressed shortcut runs. Click a shortcut and press new keys to
rebind it, `+` adds another, `×` removes one and the reset button restores a
command's defaults. Changes apply at once and are kept in `keymap.toml` in the
configuration directory, which lists only what differs from the defaults
(`"none"` unbinds a default).

Reference documentation is generated from the Rust schemas, action declarations,
keymap and Cargo manifests:

- [Settings](docs/reference/settings.md)
- [Design tokens](docs/reference/design-tokens.md)
- [Actions and shortcuts](docs/reference/actions.md)
- [Architecture](docs/ARCHITECTURE.md)

## Development

```sh
cargo fmt --all --check
cargo build --workspace --all-targets
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
NOCTERM_REQUIRE_SSHD=1 cargo test -p nocterm-ssh --test sshd
cargo xtask docs
cargo xtask docs --check
cargo xtask architecture
cargo doc --workspace --no-deps
```

The SSH tests start private local OpenSSH servers with temporary keys. With
`NOCTERM_REQUIRE_SSHD=1`, missing `sshd` fails instead of skipping the tests.
Some OpenSSH installations need `/run/sshd` created by the system administrator.
The tests never connect to your saved hosts. Unit tests do not open a GUI;
manual verification on a desktop should cover focus, input, host-key prompts,
pane dragging/resizing, aliases, tab and Explorer context menus, selection/clipboard,
connection folder drops, reconnect, SFTP transfers,
local-shell directory synchronization, vault locking and settings. Linux tests
exercise Bash/Zsh/fish and OpenSSH; macOS, Windows and PowerShell execution need
platform-specific verification.

When a schema, action, keymap or internal Cargo dependency changes, regenerate
with `cargo xtask docs`. CI rejects stale generated files and dependency boundary
violations. Follow [CONTRIBUTING.md](CONTRIBUTING.md) for branches and commits.

On Linux/X11, quitting an executable built with `--all-targets` and GPUI's
test support enabled can report a leaked `TerminalView` handle. This was
observed during the GUI smoke check; dependency inspection points to deferred
X11 input-handler cleanup after the event loop stops. A normal `cargo build`
followed by launch and Quit exited successfully. Leak detection remains enabled
for tests; the native shutdown diagnostic still needs an upstream reproduction.

The terminal emulator is a maintained local patch of alacritty_terminal 0.26.0.
See [the patch notes](vendor/alacritty_terminal/NOCTERM.md) before updating it.

## AI agents

The right AI panel supports ACP agents Claude, Codex, Hermes and custom
executables, with terminal/connection/group context, capability-driven model
and effort controls, model favorites and image input. Open it from the footer
or Window → AI Agents (Ctrl+Alt+B / Cmd+Alt+B). AI Settings is available from
Session → Preferences and the normal Settings shortcut. Disabling AI closes
chats and stops its processes. Empty chats and saved history start no processes;
sending a message starts or restores the agent. Linux agents require a running
systemd user manager and `systemd-run` from systemd 254 or later. Each agent runs in a separate user service
with limits for memory, swap and tasks, and its children stop when Nocterm exits.

The built-in Claude/Codex commands use pinned npm ACP adapters and need Node.js
and npm (`npx`); Claude requires Node.js 22 or later. Hermes uses `hermes acp`.
Desktop PATH can differ from your shell; absolute executable overrides are
available in AI Settings. Model favorites persist in `agents.toml`.

Chats are saved in `agent-chats/` and reopen their agent session after a
restart when the agent supports resuming. The history can be searched, and each
chat can be renamed, forked or pinned. Chat actions also offer **Release agent
resources**, preserving the conversation and terminals. Across all windows, the
default policy permits four starting, live or closing sessions, retains two idle
sessions and releases them after 90 seconds. Configure `[ai.sessions]` and
`[ai.resources]` in settings.toml to change these limits. Stop cancels only the current reply; the
chat goes on in the same session. The ring by the send button opens a Context
Usage card with the context window, session tokens and, for Claude and Codex,
the 5-hour and weekly plan limits. The attach menu lists saved servers under
their folders.

Local agents are not sandboxed and may read your user files. Nocterm excludes
SSH/vault credential fields from context; terminal-output filtering cannot
recognize every secret. See [AI setup, tools and privacy](docs/AI_AGENTS.md).

The Workspaces button beside Containers opens saved folders for the current
connection and other servers. Each workspace has its own terminal tabs and chat
history, using ordinary local or SSH sessions and the server’s normal permissions.

With no server selected, the sidebar shows **This computer**. You can create a
workspace there without opening a terminal first. A selected disconnected server
must be reconnected before creating a workspace on it.

Workspace creation offers three modes on this computer or the selected server:

- **Direct connection** selects a required project directory on the connected
  computer or server. Available file access provides folder suggestions and
  Home, Up and Refresh controls.
- **Existing container** lists Docker and Podman containers available to the
  current user on that host, including stopped containers. Select the
  container and optionally choose a project directory.
- **New container** selects Docker or Podman and an image. Compatible Linux
  images can install Python, Rust, Bun, Node.js and Java together; Python
  is selected initially. Each language offers its actual default or a custom numeric
  version: Python minor/exact, Rust exact, Bun exact, Node.js major/exact, Java major.
  Python, Node.js and Java default to distribution packages; Rust uses Stable and
  Bun uses Latest stable. Requested versions install inside the new container,
  according to its detected distribution, architecture and libc. Unsupported
  combinations fail explicitly; for example, Alpine uses distribution Node.js
  rather than the glibc archives used for pinned Node.js versions.

The creation window follows **Connection → Project → Tools**. Connection chooses
the mode and resource; Project contains the name, directory and optional Git
repository. Tools appears only for a new container. Back keeps your entries;
preparation starts only after **Create workspace** on the final step.
The server stays in the shared header while you move between steps. Existing
containers can be searched by name, ID, image or engine. Custom image results use
separate local and registry sources with a full-width list.

**Settings → Workspaces** controls the default container project directory,
initially `/home`. Leaving the directory empty uses the default captured when
that creation window opened. An explicit directory takes precedence. Changing
this setting affects future creation windows and preserves saved workspaces.

Workspace settings show the actual environment and installed versions when its
connection is available. **Install / update** changes only selected languages;
unchecked languages remain installed. Saving the workspace name or directory
does not install tools. Managed versions use the current user's workspace tool
directory and become available in new workspace terminals. Existing terminals
keep their current environment. Native package installation needs existing root
permissions; Nocterm does not invoke sudo or edit your shell startup files.
Cancellation can leave completed package or language changes in place.
An existing Arch environment needs an initialized, current package database;
Nocterm asks an administrator to complete `pacman -Syu` when it is missing.
Initial package synchronization is performed inside a newly created Arch container.

Creation shows its current phase beside the action buttons. Download byte counts
are shown when the engine reports them; progress remains indeterminate when a
reliable total is unavailable.

Custom image entry immediately filters local repositories and tags. After a brief
pause it also searches Docker Hub with Docker, or configured registries with Podman,
using that host's existing engine configuration. Search lists repositories, not tags;
enter a full reference such as `debian:13` for a particular tag. Search never pulls
an image. Local images and manual references remain available when registry search
fails or is rate limited.

A Git repository is optional. Cloning uses an empty destination and does not
overwrite existing project files. An empty container directory uses the configured
default described above.
Closing a workspace or removing its saved entry leaves containers and project
files intact. Container workspace paths are entered manually. Once connected,
Explorer browses and transfers files inside the selected container, using its
configured user and normal permissions. This needs Linux `/proc` and compatible
standard shell/file utilities, available in ordinary Ubuntu, Debian and Alpine
images; minimal images without them report unavailable file access. No SSH server
or helper is installed in the container. Folder rename and cross-filesystem rename
are unsupported by this adapter; ordinary file rename never overwrites an existing
entry. Server files remain available through the server's own connection.

The footer monitor follows the selected workspace's active environment. Folder
workspaces use their own server connection; container workspaces use their own
Linux cgroup CPU and memory accounting. Container CPU is shown relative to one
CPU, so it can exceed 100%. Memory without an observable limit shows bytes in use
without a percentage. Host-only metrics stay unavailable in containers. An
unavailable workspace never redirects monitoring to this computer.
