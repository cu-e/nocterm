# App and tooling

Part of the [architecture overview](../ARCHITECTURE.md).

The app layer is the composition root and may depend on everything but
tooling: `nocterm` and the optional `nocterm-vault-broker` system service.
Tooling (`xtask`) may depend on anything but the app and other tooling.

## Composition root

The application is the composition root. `src/main.rs` selects the entrypoint;
`src/startup.rs` owns paths, settings, design tokens and native collaborators prepared
once before asynchronous agent-preference hydration. Loading preferences does not
construct another filesystem, transfer queue or opener. One SettingsConfig is prepared before
hydration and consumed once to install the explicit settings handle afterward. Checked ordinary and
startup page catalogs are retained by the existing Booted settings slot; a catalog error quits
before application registration or first-window creation. `src/shell_setup.rs` lists
every system plugin once and checks them with the shell's groups and core contributions
as one `PluginCatalog`; a failure is `BootstrapError::Plugins` and quits like an invalid
settings catalog ([ADR 0082](../adr/0082-system-plugins-and-core-contributions.md)).
Every window's workspace receives that catalog through its `Deps`. The prepared inputs then
pass to `src/bootstrap.rs`, which installs services in one order shared with GUI
tests and registers independent workspace features. The vault adapter is passed
as a `VaultAccess` port; its installed feature handle owns settings observation
and is retained by the app. The application window factory reuses installed service handles rather than
initializing another vault or transfer queue.

Bootstrap supplies Projects with the existing HostServices local executor and
shared read-only directory readers. It explicitly injects NativeImageSearch alongside
preparation; each search uses the captured original executor and selected engine.
It constructs the runtime command factory, injects it into
`nocterm-project-environment::NativeWorkspaceEnvironment`, then supplies that environment
to runtime preparation through the separate creation installer port. Observed settings
and explicit tool changes receive the same environment handle. Environment-owned
`NativeWorkspaceSessions` is supplied separately to Terminal for
folder workspace activation; `nocterm-project-container-fs::NativeContainerSessions`
receives the same activation capability for its scoped resource. Ordinary connection transports retain their existing launch behavior.
The controlled managed paths are exported before the configured workspace shell,
without reading a mutable environment fragment or editing shell startup files.
All three adapters depend only on domain/foundation contracts, including their test
dependencies. Deterministic cross-adapter behavior tests live in the app project-setup
composition owner ([ADR 0080](../adr/0080-project-adapter-ownership.md)).
Local creation needs neither a PTY nor a
synthetic terminal receipt; saved targets open through the existing local resolver.

Workspace terminal resolution composes retained Connections and Terminal installations
through `nocterm-workspace-ports::ProjectTerminalResolve` and an explicitly supplied provider registry.
Project terminal consumers import the lower contracts directly; the shell keeps
private captured placement and typed admission ([ADR0079](../adr/0079-project-terminal-contracts-below-shell.md)).
The registry rejects duplicate provider identities; an unknown provider remains
unavailable instead of selecting a host fallback. Connections resolves a saved profile and checks
the original destination and ordered jump route without opening a connection.
Terminal then opens an ordinary folder session inside the workspace, where normal
host-key, password and vault prompts appear. Resolution creates no additional
server tab ([ADR 0070](../adr/0070-original-workspace-connection-recovery.md)).
Container resolution supplies the container-session adapter through retained
terminal dependencies. Captured host-backed preparation uses the existing
connected executor; that requirement does not apply to every provider reference.
Generic resource identity and concrete built-in codecs have separate domain
owners ([ADR 0071](../adr/0071-provider-workspace-locations.md)).

The same supplied local filesystem and captured remote filesystem capabilities
also provide read-only directory readers. Files and Projects receive those readers
explicitly; shared UI controls acquire neither native filesystem access nor the
current session through a global lookup
([ADR 0072](../adr/0072-shared-directory-selection.md)).

The same retained installations implement the narrow `ChatConnectionOpen` port.
It resolves an explicitly attached saved server with its current credentials and
jump route, checks the original live surface before creating a normal terminal,
and registers that terminal as background membership on that surface. The
connection directory and general session factory keep their existing contracts.

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
without replacing native trigger entities or process-wide menu state. The Window menu
lists the layouts, with the selected one checked, and the panels the layout places,
by asking the workspace; `SwitchLayout` and `TogglePanel` carry an id and are sent only
by the menu. `src/shell_setup/reference.rs` renders the application catalog for the
golden test of `docs/reference/plugins.md`.

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
handle returned by layout initialization and the shared settings handle, through which
every window follows the `[layouts]` section (ADR 0084). Every window clones these dependencies,
so closing a window releases its views and subscriptions while surviving and new
windows keep the same optimistic layout state and shutdown participant. Services
remain the composition inputs; `Booted` owns the installed per-window dependencies.

Composition supplies the ACP native workspace resolver and the built-in agent
catalog from `nocterm-agent-providers` through AgentServices;
`nocterm_ui::Assets` serves the catalog's brand marks. The sandbox display probe goes
to `nocterm-agent-settings-ui` with the agent installation's `AgentsHandle`;
the Settings host retains only the supplied SettingsHandle. Native sandbox probe
selection happens at startup composition; feature/service fixtures
supply AI's shared scripted capabilities rather than reaching adapters or globals.
Startup constructs NativeImageCodec and passes ImageDeps through bootstrap Services.
Booted retains those dependencies for all windows; GUI fixtures supply the shared
scripted AI codec. Image ports are not discovered through globals or Runtime.

