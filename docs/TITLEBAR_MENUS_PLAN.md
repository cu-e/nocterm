# Title-bar menus and terminal search

The requested change adds Session, Edit, Search, Window and Help to the existing
GPUI Kit title bar. It keeps the sidebar toggle, tab controls and standard toolkit
appearance. The user authorized implementation, verification, review, local
commits and integration into `master` without further confirmation.

## Design and boundaries

- The composition root supplies menu descriptors and application-wide window,
  directory and About actions. About reads the release system's `version.txt`.
  A new window reuses initialized transport, credentials, settings and transfers.
- Workspace owns the native menu bar and routes feature commands through the
  type-erased `ItemCommand` contract. A focused bottom terminal takes precedence
  over the central tab. Each window takes a fresh command-state snapshot before
  opening its menu and preserves that context while its popup is open.
- Native input actions handle text fields, including modal fields. Terminal
  commands only handle the terminal screen. Unsupported operations are disabled.
  Window commands reuse the existing dock and keymap actions.
- Terminal owns search controls and lifecycle operations. VT searches rendered
  characters in the screen and retained history, respecting hard line breaks,
  soft wrapping, wide characters and combining characters. Literal search is
  incremental and bounded; it does not copy the full history or collect all
  matches. Navigation wraps and highlighting stays separate from selection.
- Session Settings edits a validated transient copy of the current session's
  options. Apply affects the next reconnect; profiles and global defaults keep
  their own editors. Live reconnect first retires the old pump and session.

## Implementation sequence

1. Add the shared commands and window-local native menu integration.
2. Implement bounded terminal search, search controls and session lifecycle
   routing; reuse the existing session options editor in a standard dialog.
3. Wire the five menus, shared window services, About and key bindings.
4. Generate action/shortcut references and document the architecture.
5. Run build, formatting, lint, tests, documentation freshness and architectural
   checks. Independently review the final change, fix blocking findings and
   fast-forward the reviewed feature branch into `master`.

## Risks and verification

Opening a menu must preserve the originating terminal or input. Updating menu
descriptors must preserve native trigger entities and keyboard focus; the small
maintained toolkit patch is documented in `vendor/gpui-component/NOCTERM.md`.
Native event tests cover mouse and keyboard opening, disabled actions, multiple
windows, output changes while a popup is open and dispatch to the bottom terminal.

Search must yield during long or continuously wrapped output. Tests cover Unicode,
overlap, wraparound, history, bounded scan steps and generation invalidation.
Output, resize and alternate-screen changes invalidate stale matches. During
rapidly changing output, a consistent complete result can remain pending until
the buffer is stable; input and Escape remain responsive. No stale highlight is
presented as a current result.

Application integration tests cover actual menu actions, settings focus,
connection-name copying, About and independent window lifecycle. Real isolated
OpenSSH/SFTP fixtures remain mandatory in the final Linux gate. Screenshots and
image inspection are excluded at the user's request.
