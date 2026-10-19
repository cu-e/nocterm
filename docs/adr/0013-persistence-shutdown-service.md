# 0013. Ordered persistence and quit capture belong to a service

## Context

The foundation write queue defines admission and shutdown ordering without an
executor. Each GPUI owner still repeated a worker, disk gate and quit timer.
Layout presents a user's resize immediately; settings instead publish a change
only after disk success. These policies must remain distinct.

## Decision

`nocterm-store` is a service depending on GPUI, foundation utilities and no UI
components. `Persisted<T>` owns a bounded `WriteQueue`, a `WriteGate`, immutable
visible/durable projections and typed completion receipts. Owners supply a
snapshot writer that captures its codec and backend without an App reference.

Optimistic edits update visible state immediately and coalesce pending snapshots.
Fire-and-forget latest-desired edits consume one pending marker, never one queue
slot per resize. Receipt-bearing edits retain bounded admission and Full errors.
A failed write retains desired state and the previous durable projection. Durable
edits are applied in order to the last successfully saved document, so a rejected
write never enters a later edit's base state during normal operation. Quit capture
synchronously applies the in-flight candidate and all pending edits to a final
document. Pending active intents may be retried by that final complete document;
errors already delivered during normal work are excluded. Successful active writes
retain their own receipt revision even when the final write fails; remaining
receipts resolve only after their covering final snapshot succeeds.
Checked edits return typed semantic failures without publishing partial changes;
expected-record checks run against the latest successful snapshot in normal work
and each previous accepted mutation during final capture. Edit callbacks
therefore need not be Send and never execute after App teardown.

Bootstrap creates an explicit `ShutdownCoordinator`. Participants register a
synchronous capture that freezes admission and takes final snapshots, pending
edits and acknowledgements while App access is still valid. Capture completes
for every participant before background work starts. Background final writes do
not read entities or the App, and wait on the same gate as normal writes.
All participants drain concurrently within one 180 ms deadline. Registration
retains each service entity until synchronous capture, so closing a view or dropping
its last external handle cannot discard admitted work. Service participants have
no coordinator back-reference, and final work retains no entity.

UI crates may depend on service crates; services still may not depend on UI or
features. Layout uses this service through its existing global compatibility
shell and existing quit hook. Settings also uses the common worker and coordinator. Its existing UI Global is
a private compatibility shell around an explicit service handle; runtime receives
that handle through its constructor dependencies, eliminating AiSettingsSource.
Snippets also delegates checked durable mutations to this worker and registers
with this coordinator; its old loop and hook are removed. Connections profiles,
recents and detected facts also use the common worker. Its old profile/recents/facts
loops and feature quit hook are removed. Remaining persistence
owners retain their current writers. Registration itself adds
no global or quit handler. This staged boundary does not make other owners'
existing separate quit deadlines part of the shared budget yet.

The profile backend serializes profiles only; private generation-stamped effects
track recent unlinks and credential assignments even when delete/re-add restores
an identical serialized profile payload. A synchronous publication hook admits
those derived recent edits before profile receipts complete, without depending on
an editor or caller task. Physical success is recorded under the common disk gate
before GUI acknowledgement, so capture preserves successful active writes.

Connections captures profiles and recents together. Exactly two oneshots expose
the active successful prefix and successful final document. The projected recents
runner saves independently accepted recent uses immediately, then the prefix and
final effects sequentially on the same gate, with at most three writes. Profile
failure never blocks independent bookkeeping or rolls back a physically saved
profile because a later recents write failed. Captured selectors contain only
immutable snapshots; no App access or owner mutation happens during drain.

## Alternatives rejected

- Copying another worker into layout preserves the duplicated lifecycle.
- Publishing all state optimistically changes settings' failure semantics.
- Capturing entities from an async quit future reads App state after teardown.
- Giving each participant a fresh timeout makes total shutdown time unbounded.

## Consequences

Views remain in the UI/feature layers; background orchestration has one service
owner. Backend errors retain their typed source. Receipts complete after normal
or final publication, and interrupted final work drops receipts with a typed
completion error. Layout's compatibility global is migration debt; dependency
injection will remove it when feature constructors receive explicit handles.
