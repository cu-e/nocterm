# nocterm

An extensible desktop SSH client written in Rust with GPUI Kit. The current
workspace provides draggable split panes, terminal tabs with aliases, saved
connections, quick connect, recent destinations, a local shell, an Explorer and
streaming SFTP uploads and downloads. Passwords can be saved in a portable
encrypted vault.

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
the MSVC toolchain and its C++ build tools. CI currently verifies Linux.

Open a connection from the `+` menu. Save a profile with New Connection, or enter
`user@host[:port]` in Quick Connect. First connections ask you to verify the host
key. Passwords and key passphrases are requested when needed. Open Settings → Vault
(also reachable from Credential Vault in the `+` menu) to create or unlock a vault,
then explicitly choose Remember in
the authentication prompt; only successful authentication saves a credential.
Interactive/MFA answers are never saved. See [vault security](docs/VAULT.md).

Drag a tab to reorder it or onto a pane edge to split horizontally/vertically;
the dock previews the destination. Double-click its title to set an alias. Enter
accepts, Escape cancels, and an empty alias restores the original title. Aliases
belong to open tabs. Right-click a tab to close it, close other/left/right tabs,
close all central tabs, split its pane or open Settings. Close commands apply to
the clicked pane and never close the bottom local shell. Pane and bottom-terminal
dividers can be resized. The section, terminal and settings buttons occupy a
full-width bottom bar, including when the sidebar is hidden. Closing the last
bottom terminal removes its empty pane and resize handle.

Drag saved connections onto folder headers or Ungrouped to change their group;
collapsed and empty folders remain drop targets. Moving a connection keeps its
authentication, launch options, description and credential ID.

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

Toggle the local terminal with its bottom button or `Ctrl+backtick`
(`Cmd+backtick` on macOS). Hiding keeps it running; Close local terminal ends it.
The local Explorer footer shows immediate file/folder counts and recursive logical
bytes, excluding symlinks and special files. Its two arrow buttons synchronize
Explorer → local shell and local shell → Explorer. Changing the shell directory
requires supported shell integration and a known empty prompt. Unsupported shells
or an active command report the limitation. Remote paths are never sent to this
local shell.

Settings are opened with the gear button or `Ctrl+,` (`Cmd+,` on macOS). Switch
between Appearance, Terminal, Local shell, SSH and Vault tabs. Edit the form and
Apply; switching sections preserves the draft. Appearance, terminal font, cursor,
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
routes. SOCKS5 can resolve names at the proxy. Proxy authentication and jump hosts
are outside this subset. Selecting TERM announces server capabilities; it does
not switch to a separate terminal emulator. Charset conversion is incremental
and affects terminal text, never SFTP bytes. Unrepresentable input reports an error.
Saved connections also have a description.

Output recording is opt-in: start/stop it from a terminal or enable automatic
recording globally/per profile. Unique private UTF-8 logs contain received output,
including escape sequences, and have a configurable size limit. Authentication
answers and input are excluded; remote output can still contain sensitive text.
Recording status/errors stay visible. See the [WindTerm study](docs/WINDTERM_STUDY.md)
for selected behaviors and source references.

## Configuration

Files use the operating system's configuration/state directories (see
[paths](crates/core/src/paths.rs)). `settings.toml` stores user choices;
`theme.toml` overrides the built-in [design tokens](crates/design/tokens/default.toml).
Unset design values retain the standard GPUI Kit component appearance.
`connections.toml` stores profiles, `recent.toml` stores recent targets and
`known_hosts` stores accepted keys. `vault.bin` stores encrypted credentials and
uses a sibling `.lock` file for concurrent-writer protection; profiles contain
only opaque credential IDs. Existing OpenSSH known hosts are also read.

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

## License

Nocterm is source-available under the [PolyForm Perimeter License 1.0.1](LICENSE).
Use and modifications, including contributions, are permitted subject to its
terms. Providing others with a competing product based on this code is not
permitted, even if that product is free of charge.

Third-party dependencies, vendored code and assets retain their own licenses.
