# Workspace expansion plan

Status: approved by the user with “реализуй”; implementation and verification
are underway. The tab inset and terminal Tab regressions are fixed. This plan
covers the user's October 3 request.

## Product behavior

Keep the standard GPUI Kit appearance. Use its dock, inputs, resizable panels,
icons and theme; put new product dimensions in design tokens. Drag previews
should show the actual destination. Mouse and keyboard commands must operate on
the same model. Do not introduce cloud accounts or unrelated IDE features.

### Tabs and panes

Adapt existing workspace Items to `gpui_kit::component::dock::Panel` and use the
toolkit's DockArea/DockSkin and pane tree for reorder, split previews and resize.
Keep one authoritative layout; do not build a second tree alongside the dock.
Each pane has its own top tab strip. In a single pane this remains the top strip;
TitleBar owns window chrome, sidebar control and the new-connection entry point.

A dragged tab moves its existing Item and SSH session. Closing the last tab in
a pane collapses that pane. Active remote context follows the focused central
pane. Command handlers for move, split, focus and close also support keyboard
bindings, independently of pointer hit testing.

Double-click a tab title to edit an alias inline. Enter accepts, Escape cancels,
and an empty alias restores the original title. Store aliases as presentation
metadata separate from saved connection names and terminal OSC titles. This
request does not introduce workspace restoration; aliases live with open tabs.

### Local terminal and launch settings

One local shell belongs to the workspace's bottom panel. A bottom button toggles
visibility; hiding the panel keeps the process alive. The panel has a vertical
resize handle and an explicit close action to end its process. Switching SSH
tabs does not restart it.

Implement a local session adapter using portable-pty and reuse Terminal/VT.
Configure shell executable, argument array, initial directory and environment.
Default to the operating system's shell and home directory. Add SSH launch
defaults plus per-profile overrides for remote shell/program, arguments, initial
directory and environment. Existing profiles keep the standard SSH shell request.
Launch changes take effect on the next start or reconnect; appearance changes
continue to update existing terminals.

### Explorer and local directory synchronization

Rename Remote Files to Explorer. Remote occupies the upper half and Local the
lower half, separated by a draggable horizontal divider with usable minimum
heights. Local navigation survives remote tab changes. Focusing the bottom shell
preserves the last selected central SSH context for the remote browser.

The local footer shows immediate file/folder counts and the logical byte total
of regular files recursively. Compute the total in the background, show that it
is still being calculated or incomplete, and report permission errors. Do not
follow symlinks or read special files. Cancel scans on navigation and reject
stale replies. Logical bytes are explicitly different from allocated disk space.

Two distinct local-only actions synchronize directories:

- Explorer → Local terminal changes the bottom shell's current directory.
- Local terminal → Explorer navigates the local browser to the shell's directory.

Use shell integration (OSC 7 for cwd, prompt-state markers where supported), not
prompt-text parsing. Provide Bash/Zsh/fish/PowerShell integration without editing
the user's startup files. Escape directory arguments for the actual shell. Send
`cd` only when the shell is known to be at a prompt; otherwise show the prepared
command for explicit use. Unsupported shells must show a clear limitation.
Neither action synchronizes a remote path into the local browser.

### File uploads

Introduce a transfer service separate from Explorer. Support dragging selected
local files/folders and external file drops onto a remote directory. Snapshot the
session and destination at drop time; changing tabs must not reroute an upload.

Use streaming reads, bounded discovery/queues, limited concurrent files and
coalesced progress updates. Do not create an unbounded task or whole-file buffer
for each file. Show batch progress, current work, completion/errors, Cancel and
Retry even if Explorer is hidden. Symlinks and special files are skipped visibly.

Resolve name collisions per batch with Skip/Rename/Replace; Skip is the safe
default, and existing files are never silently overwritten. Upload through owned
temporary remote files, then publish after closing the writer. Negotiate server
rename capabilities and state any atomic-replacement limitation. Cancel/error
cleanup removes only this job's partial files. Disconnection fails the affected
jobs visibly; retry is explicit and does not silently switch sessions.

### Credential vault

Use an encrypted portable vault unlocked by a master password. The operating
system's keychain may become an optional unlock provider, but cannot be required.
Profiles and recent entries carry stable credential IDs, never plaintext secrets.
Secret contents and metadata live inside the encrypted payload. Save passwords
and private-key passphrases after successful authentication and explicit choice;
do not automatically retain interactive/MFA responses.

