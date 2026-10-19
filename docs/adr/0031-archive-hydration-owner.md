# 0031. Archived chat hydration has a private owner

## Context

Archived chats retained metadata, pending actions, draft edits and delivery permits
as separate fields of AgentThread. Pending actions were arbitrary callbacks that
could mutate the whole coordinator. Panel observers directly dropped its permit.
A session ticket alone could not distinguish separate hydration attempts within
one unchanged session.

## Decision

A feature-owned Archive holds six private fields: phase, pending typed intentions,
draft overlay, resident byte count, delivery permit and the next local generation.
It performs no filesystem I/O. The injected runtime HistoryReader continues to
own bounded reads and the shared loading gate from [ADR0022](0022-agent-storage-owners.md).

Requests are Open, Fork, Rename or Pin. The owner starts one load per archived
phase and queues later intentions in accepted order. Each completion validates
its local generation in addition to the coordinator's current session ticket,
AI enablement and Runtime document ownership. A stale completion drops its own
permit without consuming current pending work or the literal draft overlay.
Failure clears only that generation's intentions and permits a new attempt.

Successful delivery yields a concrete hydration outcome. The coordinator restores
transcript, queue, configuration and composer together, applies the latest draft
and its edit timestamp together, then interprets typed intentions and warms a
shown chat. Open retains the original weak panel, document entity and composer
revision. Fork retains the original weak panel, document entity, row and window;
its continuation uses the existing fork command only while that source and window
still exist. Panel updates are deferred until the document borrow ends.

The Archive retains an accepted HistoryPermit through hydration and resident
trimming. The panel invokes a named release command after trimming; it has no
mutable access to the permit. Clean durable hidden documents may be evicted only
when every existing detached, idle, revision, draft, error and saved-state gate
passes. The coordinator clears transcript, composer and presentation budgets as
one eviction effect.

An edited archived document produces the existing archived-draft patch. An
untouched archive produces no snapshot; storage retains the full original
transcript, queue, attachments and configuration when applying a draft patch.
The 300 ms draft debounce, 64 MiB resident budget and native quit budget retain
their established behavior.

## Alternatives rejected

- Moving more AgentThread methods into child files preserves shared mutable
  responsibility and obscures the coordination boundary.
- A callback receiving a mutable Thread or Composer defeats typed owner effects.
- Reusing only the session ticket permits an old attempt to consume a newer
  attempt's pending intentions.
- Releasing delivery before hydration and trimming weakens the shared payload bound.

## Consequences

AgentThread is still a migration coordinator, not the completed ownership split.
Its remaining session, document, configuration and tool owners and AgentPanel's
presentation owners require separate extractions. This decision introduces no
protocol-neutral domain API and does not complete the ACP migration.
