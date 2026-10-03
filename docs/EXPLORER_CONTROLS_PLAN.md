# Explorer and workspace controls

This follow-up implements the user's requested controls. The user authorized
implementation, commits and local merges to `master` without further confirmation.
The prior workspace implementation is preserved in `c73b1af`; this follow-up lives
on `feat/explorer-workspace-controls`. No push or PR is requested. Images and
screenshots are excluded from verification at the user's request.

## Boundaries and behavior

- Connections: typed profile drags change folder membership, with Ungrouped,
  collapsed folders and retained empty folders as destinations. Persist before
  publishing state; keep every other profile field unchanged.
- Workspace: native tab context menus use the clicked Item and existing dock
  tree for close/split commands. Pane-local close commands preserve other panes
  and the bottom local terminal. Move section controls into a full-width footer;
  remove the empty bottom Dock when its last terminal closes.
- Settings: use native page tabs and keep one Settings Item. Inject Vault through
  shared SettingsPage descriptors from the composition root. Section changes
  preserve unsaved settings and clear transient secret fields when leaving Vault.
- Explorer: compact token-based rows and native context menus for name/path
  copying, rename, confirmed deletion and file properties. Snapshot paths and
  RemoteFs when opening the menu; run I/O off the UI thread. Refresh only the
  matching navigation generation and filesystem. Capabilities disable unsupported
  operations. Keep special permission bits and refuse permission edits on links.
- Session/SSH: additive filesystem metadata and mutation contracts, supported by
  the existing bounded SFTP adapter. Rename never deliberately overwrites another
  entry. Recursive deletion is cancellable between operations and permanently
  deletes completed entries; no rollback is implied. Ordinary symlink targets are
  preserved. Remote path replacement races remain an SFTP v3 limitation.
- Transfers: retain existing remote-to-local drag/drop and bounded streaming
  workers. Progress/history ordering follows batch creation time.

## Verification and review

The architect, implementer, tester and reviewer stages use the repository's agent
roles. Tests use native GPUI pointer/keyboard events and element identities, never
images. Cover right-click selection, focus, collisions, permission bits, partial
deletion cancellation, stale navigation/server replies, empty-folder drops, tab
close scope, bottom-pane removal and the Settings/Vault singleton. Exercise new
SFTP operations against private local OpenSSH servers, including permission denial
and symlink behavior. Run the full format/build/clippy/tests/documentation gates
before independent review and merging. Linux checks do not verify other platforms.
