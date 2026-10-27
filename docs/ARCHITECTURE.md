# Architecture

The application is the composition root. `src/main.rs` loads paths, settings and
design tokens and passes them, with the SSH/local transports and the vault, to
`src/bootstrap.rs`, which installs services in one order shared with the GUI
tests and registers independent workspace features. The vault adapter is passed
as a `VaultAccess` port; its installed feature handle owns settings observation
and is retained by the app. The [dependency map](architecture/dependencies.md) is generated from
Cargo metadata; `cargo xtask architecture` enforces its declared layers and
rejects runtime GUI dependencies below the UI layer. Service crates sit beside
the UI layer: they own GPUI entities and background work but no views, so they
depend on GPUI alone, never on the component library, UI or features. The
[storage and security boundaries](ARCHITECTURE_SECURITY_AUDIT.md) records protections,
regression evidence and residual constraints.
The [architecture guardrails](architecture/guardrails.md) also measure existing
debt inside crates and prohibit its growth through an exact source inventory.

## Layers

| Layer | May depend on | GUI | Notes |
| ----- | ------------- | --- | ----- |
| foundation | foundation | none | [Foundation](architecture/foundation.md) |
| domain | domain, foundation | none | [Domain](architecture/domain.md) |
| adapter | domain, foundation | none | [Adapters](architecture/adapter.md) |
| service | service, domain, foundation | GPUI only | [Services](architecture/service.md) |
| ui | service, ui, domain, foundation | yes | [UI](architecture/ui.md) |
| feature | service, ui, domain, foundation | yes | [Features](architecture/feature.md) |
| app | everything but tooling | yes | [App and tooling](architecture/app.md) |
| tooling | everything but app and tooling | — | [App and tooling](architecture/app.md) |

Features never depend on each other; they meet in the workspace's seams
(items, panels, actions, the session factory). Accepted decisions and the
alternatives they rejected are recorded in [docs/adr](adr/README.md).

Each layer's notes describe the behaviour and boundaries of its crates; the
[dependency map](architecture/dependencies.md) lists every crate with its layer,
internal dependencies and purpose.

## Environments

A window belongs to one environment, local or one hub of the self-hosted Nocterm server, chosen
when the window is composed ([ADR 0097](adr/0097-a-window-belongs-to-one-environment.md)). The
local window is the application described above; a hub window offers only the hub's terminal
sessions, Snippets and the shared settings pages. The environment menu in the title bar opens an
environment in the window or in a new one. Four crates carry it: `nocterm-hubs-model` (domain:
addresses, trust rules, the `[hubs]` settings, the ports), `nocterm-hub-io` (adapter: the only
crate that speaks the hub protocol, which lives in a [separate contract
repository](adr/0096-hub-contract-outside-the-workspace.md); it keeps device credentials in
private files), `nocterm-hubs` (service: known hubs, their status, sign-in) and
`nocterm-hubs-ui` (feature: the menu, the sign-in dialog, the Servers settings page and the sessions panel). Attached
terminal tabs ([ADR 0095](adr/0095-attached-sessions-restore-and-detach.md)) show a hub's
session and detach when closed. The environments of the open windows are remembered and opened
again at the next start by `nocterm-window-set`
([ADR 0098](adr/0098-windows-and-their-environments-are-restored-after-a-restart.md)). A hub can also
be the first step of an SSH route, from any window: the transport asks the `RelayDialer` port that
`nocterm-hub-io` implements for a stream
([ADR 0100](adr/0100-the-hub-is-a-route-step.md)). User guide:
[hubs](HUBS.md).

## Plugins

The window shell hosts contributions: panels and status bar widgets. The shell's own
functions are core contributions; every other contribution belongs to a system plugin
that its crate declares and that is compiled into the application. One checked catalog
lists them with their placement hints ([ADR 0082](adr/0082-system-plugins-and-core-contributions.md)),
and the window draws the status bar and arranges its panel groups from a layout
resolved against that catalog ([ADR 0083](adr/0083-zoned-status-bar.md)).

User plugins are the second tier: files the user installs, built only with the cargo feature
`user-plugins`. A run loads them only when `[plugins] user_plugins` is set, and never in
`--safe-mode`; one gate in the application covers every load path, and the API surface is
checked against the latest release ([ADR 0093](adr/0093-user-plugins-start-off-and-the-api-stays-compatible.md);
author documents in [plugins](plugins/README.md)).

## Extension points

Add a new Item for another kind of tab, a Panel for another sidebar section or a
registered action for a command. Observe the terminal model or session contract
for integrations. Add another Transport implementation to support another kind
of session. Theme changes belong in the token source or its user override.
IDE features, collaboration, Vim mode and an extension runtime remain future
consumers of these boundaries.

The workspace seams are described in the [UI layer](architecture/ui.md#workspace);
the agent's layers in the [service layer](architecture/service.md#agents).
References and the dependency map are generated: see
[generated documentation](architecture/app.md#generated-documentation).
