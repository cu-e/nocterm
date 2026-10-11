# ADR 0029: Bounded append persistence and output recording

## Context

Session output logging previously ran on a dedicated thread in `nocterm-terminal`, using an unbounded sync channel, its own shutdown loop, and an arbitrary 150 ms quit handler. Output recording needs bounded queues, native error preservation, pinned file descriptors, and integration into the single application shutdown coordinator.

## Decision

1. Extract runtime-neutral append contracts (`OutputLog`, `OutputLogFactory`, `AppendBlock`, `PhysicalPrefix`, `AppendOutcome`) into `nocterm-session::recording`.
2. Implement native append file writing in `nocterm-local::recording`, creating files with `0600` permissions and holding the descriptor pinned.
3. Drive append operations using the shared `nocterm-store` worker with `AppendPersisted`, `CapturedAppend`, and `RecordingRegistry`.
4. Provide a private `OutputRecording` state owner in `nocterm-terminal` that delegates to `nocterm-store::AppendHandle` without dedicated OS threads or polling loops.
5. Coordinate shutdown through `ShutdownCoordinator` under the single application budget.

## Consequences

- Terminal output recording uses the shared store worker and single shutdown deadline.
- Recording files remain pinned and preserve exact partial physical prefixes on failure.
- Terminal struct field debt and impl spread are reduced.
