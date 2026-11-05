# 0148. Cloud device delivery, presentation and persistence have bounded owners

Status: accepted

## Context

The device core's flows are shared with other clients ([ADR 0144](0144-the-device-side-runs-on-the-contract-device-core.md)).
Its adapter and GPUI consumers still own the resources used to run those flows.
An unbounded update queue retains successive copies of streamed text when the UI
falls behind. A subscriber that closes during a quiet Watch must wake the flow,
and a checkpoint generation check does not bound the queued physical writes.
A transcript changed before its admission check can retain refused data and
leave its renderer on another version.

## Decision

- `nocterm-hub-io` owns the original flow, its bounded request mailbox and its
  subscribers. Each subscriber has eight queued updates. One installation has
  a 64 MiB budget for queued immutable payload generations, shared between
  readers until handoff. A slow consumer applies backpressure; deltas are not
  silently dropped. Oversized initial state produces a typed capacity refusal.
  Dropping the subscription wakes the flow without waiting for a network hint;
  original account end interrupts blocked delivery. Pending command and server
  flows retain their independent lifetime. A retained pending data delivery pauses
  further chat reads while the bounded control mailbox and control effects remain
  active; a slow history consumer cannot stall Stop or approval answers.
  The mailbox and active delivery/server flows each admit at most 32 entries.
  Up to 32 new subscribers wait as channels; their initial snapshots are prepared
  one at a time, within a total admission limit of 256 subscribers per chat.
- `nocterm-cloud-chat` admits the candidate retained cost before mutating its
  transcript or renderer. Accounting follows changed rows rather than rescanning
  every retained row. A sequential background preparation owns the admitted
  allocation through actual parsing and converts changed rows and tool JSON.
  Acceptance checks the original account and subscription generation, then
  commits both representations together. Foreground batches yield to input.
- The hub adapter keeps one keyed, coalescing checkpoint drain, reusing the
  foundation `WriteQueue` and private atomic publication. It holds at most
  eight pending files and 64 MiB of pending allocations, plus one active
  snapshot of at most 32 MiB. Submission order belongs to this owner, so a
  reopened flow does not restart a counter behind an earlier writer.
- Checkpoint reads and the command outbox use a shared native owner admitting
  eight operations and running at most two blocking closures. The actual
  closure retains its permits after cancellation of its caller. Outbox resume
  bounds both the number and total bytes of commands. Physical preservation of
  an ordinary exposed `Deliver` command completes before the adapter starts
  publishing it, retaining the exact retry bytes required by
  [ADR 0146](0146-device-transport-and-resumable-commands.md). The core's staged SSH
  binding/certificate flows keep their existing separate publication path; its API
  does not expose their nested delivery's sealed retry bytes to the adapter.
- Checkpoints are replaceable cache snapshots; the writer keeps accepted work
  alive after its originating flow closes. Signed commands are lossless records
  and are not coalesced. Neither mechanism establishes account authority or
  replaces the core's verification.

## Alternatives

A periodic subscriber check would add idle work and delay teardown. Dropping
arbitrary deltas would break the consumer's verified prefix. Giving every
checkpoint its own detached worker would retain an unbounded number of buffers.
Moving synchronous outbox I/O to the async actor would occupy a network worker
while the disk is slow. Cloning the whole transcript before each delta would
preserve transactional failure but repeatedly copy unchanged history.

## Consequences

Refused growth preserves the accepted transcript and renderer. Slow consumers
and disks have explicit capacity behavior and cannot grow their queues without
bound. Capacity and native storage failures retain typed causes. These limits
bound the named owners; they are not a total-process RSS guarantee or a substitute
for measuring interactive frame latency.
