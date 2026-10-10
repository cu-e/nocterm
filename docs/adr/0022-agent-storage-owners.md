# 0022. Agent storage and lifecycle have separate owners

## Context

Runtime owned connection admission, leases, history/catalog data, preferences,
usage refresh and two separate persistence workers. The AI domain performed
history/favorites filesystem I/O, and the feature installed its own quit hook.
A file split did not give these responsibilities independent invariants.

Snapshot persistence already has checked/optimistic publication, ordered receipts,
physical projection before acknowledgement and one shutdown coordinator
([ADR0013](0013-persistence-shutdown-service.md)). Chat files require independent
per-key outcomes: one failed chat must not stop or revoke another successful chat.

## Decision

`nocterm-agent-storage` is a GPUI-free adapter implementing `ChatRepository` and
`AgentStateRepository` from the AI domain. The application creates and injects the
repositories; runtime and feature crates have no adapter dependency. AI owns the
bounded version-one codecs, identifiers, data and storage contracts. The adapter
owns paths, bounded reads, directory permissions, atomic publication, deletion,
filesystem catalog scanning and metadata sidecars.

The transcript is authoritative. A sidecar update is best effort and cannot
turn transcript success into failure. An archived-draft patch reads the original
record and changes only draft/updated fields under the service's serial chat gate.
Files retain the 32 MiB encoded and 64 MiB structural limits; metadata is bounded
to 16 KiB and catalog selection to 200 pinned/newest rows. Private publication
uses the shared foundation mechanism from [ADR0015](0015-private-file-publication.md).

Runtime is a coordinator with nine private fields. Responsibility owners are
AgentStateStore, ChatStore/DocumentRegistry/Catalog, ConnectionManager,
AdmissionState, UsageLimits, LeaseInbox and BridgeRegistry. AdmissionState owns its queue/task and a pure AdmissionPolicy. The policy decides
idle/yield/retry effects from supplied observations without GPUI. Runtime turns
those decisions into client events. Document identity, permanent deletion seals
and captured candidate owner/revision guard all publication notifications.

Preferences use optimistic `Persisted`. ChatStore uses one
`KeyedPersisted<String, ChatIntent, CandidateStamp>`. Both policies invoke Store's
same normal background executor/gate/physical recorder and captured runner.
They provide candidate selection and result application, not separate writer
loops. One immutable operation may execute at a time. Keyed ready capacity is
32 tokens, with at most one token per key. Updates retain the newest accepted
intent even when dispatch is full; refill preserves first-wait order.

Each physical success records its exact key/tag/generation before GUI
acknowledgement. A newer pending mutation cannot be stamped from an older result.
The owner completion hook records retention synchronously before notifications.
Document events are deferred until the keyed entity borrow ends and revalidate
the current owner before delivery, outside registry borrowing. Failure additionally
requires the latest accepted owner/revision stamp; an old failure cannot pause a
newer intent, while an old success still acknowledges its actual revision. A client may
synchronously retire or edit its Runtime document during that event. Failed generations remain
dirty without busy retry; explicit retry, a new intent or final capture resumes
that key. Other keys continue. Successful payloads are released; clean closed
keys also release success metadata. Failed closed keys remain until retry/capture.

Ready capacity does not bound total dirty documents. The client owns history and
queue admission, and codecs enforce existing per-record limits. The shared worker
retains one latest payload per dirty key plus its active immutable operation.
The UI's resident archive budget is a separate display/hydration limit.

The application loads initial preferences in background before installing
Runtime; missing/malformed initial state defaults retain a typed error and cause
no clean-quit rewrite. Catalog loading stays lazy. HistoryReader holds the shared
loading permit until the GUI accepts or drops a payload. PreparedHistory is an
optional construction input: exact retained records are validated without the
persistence serializer's truncation, with unique identities and aggregate
structural memory bounded to 64 MiB. It is consumed once by Catalog.

Installation registers one strong Runtime capture with the supplied
ShutdownCoordinator and installs no new global or quit hook. Runtime closes
normal intake before Shutdown events, captures current owner-checked snapshots
and deletion intents while App exists, then freezes the common writers. It drains
queued lease releases and retains cleanup for active, closing and unaccepted
startup processes. Each accepted command handle has a private lazy shared process
cleanup completion: ordinary close and final capture join the same attempt, including
the native method invocation. Final capture bypasses a pending session-close barrier;
a late close cannot repeat successful native cleanup. An already failed normal attempt
retains ownership and gets one final retry. Preferences, chats and process cleanup start independently
under the coordinator's single 180 ms budget; captured work accesses no App.

Snapshot receipts retain their established distinction: an actually published
active prefix completes at its own revision even if the final write fails or
waits indefinitely; an unwritten prefix completes at the covering final revision
only after that final publication succeeds. Connections' baseline/prefix/final
projections keep one gate and at most three writes.

## Alternatives rejected

- One writer/entity per chat duplicates lifetime and quit scheduling.
- Persisted<HashMap> implies all-or-none publication for independent files.
- Moving Runtime methods into more files keeps shared mutable responsibility.
- Dropping or rejecting dirty keys when ready dispatch fills loses accepted work.
- An adapter-specific writer or quit timer duplicates the shared service.
- Reading views during asynchronous teardown loses the final accepted snapshot.

## Consequences

Runtime's existing global remains compatibility debt; it is not authorization for
new foreign service reads. Version-one SavedChat/SharedChat still contain legacy
ACP DTOs, and other AI usage/sandbox/image/clock/entropy work remains migration
debt. This decision physically removes history/favorites I/O and duplicate
persistence scheduling without claiming protocol-neutral history or complete AI
purity. New storage APIs and persistence events preserve typed errors and source
chains; existing untouched display/bridge APIs retain their separately tracked
debt.
