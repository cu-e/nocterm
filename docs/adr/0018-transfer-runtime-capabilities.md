# 0018: Transfer execution belongs to an adapter over held filesystem ports

## Context

Transfers combined request rules, progress, a Tokio runtime, native file operations
and entropy in a domain crate. Explorer depended on the concrete queue. A native
path reservation could confuse two paths to one held directory, and reopening a
source through its display location lost directory identity after ancestor replacement.
A consuming upload finish could not retry a late name collision without losing
its already acknowledged stage.

## Decision

`nocterm-transfers` owns pure requests, checked limits, collision decisions,
opaque caller-supplied IDs, typed diagnostics, progress transitions and
`TransferService`. It performs no I/O, entropy or scheduling. Queued cancellation
opens no stream. Cancellation becomes terminal only after discovery and all
workers drain; only acknowledged writes advance byte progress. Diagnostic details
are bounded independently of failure counts. Snapshots are newest first and only
finished batches are eligible for bounded history eviction.

`nocterm-transfer-runtime` implements the service as an adapter. Application
composition injects the same `LocalFs` capability used by Explorer. The adapter
has no dependency on a concrete filesystem adapter or GPUI. Its dedicated executor
admits eight queued batches, buffers 32 file jobs and runs four file workers.
Every synchronous port operation uses a bounded blocking dispatcher whose permit
remains with a started operation until it returns. Discovery, stream and queue
limits have distinct owners and checked constructors cannot raise production caps.

`LocalDirectory::open_source` opens one native name relative to that held directory.
Unix no-follow/nonblocking open and regular-file checks apply to the same opened
handle; callers do not recover a lexical path from its display location. Local
reservations use held directory identity plus the exact native name. Downloads
publish only through the held directory's already synced `LocalStage`; Rename
retries keep that same stage and Skip never replaces an external creator.

`RemoteUpload::publish_as` seals and publishes its existing stage under one
validated leaf in the originally admitted parent. It cannot change the parent or
escalate the original Create/Replace mode. A verified Create collision retains the
sealed stage for bounded name retries, and writes after sealing fail. Failed or
uncertain acknowledgements, close and publication prevent further publication.
Successful publication removes ownership before cleanup can run. Consuming
`finish` remains a compatibility wrapper over the same publication path. The SSH
adapter and shared filesystem fake implement this contract.

Explorer retains `Arc<dyn TransferService>` through explicit installed dependencies.
Feature tests drive a shared, explicitly controlled transfer-port fake; adapter
tests use session's filesystem fake, and tests combining native filesystem and
transfer adapters belong to application composition. The fake contains no scheduler
or duplicate transfer algorithm.

`freeze_and_drain` closes admission and requests cancellation before its future
is first polled. The future retains executor ownership until cleanup joins; root
composition can register it with the shared shutdown coordinator. No transfer
handler or independent shutdown budget is introduced.

## Consequences

Domain changes can be tested without a runtime or physical files. Concrete adapters
stay in the composition root, and Explorer dependencies are visible in constructors.
The former duplicated download filesystem implementation and domain complexity
exceptions disappear.

A started kernel operation on a hung filesystem may outlive its async waiter;
dropping a waiter does not interrupt a syscall. Publication awaits its answer so
an acknowledged commit is not reported as abandoned merely because cancellation
arrived concurrently. Root shutdown bounds the whole drain, rather than giving
transfers another timeout. Executor Drop retains a background shutdown fallback.

Portable native traversal retains its documented active-ancestor race limitation.
SFTP v3 cannot atomically enforce identity between remote metadata and open. Expected
sizes reject short or growing files, while same-size concurrent content mutation
remains undetectable. Remote directory replies are materialized vectors and then
validated against the aggregate pending-entry/name budget.
