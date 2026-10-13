# Service layer

Part of the [architecture overview](../ARCHITECTURE.md).

Service crates own GPUI entities and background work but no views: they
depend on GPUI alone, never on the component library, the UI layer or features,
and UI/feature crates depend on them. `nocterm-store` owns ordered persistence
and synchronous quit capture for layout, settings, snippets, connection state and agent preferences/documents ([ADR 0013](../adr/0013-persistence-shutdown-service.md)).
`nocterm-agent-runtime` owns agent lifecycle
([ADR 0005](../adr/0005-agent-runtime-service.md)).

## Agents

The ACP AI panel uses four layers: `nocterm-ai` declares runtime-neutral agent
contracts and bounded context/tool rules; `nocterm-acp` supplies subprocess and
authenticated local bridge adapters; the `nocterm-agent-runtime` service admits
connections, closes idle ones and coordinates document writes; `nocterm-agent` supplies threads
and chat UI. App injects the adapter. The runtime knows no thread type: a thread
registers a `SessionClient` and applies the `SessionEvent`s the runtime emits,
synchronously and in order, while the runtime reads only a `ClientState`. The
host injects the service-layer `SettingsHandle` into runtime dependencies. Workspace exposes narrow terminal ports
and `ConnectionDirectory` seams so the agent feature never imports other
features. The right panel lives outside the tab dock and remains available with
no tabs; maximize uses the working area while preserving the footer.
`ConnectionDirectory` methods take the workspace as a weak handle and are called
outside workspace updates, because opening a connection updates the workspace.

Saved history starts as a catalogue of at most 200 metadata rows, with transcript
files as the source of truth and rebuildable metadata sidecars. Opening or
editing a row loads its document in the background. A shared loading permit
bounds hydration and archive search, including payloads awaiting GUI delivery;
inactive, clean documents can be evicted under a conservative 64 MiB resident
budget. Collection limits and a structural preflight also bound allocation from
small JSON elements, independently of the 32 MiB file limit.

The client supplies a saved session's directory before connection startup.
Background startup schedules the injected `WorkdirResolver`; its adapter prepares
and validates that directory, and the resulting
root is used for the process, sandbox and resumed or fresh ACP session. Dormant
documents have no maintenance timer. Workspace events revoke execution access;
an execution watchdog runs only while structured commands are active.

Quit freezes normal writers and captures final immutable documents and deletion
tombstones before windows disappear. Serialization, ordered disk writes and
process cleanup then run in the background without consulting the application.
Active and already-closing processes start cleanup concurrently. Persistence and
cleanup share a bounded shutdown deadline; failures and timeouts are logged.

## Agent storage ownership

Runtime coordinates nine private responsibility owners rather than exposing
mutable persistence, admission and connection maps. `AdmissionState` owns its queue/task and a pure `AdmissionPolicy`; its effects become GPUI client events in Runtime. `AgentStateStore` uses
optimistic `Persisted` snapshots. `ChatStore` checks current document identity and
permanent deletion tombstones before admitting independent keys to one
`KeyedPersisted` worker; one chat failure does not stop unrelated chats.

Both snapshot and keyed policies use Store's common normal/captured disk
executor and gate. Ready capacity is 32 key tokens, with one newest retained
intent per dirty document; a full ready queue never drops accepted work. This is
not a 32-document limit or a constant total payload budget: client history/queue
validation owns per-document admission, and repository codecs retain the existing
32 MiB encoded/64 MiB structural bounds. The service retains one newest payload
per dirty key plus its immutable active operation; failed closed documents remain
until retry or capture. The UI's 64 MiB resident archive policy bounds hydrated
archive display, independently of dirty persistence admission. Exact
physical key/owner/revision stamps are recorded before foreground acknowledgement.
Clean closed keys release their payloads; failed dirty keys remain through retry
or final capture. The existing snapshot receipt distinction is preserved: an
already published active prefix completes at its own revision; only a covering
final publication completes an unwritten prefix at the later revision.

The app preloads preferences through injected repositories. Catalog loading stays
lazy and bounded, with one shared hydration permit through GUI delivery.
`PreparedHistory` is an optional bounded construction input, not a mutable
Runtime fixture setter. Normal intake closes before shutdown client events;
a private capture transition checks the current owner and takes final snapshots
while App still exists. Normal session close and final capture share one lazy process-cleanup completion per
command handle. Final capture bypasses a blocked close acknowledgement and joins
cleanup already running; a previously failed normal attempt gets one final retry.
Late normal close joins the same attempt rather than invoking cleanup again.
Favorites, chats and process cleanup start independently
under the host's one `ShutdownCoordinator` budget. Runtime installs no quit hook
or persistence deadline. See [ADR0022](../adr/0022-agent-storage-owners.md).

The runtime's private `UsageLimits` owner coalesces account refreshes per agent and
schedules the injected `AccountUsageSource` on its existing background executor.
Missing reports and failures retain the last accepted limits; stopping the runtime
cancels pending delivery. Native account paths remain in the injected adapter.

Runtime performs no native workspace filesystem or home lookup. Its startup task
checks cancellation before and after preparation and passes one resulting canonical
root to the process, session requests and connected event. Candidate connection
keys/serials remain unchanged when the prepared root is accepted. A late successful
connector still follows the existing retained orphan-cleanup completion.
