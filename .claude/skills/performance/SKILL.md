---
name: performance
description: Evaluate changed hot paths, rendering, repeated I/O or caches using representative costs and behavior. Use for optimization work or changes that affect input latency, redraws, large collections, background jobs or retained memory; not as a benchmark gate for every edit.
user-invocable: false
---

# Runtime cost and rendering

AGENTS.md's Performance rule applies. Keep notes and results in the existing
compact plan; this procedure creates no extra approval or completion stage.

## Find the cost

1. Name the user action and the owner doing the work: typing, scrolling,
   streaming output, opening a panel, refresh or background synchronization.
2. Inspect the whole affected path, including consumers. Estimate frequency ×
   work per event, collection sizes and retained memory. Look for repeated
   scans/sorts/parsing, large clones, allocations, task spawning and fan-out.
3. For an optimization, capture a representative baseline using existing
   profiling, tracing, counters or a focused benchmark. Choose the metric that
   answers the issue: input/frame latency, calls/bytes, CPU or peak/retained memory.
   Record workload, build mode and environment; compare the same conditions.
   Use distributions or repeated samples for noisy timing, not one fast run.

Compare plausible solutions when the tradeoff matters. First remove avoidable
work or improve the algorithm/data access; then consider batching, incremental
updates or caching. Account for invalidation, memory and maintenance cost.
Do not introduce a cache/framework just because it might help later.

## Keep interaction responsive

- Render and input callbacks consume prepared state; perform I/O and expensive
  parsing/decoding in the appropriate adapter/service with bounded background
  work. Moving work to a task does not make it free: bound concurrency/queues,
  deduplicate equivalent requests and cancel obsolete work.
- Notify the affected owner only when its observable state changes. Coalesce
  bursts without losing the final update or delaying required feedback.
- Reuse stable view identities and unaffected render regions. For large lists,
  prefer existing virtualization and prepare only visible/needed data. Avoid
  rebuilding the entire collection for one row or reparsing text every frame.
- Check theme/font/scale changes, resizing, scrolling, focus, selection and
  content updates where affected. A reused frame must still respond correctly.
- Background watchers/pollers have a defined lifetime and cadence; share a
  source when existing ownership permits it. Stop unused work on close/disable,
  or explain why the underlying service must continue independently.

## Cache contract

Before adding or changing a cache, state:

| Concern | Required answer |
| --- | --- |
| Value | Which repeated expensive work is avoided; why reuse is likely |
| Owner | Who owns entries and releases them; reuse an existing cache if it fits |
| Key | All semantic inputs, revisions and relevant account/session/source identity |
| Freshness | Events/revisions that invalidate entries; TTL only where staleness is acceptable |
| Bounds | Entry/byte budget, eviction and in-flight request limit appropriate to the data |
| Delivery | Old results cannot overwrite newer state or cross an account/session boundary |
| Failure | Miss/error/cancellation behavior; retries and negative caching are bounded |

Keep authoritative state and access checks outside the cache. Release entries
when their scope ends; do not retain secrets or stale privileges for reuse.

## Verify the affected behavior and cost

Exercise cold/miss and warm/hit paths, changed inputs/invalidation, eviction and
late completion where applicable. Reuse public behavior tests and shared fakes.
Cost assertions may count externally requested work (for example network calls)
when reducing it is the requirement; avoid private-field and timing-flaky tests.

For render changes, use relevant GUI/render tests and inspect the actual affected
interaction when a GUI session is available. Check visual correctness as well as
work avoided, including idle and repeated updates. A compilation pass or a static
screenshot alone does not establish responsive interaction or correct invalidation.
Report measured before/after results and tradeoffs; operation counts establish
less work, not a measured latency improvement. If profiling or GUI execution is
unavailable, state exactly what remains unverified. Completion checks use `gate`.
