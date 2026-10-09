# 0005. The agent runtime is a service crate reached through a client port

## Context

The agent runtime edited `AgentThread` fields directly (pending controls,
entries, dirty rows, persistence), and threads called back into the runtime:
a module cycle in which each side knew the other's internals. Unanswered
permission requests and released leases were special-cased by hand.

## Decision

- `PermissionResponder` answers `Cancelled` when dropped unanswered and
  `SessionLease` queues its own release when dropped.
- The runtime moved to `nocterm-agent-runtime`, in a new `service` layer: it may
  use GPUI (entities, tasks) but not the component library, UI or features.
- It knows no thread type. A thread registers a `SessionClient`
  (`Rc<dyn SessionClient>`) and applies the `SessionEvent`s the runtime emits,
  synchronously and in order; the runtime reads only a `ClientState` to
  schedule connections and captures documents through the port.
- Settings are read through an injected `AiSettingsSource`, because the
  settings store lives in the UI layer.

## Alternatives rejected

- An `AgentSession` entity with `EventEmitter`, as first planned: subscribers
  run later in the effect cycle, so events could interleave with other updates.
  The trait object gives the same inversion with synchronous, ordered delivery
  and no extra entity.

## Consequences

- `cargo xtask architecture` enforces the direction: feature → service →
  domain/foundation.
- The sign-in terminal opener stays in `nocterm-agent`, because its type names
  `Workspace`; the runtime only knows whether one is installed.
- Separating the thread's model from its composer state is not done yet.
