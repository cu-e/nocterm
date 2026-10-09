# Architecture

The application is the composition root. `src/main.rs` loads paths, settings and
design tokens and passes them, with the SSH/local transports and the vault, to
`src/bootstrap.rs`, which installs every global in one order shared with the GUI
tests and registers independent workspace features. The [dependency map](architecture/dependencies.md) is generated from
Cargo metadata; `cargo xtask architecture` enforces its declared layers and
rejects runtime GUI dependencies below the UI layer. Service crates sit beside
the UI layer: they own GPUI entities and background work but no views, so they
depend on GPUI alone, never on the component library, UI or features. The
[architecture and security audit](ARCHITECTURE_SECURITY_AUDIT.md) records findings,
regression evidence and residual constraints.

## Layers

| Layer | May depend on | GUI | Notes |
| ----- | ------------- | --- | ----- |
| foundation | foundation | none | [Foundation](architecture/foundation.md) |
| domain | domain, foundation | none | [Domain](architecture/domain.md) |
| adapter | domain, foundation | none | [Adapters](architecture/adapter.md) |
| service | service, domain, foundation | GPUI only | [Services](architecture/service.md) |
| ui | ui, domain, foundation | yes | [UI](architecture/ui.md) |
| feature | service, ui, domain, foundation | yes | [Features](architecture/feature.md) |
| app | everything but tooling | yes | [App and tooling](architecture/app.md) |
| tooling | everything but app and tooling | — | [App and tooling](architecture/app.md) |

Features never depend on each other; they meet in the workspace's seams
(items, panels, actions, the session factory). Accepted decisions and the
alternatives they rejected are recorded in [docs/adr](adr/README.md).

Each layer's notes describe the behaviour and boundaries of its crates; the
[dependency map](architecture/dependencies.md) lists every crate with its layer,
internal dependencies and purpose.

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
