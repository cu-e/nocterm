# 0128. Agents transfer files through the shared transfer runtime, relaying between servers

Status: accepted

## Context

Agents move files today only by typing into a terminal or running `exec_command` on
one host. They cannot copy many files between this computer and a server, or between
two servers, without a person doing it, and a server cannot copy to another server on
the agent's behalf: the servers hold no credentials for each other and Nocterm keeps
its credentials to itself. A Cloud run needs the same ability on the runner.

`nocterm-transfer-runtime` ([ADR 0018](0018-transfer-runtime-capabilities.md)) already
streams between the local filesystem and SFTP with bounded workers, staged uploads,
atomic publication, reservations, collision policy and cancellation, but only for the
Files feature and only local to remote or remote to local. The terminal ports
([ADR 0032](0032-narrow-terminal-ports.md)) expose no filesystem of a terminal's
exact connection. A Cloud run's authority binds exactly one session
([ADR 0107](0107-cloud-run-authority-and-host-controlled-ports.md)).

## Decision

**Three tools.** The `nocterm` MCP catalog gains `transfer_files`, `read_transfer`
and `cancel_transfer`. `transfer_files` takes a source terminal and absolute source
paths, a destination terminal and an existing destination folder, `overwrite` and an
optional `yield_ms`. It starts the copy in the background, waits `yield_ms` (default
1000, at most 10 000) and returns the transfer id with the current report.
`read_transfer` waits only while the transfer is active; `cancel_transfer` stops one.
All three act on transfers the same chat started. Paths are absolute with no `.` or
`..` components and `~` is not expanded; each source is copied into the destination
under its own name, recursively. Existing files are skipped and counted unless
`overwrite` is true (`CollisionPolicy::Skip` or `Replace`); the destination is never
created; symbolic links and special files are skipped and reported. Two local
terminals are refused. A call carries 1 to 64 sources; each path is at most 4096 bytes
and all sources at most 64 KiB.

**Relay through Nocterm.** A copy between two servers streams through Nocterm over the
SFTP connections it already holds, so neither server needs the other's credentials.
`nocterm-transfers` gains `Request::Relay(RelayRequest)` and `TransferDirection::Relay`.
`Request::between(source, sources, destination, directory, collisions)` pairs two
`TransferEndpoint`s into an upload, a download or a relay, and refuses two local
endpoints (`SameLocalEndpoints`) and a destination equal to or inside a source on the
same host (`DestinationInsideSource`). Copying within one host is allowed.

**One engine.** `nocterm-transfer-runtime` executes relays with the scheduler, workers,
reservations, staging, cancellation and publication it uses for uploads and downloads.
The source tree is walked on the source `RemoteFs`, directories are prepared on the
destination, and each file streams from a `RemoteDownload` into a staged
`RemoteUpload` with double buffering: the next read batch is in flight while the
previous write batch is acknowledged. Only acknowledged bytes count; a size change
mid-stream fails the file; a failed or cancelled file leaves no partial file. The
destination publication code is shared with uploads (`remote_destination`).

**Pipelined reads.** `RemoteDownload::read_batch(max_bytes, count)` is an additive
contract method with a sequential default. The SFTP adapter issues up to eight
concurrent READs and keeps the in-order prefix up to the first short or end-of-file
reply. Downloads and relays use it, so no transfer pays a round trip per 32 KiB chunk.

**A narrow submit port.** Agents reach the runtime through `TransferRequests`
(`submit`, `progress`, `cancel`) declared in `nocterm-transfers` and implemented by
`TransferRuntime` and by the shared `FakeTransfers`. They need neither retry nor
drain, so `TransferService` is not widened. The request carries the endpoint
filesystems the consumer captured under its own authority; the port grants nothing.

**A per-chat book.** `OwnedTransfers<B>` in `nocterm-transfers` is a pure book over
the injected port: it bounds an owner to four active and sixteen retained transfers,
refuses ids it did not submit (`NotOwned`), caches final snapshots because the shared
runtime trims its history, and cancels active transfers whose binding became invalid.
The GUI chat and the headless run both use it. `TooManyActive`, `NotOwned` and
`Expired` are typed variants of `OwnedTransferError`.

**Terminal filesystems.** `nocterm-workspace-ports` gains the `TerminalFiles` port,
returning `TerminalFilesystem::Local` or `Remote { host, fs }` for a terminal's exact
current connection under an `ExpectedSession`; a stale session, a disconnected
terminal, a pending prompt or vault wait, and a terminal without a filesystem are
distinct `TerminalError`s (`TerminalCapability::Files`). The terminal feature
implements it. The agent chat captures both endpoints through it at execution time and
does not reconnect: a lost endpoint fails with a typed error telling the agent to open
the terminal again.

