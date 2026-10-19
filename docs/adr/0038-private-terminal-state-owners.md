# 0038 — Private terminal state owners and admitted input presentation

Status: accepted. Complements [0032](0032-narrow-terminal-ports.md).

## Context

The terminal model coordinated session lifetime, prompts, input encoding and live
command ownership, screen/search presentation, credentials and recording through
one set of fields. The view shared mutable frame-cache cells with its element and
mixed keyboard/composition, cursor timing and pointer transitions. Generic mutable
emulator callbacks made selection and paint invalidation depend on caller discipline.

A rejected transport input could also clear selection or return the viewport to
live output before admission. Input admission, live authority and presentation
therefore need explicit transitions with a defined effect order.

## Decision

Private feature owners separate the model's responsibilities:

- `SessionLifecycle` owns the original session, filesystem/prompt authority,
  connection and prompt epochs, pump retirement and once-only command completion.
  It commits retirement and status before returning external effects. Program tabs
  retain their host context after normal program exit.
- `TerminalInput` owns incremental conversion, shell integration, effective shell
  syntax, input notices and live-command ownership. An immutable prepared submission
  separates human, agent and protocol input. Validation precedes one synchronous
  admission to the captured session; only accepted bytes commit integration changes
  and human lease revocation. A live lease is installed after accepted command input
  within the same entity update. Refusal installs no lease and edits no screen state.
- `TerminalScreen` owns emulator, search, synchronized updates and presentation
  revision. Selection and scrolling use named transitions. Read-only emulator
  projections grant no mutation authority. Revision advances before Output delivery.
- `CredentialState` owns private candidate, lookup epoch/task, message and vault-wait
  transitions. `OutputRecording` retains its shared persistence driver.

The pump remains a bounded yielding runner, not another entity. Captured epochs
are checked before a batch can mutate input, recording or screen state. Completion
callbacks retain their existing synchronous contract; observing the same model from
an App effect follows GPUI's entity borrow rules and occurs after its update releases.

Private view owners separate frame publication, keyboard/composition, cursor timing
and pointer state. `FrameCache` publishes by source generation, presentation revision
and size, owns palette invalidation and geometry, and supplies immutable paint reads.
`TerminalElement` receives the private shared cache through typed construction.
`CursorClock` retains one task across input bursts. `ViewInput` retains bounded key
ownership and composition, while `PointerState` owns selection/autoscroll and protocol
state and receives immutable transient host dependencies.

Accepted typed input prepares selection/viewport after admission, within the same
synchronous update. Unchanged live input waits for echo. Accepted native path paste
uses a separate named completion that preserves its unconditional single Output
publication, even on a clean live viewport. Paste bytes, bracketed-paste framing,
final Enter policy, focus and captured authority remain the existing contracts.

## Alternatives

Private field bags or mutable owner projections would relocate access without
establishing transition invariants. A generic emulator mutation callback would keep
publication dependent on arbitrary caller edits. Publishing a live lease before
admission would introduce a reservation/rollback protocol that the synchronous
session contract does not require. Additional GPUI entities would add lifetimes to
state that already belongs to one model or view.

## Consequences

Model and view coordinators remain small private owners of their collaborators.
Tests drive constructors, session events, actions, ports and immutable rendering
projections rather than assigning session, codec, epoch or cache fields. Shared
session test-support retains prepared channel identity for public lifecycle fixtures.

This decision changes no transport, storage format, recording/shutdown ownership or
workspace capability boundary. Existing transport/local-factory/credential globals
remain installation dependencies; explicit installation is a separate composition
boundary and is not supplied by these state owners.