Use maintained RustCrypto implementations of Argon2id and XChaCha20-Poly1305.
A random data-encryption key encrypts the payload; a master-password-derived key
wraps it. Use OS randomness, fresh nonces, authenticated format metadata and a
versioned envelope. Validate format length and KDF resource limits before memory
allocation. Start with RFC 9106's second recommended Argon2id configuration:
64 MiB, three iterations, four lanes, a 16-byte salt and a 32-byte derived key.

Provide create/unlock/lock, automatic locking and master-password changes.
Use secrecy/zeroize for sensitive buffers, redacted Debug and no secret logging.
Write atomically with private permissions and concurrent-writer protection;
failed writes preserve the previous vault. Explain at creation that forgetting
the master password prevents recovery. Future storage providers receive only
ciphertext; no cloud backend or account system is implemented now.

The cryptographic libraries' audits are evidence about the primitives, not an
audit of the complete vault. Require a separate security review of its format,
locking, persistence and authentication integration.

### Terminal line metadata

Add optional timestamps and sequential logical-line numbers, disabled by default.
Wrapping does not allocate another number; timestamps record first output of the
line. Keep metadata coherent through history, trimming, clear, redraw and reflow.
Hide the gutter in alternate-screen programs. Gutter text is outside the terminal
grid, clipboard selection and PTY stream. Implement metadata at the emulator
boundary, not by interpreting network chunks as complete lines.

## Implementation order and boundaries

1. Workspace dock adapter, tab aliases and shared layout commands:
   `crates/workspace/src/{workspace,item,actions}.rs` and new dock adapter modules.
2. Session launch contracts and local PTY adapter: `crates/session`, new
   `crates/local`, `crates/terminal`, shell integration assets and `src/main.rs`.
3. Local/remote Explorer and shell navigation bridge: split `crates/files` into
   browser, statistics and drag modules. Files must not depend on Terminal.
4. Transfer domain and streaming SFTP primitives: new `crates/transfers`,
   `crates/session/src/fs.rs`, `crates/ssh/src/sftp.rs` and Explorer queue UI.
5. Vault and credential UI: new `crates/vault`, sensitive session types,
   connections storage/editor and authentication prompts.
6. VT line metadata and gutter: `crates/vt` and terminal rendering/input geometry.
7. Complete settings forms, per-profile controls, shared styles/icons, actions,
   generated references, architecture rules, development docs and CI.

Each completed stage goes through implementer → tester → reviewer. Keep the
current feature branch; do not commit, push or open PRs without a separate request.

## Verification

- GPUI tests: inline aliases, drag versus double-click, reorder and four-way
  splits, active session/focus, pane collapse, resize, settings singleton and
  exactly one close/reconnect. Native pointer-based smoke checks are also needed.
- PTY tests: Tab/input/resize, startup options/cwd, exit and process reaping,
  hide/show without restart, shell integration across fragmented sequences,
  quoted paths and unsupported shells.
- Explorer/transfers: stale replies, permissions, large directories, symlink
  cycles, external drops, many small files, bounded large-file memory, collisions,
  disconnect, cancellation and partial cleanup against real local OpenSSH/SFTP.
- Vault: wrong passwords, tampering, truncation, hostile KDF headers, random
  key/nonce freshness, lock/update races, failed atomic writes, concurrent writes,
  permissions, rotation and absence of plaintext in config/state files.
- VT: wraps/reflow, CR/redraw, insertion/deletion, scrollback trimming/clear,
  alternate screen and selection/clipboard geometry with gutter enabled.
- Full fmt/build/clippy/tests/docs check/architecture/rustdoc. Report platform
  coverage honestly; Linux execution does not verify Windows or macOS.

Highest risks are the vault envelope/lifecycle, streaming SFTP cancellation,
shell integration with arbitrary foreground programs and logical-line metadata
inside a mutable terminal grid. Review these explicitly instead of treating
compilation as behavior verification.

## Primary references used in design

- [RFC 9106: Argon2 recommendations](https://www.rfc-editor.org/rfc/rfc9106.html#section-4)
- [RustCrypto ChaCha20-Poly1305 documentation and security notes](https://docs.rs/chacha20poly1305/latest/chacha20poly1305/)
- [portable-pty](https://docs.rs/portable-pty/latest/portable_pty/)
- [WezTerm shell integration and OSC 7](https://wezterm.org/shell-integration.html)
- [age Rust implementation's pre-1.0 warning](https://docs.rs/age/latest/age/)

The installed GPUI Kit 0.7 sources were inspected for dock and focus APIs. Age was
considered as a portable existing encrypted format, but the current Rust crate's
pre-1.0 production-use warning makes it unsuitable for this plan's choice.
