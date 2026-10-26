# 0092. The plugin SDK is a guest-only crate, and a developer session keeps its plugin and settings in memory

Status: accepted

## Context

[ADR 0091](0091-user-plugin-code-runs-in-process-on-wasmi.md) fixed the ABI between the host and
a code plugin. An author still needs a way to write the guest side without hand-encoding
postcard, to test it without an application, and to try it in the application without installing
it. Three decisions follow, and each crosses a rule that applies to the rest of the workspace:

- The workspace sets `unsafe_code = "forbid"`, and the isolation policy bans `macro_rules!`,
  `include_bytes!`, `link_section` and `unsafe` in the plugin crates. A guest module needs all of
  them: the exports of the ABI, the manifest section and a slot for the plugin value.
- A run mode that composes the host from different ports owes a guarantee about what it writes.
  "Nothing is saved" is too broad to promise, because the application around the host keeps
  its own state.
- The settings written by the layout editor depend on which plugins the running session knows.

## Decision

**The SDK.** `nocterm-plugin-sdk` (domain layer, `no_std` with `alloc`) is the guest side of the
ABI: the `Plugin` trait, `Cx`, `ViewBuilder`, observation markers, effect constructors and the
`export!` macro. Its source is safe code. The only `unsafe` in the guest is the expansion of
`export!` in the author's crate, for `wasm32` only: a `Sync` impl for a cell that holds the plugin
and the call buffers, the pointer check in `nocterm_call`, and `link_section` for the manifest.
The `Sync` impl is sound for a single-threaded module, which every module the host accepts is
(shared memory needs an import, and the host refuses all imports); the macro states that premise
with a `compile_error!` under the `atomics` target feature. A panic aborts the guest, which the
host sees as a trap.

**Guest-only.** The SDK has no isolation row, because the structural bans would reject `export!`.
`xtask/architecture-policy.toml` lists it under `guest_only` instead, and
`cargo xtask architecture` requires that the closure of `nocterm`, with and without
`user-plugins`, does not reach it (development dependencies do not count). The rule fails when
the application or a listed crate is not a workspace package, so a rename cannot turn it into a
check that passes on nothing. A narrower row that keeps the closed dependency list for the SDK
itself is possible later.

**The test host.** `testing::Host` (feature `testing`) drives the same dispatch as the module,
with the real encoding, and applies the host's rules to the reply: it validates the view, refuses
a set of more than eight effects and a reply with more log lines than allowed, answers
`NotGranted` and `Denied` as the host does, and refuses to deny a required capability, which the
host never withholds. A test that passes there fails the same way in the application.

**The developer session.** `nocterm plugin dev <file>` starts the application with
`UserPluginMode::Dev(file)` in place of `Installed`; the mode is one field of
`bootstrap::Services`, and a further mode extends the enum, not the struct. The command refuses a path that is a link, not a
regular file or not valid UTF-8 before a window opens, with a typed error. The session:

- composes the host from `SessionRecords`, `SessionData` and `SessionStore`, which hold plugin
  records, data and packages in memory and register nothing for shutdown, so the host has no path
  to a file; installed plugins are not loaded, and the file is offered through the same consent
  as any package;
- reads `settings.toml` but keeps every change in memory. `PreparedApp::load` chooses
  `SettingsConfig::in_memory` from `UserPluginMode::saves_settings`. The reason is the layout
  editor: it settles the selected preset against the plugins the session publishes, which lack
  the installed ones, so the first edit would move their placed entries to the tray and write the
  presets back. Enablement of system plugins, the theme choice and every other setting are
  covered by the same choice;
- saves everything else as usual, including connections, snippets, the vault, saved workspaces,
  installed themes, the keymap, layout sizes, terminal recordings and agent state and chats. The session is not a sandbox for the
  application, and no text may say it is.

**DEV markers.** Everything of the dev plugin is labelled as the developer's: the consent dialog,
the settings banner and a notice in every window say the plugin and the settings are not saved; a
panel header ends in "· DEV"; a bar item carries a DEV chip and its tooltip ends in "· DEV". Menu
and palette entries use the ordinary plugin label.

## Alternatives rejected

- **A `forbid(unsafe_code)` SDK with the glue generated elsewhere** (a build step or a proc macro
  in a separate crate): the author would depend on two crates and a build script for four
  functions, and the glue would still be `unsafe`.
- **A row for the SDK in the isolation policy:** I4 would have to allow what it exists to ban.
- **A read-only layout editor in a session:** it keeps the presets but leaves other settings
  (plugin enablement, themes) free to be written, and the user cannot arrange the bar they are
  testing in.
- **`persist = false` for the whole session:** it would also stop connections, snippets and
  recordings, which a developer expects to work, and change the shutdown path of the session.

## Consequences

- The text a user reads about a session must name what is kept in memory (the plugin and the
  settings), never "nothing". Changing the set of in-memory state changes
  `UserPluginMode::saves_settings` and these texts together.
- Layout sizes of a developer panel are saved under its contribution id, like those of any panel.
  They are harmless and are not cleaned up.
- A guest author's crate must not `forbid(unsafe_code)`, and builds for `wasm32-unknown-unknown`
  without the `atomics` feature.
- The SDK is a workspace crate used by path or git dependency; publishing it, and a version policy
  for the ABI it targets, are release decisions.
