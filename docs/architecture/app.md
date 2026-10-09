# App and tooling

Part of the [architecture overview](../ARCHITECTURE.md).

The app layer is the composition root and may depend on everything but
tooling: `nocterm` and the optional `nocterm-vault-broker` system service.
Tooling (`xtask`) may depend on anything but the app and other tooling.

## Composition root

The application is the composition root. `src/main.rs` loads paths, settings and
design tokens and passes them, with the SSH/local transports and the vault, to
`src/bootstrap.rs`, which installs every global in one order shared with the GUI
tests and registers independent workspace features. The application window factory reuses global services rather than initializing another vault or transfer queue.

## Menus

The composition root supplies Session/Edit/Search/Window/Help descriptors to
Workspace's native `AppMenuBar`. `ItemCommand` routes feature operations without
Workspace depending on Terminal. Each window snapshots enabled/checked state
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
