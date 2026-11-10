# 0109. Cloud commits and retained effects share one native durable journal

Status: accepted

## Context

A model checkpoint cannot acknowledge a prompt, release an SSH effect, or prove
that a background command has finished. A lost acknowledgement must not admit a
second command, and an observed command handle must not erase an outstanding
mutation when the original run or worker is replaced.

## Decision

`nocterm-cloud-runner` is an adapter without GPUI or provider/SSH/hub SDKs. Its
`DurableRunJournal` implements the Cloud domain writer, reader, retained command
lifecycle and cross-run target-effect lookup ports. Application composition supplies
the journal cipher and, before background commands are enabled, an event encoder.
There is no plaintext production cipher or ambient key discovery. Native binary
records use an explicitly versioned bounded Postcard codec, independent from signed
hub formats. The cryptography owner binds stream, storage class, opaque item id,
journal sequence, stream revision and writer epoch; decoded coordinates are checked
again. Plaintext serialization buffers are zeroized after use.

One SQLite transaction covers writer fencing, expected stream/control revisions,
immutable command admission, run transition records, derived checkpoint, encrypted
effect indexes and exact outbox bytes. It uses DELETE journaling with synchronous
EXTRA, including SQLite's directory synchronization after removing the rollback
journal. No provider, network or target operation is held inside that transaction.
A successful result follows SQLite's commit acknowledgement; nothing fallible is
performed after that acknowledgement and then reported as a failed append. Duplicate
command lookup precedes current control/writer checks and returns the original
receipt. Changed semantics or original issuer attribution conflict.

Account plus chat selects the unique physical stream. Runner id and key epoch are
checked authority and cannot create a second partition. Claims compare and advance
the current writer epoch. Changing runner assignment or key epoch requires a
separate explicit migration and currently fails closed. Per-run FSM revision,
stream revision, journal sequence, event sequence and control revision retain their
separate meanings, including several public events referring to one journal record.

A background job links to its exact already stored intent and dispatch stamp
before its runner is polled. Retained, observed exit and unknown outcomes have
their own run-scoped records; they do not fabricate foreground FSM revisions.
Each observation, the original effect index, its public encrypted event and the
chat head commit atomically. A Starting/Running foreground result never resolves
that retained mutation. Cancellation, missing exit and worker loss remain unknown;
a final observation or explicit original-intent reconciliation resolves only that
one effect. A replaced writer cannot complete the old worker's job.

An authenticated chat header lists its active mutating intent ids. Target lookup
reads bounded headers and their bounded active effect records, so editing an SQL
state filter cannot hide a required fence. It compares the original account, host
key, principal and canonical workspace; route and profile aliases cannot bypass
it. Target/principal/workspace data and their unkeyed hashes are not exposed as
search indexes. Clear columns contain only opaque routing coordinates, bounded
positions and lengths. Whole valid database rollback or deletion by a party able
to replace the runner's private storage cannot be detected without an independently
retained trusted head; this adapter does not claim that protection.

Replay checks lengths before loading a body, checks exact sequence and native
record ownership, and refuses missing prefixes or capacity rather than silently
skipping them. An authenticated delivery watermark permits atomic snapshot
installation and prefix compaction. Old journal and event cursors return separate
typed gaps. Immutable command receipts, tombstones and unresolved effects survive
compaction. Storage charges release replaced checkpoints and compacted payloads;
SQLite page and per-chat limits remain finite, and full storage refuses new work.

The supervisor supplies an existing private directory with trusted ancestry.
Native startup refuses linked ancestors, nonregular or linked database/sidecar
leaves, multiply linked database files and unsafe Unix permissions/ownership. It
reuses the foundation's exclusive 0600 creation, synchronizes initial file and
directory publication, and opens SQLite with NOFOLLOW. SQLite creates its rollback
journal with the database's permissions. Its pathname VFS does not offer the
held-directory rename guarantees of `LocalDirectory`; hostile replacement of
trusted ancestry is outside this deployment boundary. Blocking journal futures
are polled on the composition's background/blocking pool. A handle refuses
concurrent capacity instead of keeping an unbounded queue.

## Alternatives

Atomic snapshot files alone cannot combine command receipts, effect admission and
stream/outbox progression. An independent background-job ledger could disagree
with the authoritative transcript. Using raw SQL state as the target-fence index
could hide an outstanding mutation. Reusing command or key-grant crypto purposes
for native journal bodies would mix security contexts.

## Consequences

The coordinator still owns signed command semantics, current grants, replay into
the run state, authority checks immediately before actual I/O and cleanup. The
cryptography and event adapters must be composed before native background tools
can start. File-backed tests exercise restart, concurrent writers, native write
failure and process death before/after commit; they do not simulate a physical
power failure. Cryptographic correctness is verified by the separate contract
suite, while journal tests use an explicitly test-only reversible codec.