Prepared connection observation dependencies include an explicit remote OS detector:
startup supplies `NativeOsDetector`; memory composition supplies an inert detector
that performs no remote acquisition. Bootstrap consumes the prepared collaborators. Installation supplies the existing
App background executor to raw FactsService; isolated blocking-port tests explicitly
supply a threaded test executor. This creates no production executor/pool or global.

Booted retains the explicitly supplied chat metadata capability before consuming
AgentServices and passes it to each window's agent registration. Native startup
uses agent-storage's NativeChatMetadata; fixtures supply shared deterministic
metadata. Panel registration binds the capability to its originating weak
workspace without a global lookup. See [ADR0040](../adr/0040-native-chat-metadata.md).

Startup prepares the native local-session factory and default shell once before
preference hydration. Bootstrap retains the actual Connections entity for saved
credential association, composes vault credentials and recording before terminal
installation, and binds the existing Send+Sync authentication factory to that
installation. `Booted` shares it with every workspace window. Authentication tabs
retain explicit isolated transports and once-only completion; no transport or
credential compatibility global participates. See [ADR 0042](../adr/0042-terminal-installed-dependencies.md).

Bootstrap retains the optional connection `Installed` handle in Booted and passes
it to every new workspace. Vault association captures its exact connection entity
rather than discovering a global. An installation failure skips connection view
registration and returns a typed association failure while keeping vault access.
All windows share the retained profile/facts/settings owners and subscription
lifetimes; native preparation remains the earlier background capability boundary.


## Supplied local host execution

Native startup supplies the optional local `HostExec` implementation once. Bootstrap
retains that same executor with the window dependencies and supplies it explicitly
to the retained resource-monitor installation and container registration. New windows clone the retained
capability; test composition supplies an inert fake or None before installation.
Workspace host resolution uses the provided capability while preserving the active
remote session's executor and connection authority. It stores no global runner.

Bootstrap retains one agent `Installed`, combining actual runtime, metadata,
image ports, shared layout/settings and optional terminal authentication. Every
workspace receives that installation. Agent installation returns usable authority
and a separately typed shutdown registration status; reporting a registration
failure preserves the existing available feature. Capture holds RuntimeHandle,
never Installed or Workspace. See [ADR0052](../adr/0052-retained-agent-installation.md).

Bootstrap retains the optional snippet `Installed` handle over the library and
supplied settings. Successful installation is reused by every workspace, editor
and action; a shutdown registration failure reports its typed error and omits the
feature. Closing the first window does not change the library used by surviving
or newly opened workspaces. See [ADR0050](../adr/0050-retained-snippet-installation.md).


Bootstrap retains the exact KeymapHandle installed from application defaults and
the optional user file. Settings/OpenKeymap page factories and NewWindow clones
share that installation; closing its first workspace does not reload or replace
bindings. Services remain composition inputs rather than a lookup registry.
Application directory actions capture config and fallback state/logs paths once.
LogsDirectory reads the supplied settings handle at click time, so only published
LoggingOptions changes affect the next native open and Some(empty) stays explicit.
Directory authority comes only from captured paths and settings. The existing native
create/open background lifecycle and notices remain unchanged. See
[ADR0051](../adr/0051-retained-keymap-installation.md).

Bootstrap supplies the exact UI settings owner to settings-ui Installed.
Booted retains that installation in its existing
settings slot and supplies it to ordinary Settings, guest-page deep links and the
startup vault page. Opening another window does not install another document or
rediscover a compatibility settings global for these builtin consumers.
Explorer installation retains its prepared native ports together with the actual
settings handle. Its settings page factory captures the same policy as its panes.
Projects receives the same settings handle during installation and registers its
provider-owned Workspaces section. The app composes its lazy settings factory only
when that installation succeeds, alongside Keymap, Monitor and Explorer. The
startup Vault-only guest catalog remains independent. No additional settings
document or compatibility lookup is created.

Bootstrap also retains a Monitor installation over that settings owner and the
supplied optional local HostExec. Per-workspace monitor models and lazy settings
pages receive it; constructing the installation or a page factory starts no sampling.
The ordinary page vector and deep links preserve their existing order and IDs.
Vault installation retains the actual settings owner used to initialize its native
auto-lock duration. Its published-policy observer and all page factories use that
same handle. The startup prompt reads it at workspace construction rather than
acquiring settings through a compatibility lookup. Native service creation and
credential/terminal association keep their existing order and collaborators.
Bootstrap retains one ThemeUi for every application window. Each existing native
window-appearance callback captures that projection and applies its current published
mode. Design remains paint data; neither Design nor the toolkit theme discovers a
settings service. Closing one window releases its native callback while surviving
and later windows keep the root projection.

Snippet composition acquires the supplied native document at the existing installation
phase, prepares its checked service state and supplies a repository-backed snapshot
writer to SnippetsHandle. Only successful shutdown registration exposes Installed to
windows. The same installed library/settings and native identity capability are reused
across windows; UUID acquisition occurs only for an actual new editor draft.

Booted retains its original theme and optional vault installations privately;
composition reads named borrowed projections when registering windows and settings.