**Approval.** `transfer_files` is a writing call with a new `ToolCapability::Transfer`.
Under the write policy it asks unless both endpoint terminals hold a `Transfer` grant;
the card shows the source, the sources, the destination and the collision policy, and
"Allow transfers between these terminals" grants `Transfer` on both for the chat's
session. A new session of either endpoint drops its grant. `Execution` is not reused,
so "allow execution on this terminal" never silently authorises transfers. In the
Cloud grant, which is a contract format, `Transfer` maps onto the existing execution
bit and the grant bytes are unchanged. `read_transfer` needs no approval;
`cancel_transfer` only stops this chat's own transfer.

**Composition.** The application builds one `TransferRuntime` and gives it to Files as
`TransferService` and to the agent as `TransferRequests`, so agent transfers appear in
the user's transfer activity, share its queue and are frozen and drained by the
existing shutdown participant. The managed runner composes a remote-only runtime
(`TransferRuntimeDeps::remote_only`): it has no `LocalFs`, upload and download
requests are refused with `LocalUnavailable`, and a contained local target has no
`RemoteFs` and reports files unsupported. The headless executor
(`nocterm-agent-tools::headless`) holds one chat-scoped book in the retained tools
factory, shared by every run view, and cancels it on revoke and chat close.

**Lifetime.** In the GUI, Stop, chat close or restart and disabling AI cancel the
chat's transfers, as for commands; detaching either endpoint terminal or replacing its
session cancels that transfer. Files already published stay; staged files are
discarded. On the runner, transfers are chat-scoped and are not durable across a
runner restart; their effects are reported as observed, never unknown.

**Contract.** The Cloud tool arguments gain tags 9, 10 and 11 for the three tools
(`nocterm/cloud-tool-arguments/v1` is unchanged), journaled by `nocterm-hub-io` and
bounded to 64 sources.

## Alternatives

- **`scp`, `rsync` or `sftp` run on a server through `exec_command`**: needs
  credentials between servers, fails for local endpoints and reports no progress.
- **A separate agent transfer engine**: duplicates scheduling, reservation, staging
  and collision code, needs an adapter-to-adapter dependency, and hides agent
  transfers from the user's transfer activity.
- **Methods on `TransferService`**: the trait is wider than agents need and Files
  uses it for retry and drain; a port named for the agent need is narrower.
- **A factory port like `CommandJobsFactory`**: the book is bookkeeping over an
  injected port and has no I/O of its own.
- **Reusing `ToolCapability::Execution` in the GUI**: grants would cross between
  running programs and moving files. Only the Cloud bitmask, a contract format, stays
  coarse.
- **Two terminal ids in `target()`**: the approval, grant and reconnect machinery is
  keyed by one target; keeping the destination as the target and adding a source
  binding changes fewer call sites and leaves reconnect untouched.
- **Multi-target Cloud authority now**: a separate decision about run authority.

## Consequences

- **Cloud runs reach one session.** An `ExecutionAuthority` binds one session receipt,
  so on the runner a transfer is admitted only when source and destination are that
  session: a copy within the run's server. The headless code checks both endpoints
  independently, so a later decision that gives a run several targets changes the
  authority check, not the transfer path. That decision needs its own ADR; until then
  the tool description makes no server-to-server promise for Cloud.
- **Contract compatibility.** Older decoders reject tags 9 to 11 as an unsupported
  kind, so an old device cannot read a Cloud event stream that contains a transfer
  intent. The contract README records it.
- **A shared queue.** Agent and Files batches share the runtime's queue of eight
  batches and run one batch at a time. A long user transfer delays an agent's and the
  reverse; a full queue is `TransferError::Full`; the agent sees `queued`. The user
  can cancel an agent transfer in the Files activity and the agent sees `cancelled`.
- **Local reach.** A local endpoint reads and writes anywhere the user can, including
  Nocterm's own state directories. That is the reach `exec_command` has on a local
  terminal, behind the same write approval, so paths are not blocked lexically.
- **Retry reuses the original connection.** Retrying a relay from Files reuses its
  source `RemoteFs`; if that connection is gone, the retry fails per file.
- **Runner restart.** In-flight transfers are lost and remote staged temporaries may
  remain; they are recognisable by name and never fence later writes.
- **Downloads pipeline.** Every SFTP download now keeps up to eight reads in flight,
  so short-read and end-of-file handling are exact and covered by the SFTP tests.
- **Accepted ADRs 0018, 0032 and 0107 are unchanged.**
