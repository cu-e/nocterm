# App and tooling

Part of the [architecture overview](../ARCHITECTURE.md).

The app layer is the composition root and may depend on everything but
tooling: `nocterm` and the optional `nocterm-vault-broker` system service.
Tooling (`xtask`) may depend on anything but the app and other tooling.

## Composition root

The application is the composition root. `src/main.rs` selects the entrypoint;
`src/startup.rs` owns paths, settings, design tokens and native collaborators prepared
once before asynchronous agent-preference hydration. Loading preferences does not
construct another filesystem, transfer queue or opener. The prepared inputs then
pass to `src/bootstrap.rs`, which installs services in one order shared with GUI
tests and registers independent workspace features. The vault adapter is passed
as a `VaultAccess` port; its installed feature handle owns settings observation
and is retained by the app. The application window factory reuses installed service handles rather than
initializing another vault or transfer queue.

## Menus

The composition root supplies Session/Edit/Search/Window/Help descriptors to
Workspace's native `AppMenuBar`. Feature commands are ordinary GPUI actions:
Workspace forwards them to the command item's focus handle and checks action
availability on its last rendered frame ([ADR 0002](../adr/0002-commands-through-dispatch.md)).
Each window snapshots enabled/checked state
before opening; an open popup retains its originating Item. A focused bottom
terminal takes priority over the central Item. Native clipboard/edit actions
continue to dispatch to native text fields, including dialogs. The maintained
[GPUI Component patch](../../vendor/gpui-component/NOCTERM.md) updates menu snapshots
without replacing native trigger entities or process-wide menu state.

## Generated documentation

`cargo xtask docs` derives field descriptions/defaults from schemars schemas
and serialized defaults, action descriptions from parsed `actions!` declarations,
shortcuts from the keymap, and crate dependencies from Cargo metadata. Generated
files are deterministic and contain no timestamps. `cargo xtask docs --check`
compares them without modifying files. CI also runs format, build, lint, tests
and rustdoc. Behavioral architecture notes in these documents remain authored text;
the dependency graph and references update from code.

Explorer's collaborators are composed before GPUI initialization: the native local
filesystem supplies session capabilities and the transfer runtime receives the same LocalFs port. Explorer retains the TransferService contract explicitly.
Bootstrap installs `nocterm_files::Deps` and returns `Installed` in `Booted`; every
window receives those handles through `build_workspace`, including GUI fixtures
that supply session's in-memory filesystem. Neither queue access nor completion
bindings require a files global.

The root composes one native `LocalProgramOpener` with the filesystem and transfer
ports in Files `Deps`. `Booted` retains Files installation across new windows, so
all windows share its external-program admission bound and transfer queue. GUI
bootstrap tests inject the session `FakeExec` program port explicitly.

Bootstrap also retains `nocterm_workspace::Deps`, containing the explicit layout
handle returned by layout initialization. Every window clones these dependencies,
so closing a window releases its views and subscriptions while surviving and new
windows keep the same optimistic layout state and shutdown participant. Services
remain the composition inputs; `Booted` owns the installed per-window dependencies.
