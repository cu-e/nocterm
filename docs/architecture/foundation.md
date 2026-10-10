# Foundation layer

Part of the [architecture overview](../ARCHITECTURE.md).

Foundation crates depend only on each other and on no GUI:
`nocterm-core`, `nocterm-settings` and `nocterm-design`.

## Paths, persistence and settings

`nocterm-core` provides paths, private file creation, staged atomic publication and
comment-preserving TOML persistence. `fs::create_private` refuses existing paths,
including symlinks; `fs::write_private_atomic` replaces the final path using a
private synced staging file. Owners with revision or cancellation checks stage an
`AtomicPublication`, validate while retaining their lock, then publish. SSH trust
replacement preserves explicitly supplied original permissions. Parent directories
must be trusted; these helpers do not pin ancestors against mutation. Publication
commits at rename; a later directory-sync failure is best effort and cannot report
an already-published snapshot as rejected. TOML saves intentionally resolve existing
configuration symlinks, while private data and backups replace their final symlink.
See [private file publication](../adr/0015-private-file-publication.md).
`nocterm-settings` owns the settings mechanism, not their schema: the
`SettingsSection` trait, a document that reads each top-level table on its own
(a broken table falls back to its defaults and is reported, the others keep
working, and saving leaves it untouched) and the file. Each section lives with
the crate that uses it: `[ssh]`, `[local]` and `[logging]` in `nocterm-session`,
`[appearance]` and `[terminal]` in `nocterm-ui`, `[ai]` in `nocterm-ai`,
`[monitor]` in `nocterm-monitor-ui`, `[explorer]` in `nocterm-files` and
`[vault]` in `nocterm-vault-ui`. Owners register their sections at startup and
read them with `cx.setting::<S>()`; `cargo xtask architecture` fails if the
settings reference does not list a section.

## Design tokens

`nocterm-design` owns typed visual tokens and theme overrides. These foundation
crates have no UI or transport dependencies.
