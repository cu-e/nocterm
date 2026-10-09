# 0001. Settings are typed sections owned by the crates that use them

## Context

`nocterm-settings` held one schema for every feature. A broken value in one
table made the whole file fail to load, and every feature depended on the
settings crate for types that were not its own.

## Decision

`nocterm-settings` provides only the mechanism: the `SettingsSection` trait, a
document that reads each top-level table independently, and the file. Each
section lives in the crate that uses it (`[ssh]` in session, `[appearance]` in
ui, `[ai]` in ai, `[vault]` in vault-ui, …), is registered at startup, and is
read and written with `cx.setting::<S>()` / `SettingsExt`. A broken section
falls back to its defaults, is reported, and is left untouched on save.

## Alternatives rejected

- One schema struct with `#[serde(default)]` per field: still couples every
  feature to one crate and loses the broken table on save.
- Per-feature files: splits what users edit as one file.

## Consequences

- Settings pages are generic over sections rather than over one struct.
- `nocterm-session` depends on `nocterm-settings` only to implement the trait
  for its sections (the orphan rule forbids implementing it elsewhere).
- `Appearance` was renamed `AppearanceSettings` to name the section.
- `cargo xtask architecture` fails if a crate defines a section the generated
  settings reference does not list.
