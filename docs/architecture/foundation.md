# Foundation layer

Part of the [architecture overview](../ARCHITECTURE.md).

Foundation crates depend only on each other and on no GUI:
`nocterm-core`, `nocterm-settings` and `nocterm-design`.

## Paths, persistence and settings

`nocterm-core` provides paths and atomic, comment-preserving TOML persistence.
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
