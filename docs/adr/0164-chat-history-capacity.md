# 0164. Installation history capacity is separate from a single document

Status: accepted

Supersedes only the numeric history ceilings in [ADR0022](0022-agent-storage-owners.md),
[ADR0031](0031-archive-hydration-owner.md) and [ADR0142](0142-live-transcript-and-image-admission.md).
Their ownership, admission, delivery and eviction rules remain in force.

## Context

The installation reused the domain's 64 MiB single-document ceiling as its
aggregate history budget. Catalog work, immutable writer generations and hydrated
documents from every account and window competed for the same 63 MiB normal
lane. About 55 MiB of retained history could prevent a 14 MiB metadata scan or a
21 MiB document from being admitted. Separately, persistence kept only the last
2,000 rows even though live transcripts admitted 20,000 rows.

## Decision

`nocterm-agent-runtime::history_admission` owns an independent 1 GiB aggregate
history ceiling, preserving its 1 MiB and two-slot urgent lane. The existing
32-payload bound, weak participant accounting, clean hidden-history reclamation
and protected shown documents remain unchanged. The budget includes outstanding
catalog work, retained snapshots and prepared records across all windows/accounts.

`nocterm-ai::history` keeps the per-document policy: 256 MiB conservative
structural cost, 128 MiB encoded data and the most recent 20,000 transcript rows
with aligned timestamps. Live transcript byte admission uses the same 256 MiB
ceiling and retains its existing 20,000-row limit. The decoder and persistence
projection use one saved-row constant. Collection, image, header and scope checks
continue to refuse before unbounded work; their other limits do not change.

These are finite admission ceilings, not eagerly reserved buffers, measurements
of process RSS or an aggregate disk quota. History remains lazy; raising the
capacity starts no workers and preloads no transcripts. Existing formats and
ownership checks are unchanged. Existing saved rows are preserved; content
already discarded by an older writer cannot be reconstructed.

## Alternatives

- Increasing the shared single-document constant alone couples two different
  policies and lets one transcript consume the installation's entire budget.
- Removing admission can turn a long conversation or many windows into an
  allocation failure. It also removes headroom for urgent controls.
- Reserving the full new ceiling or eagerly restoring larger archives adds
  startup and idle memory cost without helping a user opening one saved chat.

## Consequences

Ordinary catalog refresh and hydration can coexist with much larger retained
histories. Longer conversations survive reopening without the old 2,000-row cut.
Larger accepted payloads can take more memory and time to encode, scan or render;
existing background I/O, single hydration delivery gate and eviction remain
necessary. Older binaries still enforce their earlier read ceilings and may
refuse new files that exceed them; no automatic migration rewrites old files.
