# 0062. Terminal and connection contracts live below the window shell

Status: accepted

## Context

[ADR 0032](0032-narrow-terminal-ports.md) gave consumers narrow terminal ports
instead of terminal views. The ports, their `TerminalCapabilities` bundle,
`ExpectedSession` and `UserPromptToken` authority, `SessionContext` and
`ConnectionSummary` were declared in `nocterm-workspace`, a UI-layer crate that
also owns the window shell, tabs, menus and the component library. The contracts
themselves need only GPUI core and `nocterm-session`. Their home forced every
consumer into the UI layer or above: a service that drives agent tools on the
user's terminals could not name a terminal without depending on the shell.

## Decision

The `nocterm-workspace-ports` service crate declares the terminal contracts of
ADR 0032, `TerminalCapabilities`, `ExpectedSession`, `UserPromptToken`,
`LiveCommandLease`, the connection-facts session, the terminal DTOs,
`SessionContext`, `ConnectionSummary` and the typed `TerminalError` and
`TerminalCapability` refusals. It depends on `gpui` and `nocterm-session` only.

`nocterm-workspace` re-exports these names unchanged and keeps what concerns the
shell: `Workspace`, `Item`, `Panel`, `ConnectionDirectory` (which opens a
connection into a workspace), `OpenOnHostError` and the window actions.
Implementations stay where they are.

ADR 0032's rules on authority, refusals and capability advertisement are
unchanged; only the location of its contracts moves.

## Alternatives

Keeping the contracts in the workspace and letting services depend on the UI
layer would break the layer rule that services reach no views. Moving them into
`nocterm-session` would mix GPUI entity identities into a crate that has none.

## Consequences

Services can consume terminals through the same contracts as features. Existing
consumers keep importing from `nocterm-workspace`; new service consumers import
from `nocterm-workspace-ports`. A shared scripted terminal fake can live with
the contracts as test support.
