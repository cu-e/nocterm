# 0056 — Settings pages and schemas live with their owners

Status: accepted

## Context

The generic settings Item retained Appearance, terminal, local-shell, SSH and AI
controls. This made unrelated features share a mutable parent and left pure
schemas in presentation crates. UI terminal styling needs terminal display
settings without a dependency on the terminal feature. Explorer indexing and
opener selection are rules independent of native traversal or program launch.

## Decision

Shared UI owns the Appearance page and browser. Terminal supplies its three
pages through its retained installation; Agent supplies its AI page, retaining
its original settings and injected SandboxProbe. Native capabilities stay in
adapters. Workspace action registration remains in the app; UI exports narrow
picker operations without depending on the workspace.

Terminal display schemas live in `nocterm-terminal-model`; Explorer settings and
opener rules live in `nocterm-explorer`. Monitor and Vault schemas live in their
existing domains. Serde formats, sanitization and public schema names and IDs
remain stable. The foundation settings schema builder owns ordered property and
default construction without application policy.

The app supplies the existing five builtin factories and ordered guest factories.
Builtins are constructed eagerly with the Item; guests remain lazy and reused.
An additive SettingsPage flush operation preserves all-builtin submission before
selection, guest deactivation and close. The per-Item feedback handle shares only
the save diagnostic; it provides no access to the host view or page internals.
Text controls retain the shared checked-receipt owner from ADR0054.

## Alternatives

Putting schemas into Terminal or Files would make UI or another feature depend on
a feature. Placing them in Session or Core would mix display/Explorer policy with
unrelated contracts. Keeping controls behind callbacks into SettingsView would
retain the oversized parent responsibility. Making builtins lazy or changing the
guest catalog would alter lifecycle behavior and is a separate decision.

## Consequences

Settings UI owns navigation, diagnostics and page lifecycle only. Each actual
page retains its own controls, observations and original authority. The generic
host has no AI, theme-service or native probe dependency. Existing fixed builtin
slots remain compatibility metadata; this decision does not redesign the catalog.
