# 0116. Cloud retention activates a stored signed baseline before deleting its native prefix

Status: accepted

## Context

A native execution archive and an SDK checkpoint cannot stand in for a signed
public Snapshot. A relay cursor also does not identify a signed event sequence.
A lost transport response must retry the exact durable publication, and a later
writer or terminal observation must not be discarded while retention awaits the hub.

## Decision

The Cloud domain separates the signed EventId from `OutboxDeliveryId`, the stable
transport deduplication ID. Both identities, original coordinates, material epoch
and ciphertext are persisted before the first HTTP request. Retries reconstruct
transport metadata from that durable record; they do not generate another ID,
writer, time, nonce or body. Hub storage receipts do not admit commands.

`JournalSnapshotPublication` stages, records storage and activates through separate
ports. Under the common tracked native write gate, the adapter captures covered
position P, the complete native execution archive and bounded actual signed public
records. The public encoder verifies/reduces those public records and creates
Snapshot marker S at P.next(1,1,false). Native frames supply no fabricated public
identities or payloads. Stage atomically stores S, its exact outbox and the pending
archive, updates protected current-head metadata and deletes nothing.

A tracked finite FIFO uploader releases the native gate during network waits.
It publishes earlier entries first, persists the matching storage receipt and
requests retention at the actual relay cursor T, purging through T-1. The receipt
is checked against the staged signed context, delivery ID and ciphertext digest.
Activation under a current fact-only writer installs the exact public baseline
and native archive P, deletes only records/outbox/delivery receipts covered by P,
and keeps S and all later records. The public inclusive floor is S; the native
exclusive floor is P. Current writer, control revision, tombstone and later head
remain intact. The active baseline retains its original signed writer and receipt.
Restart resumes the same durable transfer phases and IDs. A missing or inconsistent
baseline refuses retention rather than accepting an SDK/native substitute.

The execution archive preserves original ModelAction positions and matching
checkpoint bytes/context, including historical grants, usage and effect facts.
A genuinely absent action has no invented checkpoint anchor. Recovery ignores
publication markers, merges the archive with later native records and never uses
compaction to refund unknown usage or redispatch a mutation.

Actual allocation permits travel with immutable resident public publications and
returned native archives. The blocking native closure and tracked upload own them
until the corresponding buffers finish or the last shared carrier drops. Ordinary
and urgent publication share their exact configured command receive pools; immutable
accepted Stop/Revoke/Delete commands and their receipt-bound status retain urgent
capacity. Terminal facts and explicit original-effect reconciliation use a separate
finite cleanup reserve. Snapshot work has its own configured capacity and cannot
consume those urgent or terminal-fact reserves. No idle chat reserves worst-case
payload space, and no ambient budget or execution authority is discovered.

## Consequences

Contract crypto/transport, native SQL and coordinator composition remain separate.
Per-account/process resource policy supplies finite aggregate capacities and tracked
concurrency. Source or material changes fail closed; rotating material remains a
separate baseline-first transition and never relabels historical anchors. Native
snapshot/outbox layouts are explicitly versioned; layouts without durable delivery
identity require recovery instead of assigning replacement IDs.
