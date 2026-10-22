# Service layer

Part of the [architecture overview](../ARCHITECTURE.md).

Service crates own GPUI entities and background work but no views: they
depend on GPUI alone, never on the component library, the UI layer or features,
and UI/feature crates depend on them. `nocterm-store` owns ordered persistence
and synchronous quit capture for layout, settings, snippets, connection state and agent preferences/documents ([ADR 0013](../adr/0013-persistence-shutdown-service.md)).
`nocterm-agent-runtime` owns agent lifecycle
([ADR 0005](../adr/0005-agent-runtime-service.md)); `nocterm-agent-directory` owns
`AgentsHandle`, the one resolution of the injected agent catalog and the `ai`
settings that the runtime, panel and settings page read; `nocterm-snippets-store` owns
checked snippet publication over the shared persistence driver.
`nocterm-workspace-ports` declares the terminal contracts, `SessionContext` and
`ConnectionSummary` with GPUI core alone; the workspace re-exports them and keeps
`ConnectionDirectory`, which opens connections into a workspace
([ADR 0062](../adr/0062-terminal-contracts-below-the-workspace.md)).
`nocterm-agent-chat` owns agent chats without views: `AgentThread`, the
`ChatRoster` of a window's selected and retained context chats and their lifecycle,
and the `ChatWorkspace` and
`SignInTerminal` ports the window shell implements
([ADR 0063](../adr/0063-agent-chats-below-the-panel.md)).

`nocterm-project-service` owns the durable workspace folder catalog and the
open-window registry used to focus an already opened workspace. It uses
`Persisted` and the shared shutdown coordinator. Catalog bindings identify the
original committed definition so an old callback cannot open a changed folder
([ADR 0067](../adr/0067-project-tabs-with-inner-surfaces.md)).

## Agents

The ACP AI panel uses four layers: `nocterm-ai` declares runtime-neutral agent
contracts, neutral protocol records and bounded context/tool rules; `nocterm-acp` maps the
protocol SDK to them and supplies subprocess and
authenticated local bridge adapters; the `nocterm-agent-runtime` service admits
connections, closes idle ones and coordinates document writes; `nocterm-agent-chat` supplies
threads and `nocterm-agent` the chat UI. App injects the adapter. The runtime knows no thread type: a thread
registers a `SessionClient` and applies the `SessionEvent`s the runtime emits,
synchronously and in order, while the runtime reads only a `ClientState`. The
host injects the service-layer `SettingsHandle` into runtime dependencies. Workspace re-exports the narrow terminal ports
of `nocterm-workspace-ports` and exposes `ConnectionDirectory` seams so the agent feature never imports other
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

Workspace chat scope selects document ownership, history and the original tool
seat. All scopes use the existing local ACP connector, workdir preparation,
terminal sign-in and configured sandbox settings. A workspace scope does not
install another agent process or change provider authentication
([ADR 0069](../adr/0069-workspace-chat-launch-boundary.md)).

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

The existing `save_chat` method returns typed acceptance: false for stale ownership,
true only after the keyed writer owns the immutable candidate. Revision-bearing
`SaveAdmitted`, `Saved` and `SaveFailed` events distinguish intake from physical
publication. Registry stamps are committed after intake succeeds. Capture retains
writer-owned intents when a client has nothing pending; shutdown returns captured
admission errors together with drained publication failures. Aggregation retains
the original primary StoreError in its standard source chain; the secondary
diagnostic stays separately inspectable, and both messages remain displayed. Normal detach leaves a
client registered when admission fails so its desired candidate can still be retried.
An older revision from the same current owner cannot replace a newer admitted intent.
Successful detach admission acknowledges its original client outside the Runtime
borrow even after unregistering it; candidate revision validation remains feature-owned.
AI disable finalizes session transitions before taking that durable capture.

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
Cleanup receipts retain the original typed AgentError through foundation SharedError,
including any native source already retained by its variant.
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

## Connection publication

`nocterm-connections-store` owns checked profile admission, ordered profile/recent
publication and coupled shutdown capture. The feature re-exports its actual entity,
so installed views and credential callbacks retain identical ownership. Writers and
a `ConnectionClock` are explicit dependencies; editor inputs, native acquisitions,
settings storage and views stay outside this service.

Profiles use durable checked edits, recents use optimistic coalescing, and both
reuse `Persisted` and `ShutdownCoordinator` with the common 180-ms budget.
Each candidate applies pure authority transitions before the next candidate.
Incarnation, exact endpoint and manual icon/country stamps therefore retain
delete/re-add and A→B→A history even when one final capture serializes identical
profiles. Stamps and derived effects are internal; writers receive only `Profiles`.

The synchronous success hook applies derived recents before completing receipts
and publishes a supplied immutable projection into a distinct `ProfilePublications`
entity. It never rereads leased profile persistence or starts observation jobs.
Failed, rejected and unadmitted mutations cannot publish authority. Captured
background drain has no App/entity access; successful physical prefix/final
snapshots alone select recents effects through the existing two-event feed.
See [ADR0047](../adr/0047-connection-publication-authority-service.md).

## Raw connection facts

`FactsService` in `nocterm-connections-store` owns optimistic persisted Known data,
immutable raw flag state and bounded native job reservations. The supplied background
executor runs only native ports/data. Foreground tickets read the distinct committed
profile carrier, narrow enabled/published-revision policy and captured original source
at every admission and completion. Endpoint/incarnation/manual-override stamps retain
intermediate history; original Item/session authority cannot renew through another
window, equal filesystem Arc or a profile-only flag request. A newer original epoch
may admit its own job without releasing or clearing obsolete physical jobs.

All windows share 32 physical slots. Atomic paired OS/location reservation precedes
capability decisions; stale work retains slots until actual completion. Rotating weak
source cursors reproject on capacity delivery, preserving pending paired reservations
and alternating raw flag/probe lanes. There is no native cancellation guarantee,
production pool, hidden runtime/global lookup or waiting filesystem request queue.

Raw CountryCode flag requests are independent of live sessions and detection policy:
persisted startup keys, manual saved choices and editor drafts keep their previous
behavior. DNS/GeoIP and OS require an original connected matching session. A terminal
flag negative result is cached; image decoding/presentation stays in the feature.
Known capture reuses Persisted/ShutdownCoordinator; closing blocks admissions and
late completions from mutating captured data. See [ADR0048](../adr/0048-guarded-connection-facts.md).

Immutable FactWorkload queries expose physical work and retained original-source
metadata. The logical ledger prunes original Item/epoch/session lifetime separately
from full profile/policy validity; retaining an FS cannot retain closed Item metadata,
and changing association/policy cannot renew a live capability's OS once marker.
Feature ItemsChanged schedules App-deferred reprojection outside the Workspace
subscriber lease; original receipt validity checks are still synchronous.

Connections read-only refusals retain the original loaded diagnostic allocation
through foundation `SharedError`; standard sources reach the concrete storage
error and its native cause. Publication rejects edits without replacing diagnostic
identity, profile authority or the malformed source file.

Agent runtime installation exposes immutable scope-bound `RuntimeHandle` views
of one private Runtime entity. History readers capture the original context before
async work, while favorites, settings and usage remain installation services.
Catalog snapshots retain their scope and load generation; stale results cannot
replace newer ones. The catalog owner reserves at most32 live metadata payloads,
including pending requests and caller-retained old snapshot generations. Unretained
completed entries can be evicted, and capacity refusal leaves the current snapshot
untouched before I/O. The one shared HistoryGate is independent of that metadata
budget ([ADR0066](../adr/0066-scoped-chat-storage.md)).

Archive rows retain an index into their original accounted CatalogSnapshot instead
of copying a complete summary per roster. Prepared General records transfer one
at a time through the original document claim. A single installation history
admission counts live histories across contexts/windows, remaining prepared bodies
and distinct immutable active/latest writer snapshots. Shared subparts may be
conservatively counted more than once. Shown chats remain protected and each chat
owns its durability/lease/error eviction gates. Capacity refuses before hydration
mutates its requester. New live output retains existing per-chat limits; the
admission budget does not claim a hard total-heap limit. Store's borrowed
retained-values observer supplies ownership facts without a second writer map.

ChatRoster retains original context collections, releases quiet hidden archive
entities and drops empty inactive collections. A committed outer surface selects
one projection; retained threads keep immutable original dependencies. Accepted
archived forks complete in their original collection while another context is
visible. Global AI disable captures and releases all retained collections.

Chat factories reserve `(ContextId, chat UUID)` before constructing an entity.
Noncloneable claims bind the original registry, window and incarnation; dropping a
pending claim rolls back only that reservation. Duplicate opens return the original
window, deletes seal only their scoped key, and restart requires the exact previous
owner. Immutable ChatDocument identity, writer keys and completion checks retain the
same scope. Imported General documents retain their original transcript format
through edits and restart; new chats and forks use v2. Catalog rows declare a
typed transcript version separately from the rebuildable cache schema. Activation
and bridge registration check the original scoped chat owner. Every scope then
uses the same existing local connector and preparation; terminal tools stay attached to their original workspace membership.
Runtime registers AiSettings on its supplied settings handle before its first
registry read. Settings observation uses a weak runtime; the feature installation
owns that subscription. Shutdown capture returns existing App-free work, and no
runtime global selects a deferred operation's recipient. Service signatures use
only neutral records ([ADR0052](../adr/0052-retained-agent-installation.md),
[ADR0066](../adr/0066-scoped-chat-storage.md)).

SettingsHandle supplies additive edit_checked and update_checked operations over the
same Persisted worker. Checked callbacks run at the actual ordered document base;
an error discards all candidate mutations before semantic revision bookkeeping or
physical write. Typed section edits register/read their section, apply the callback
to a clone and insert its sanitized value only on success. Memory edits publish
synchronously; durable receipts retain the existing queue/backend timing and return
semantic settings revisions, including unchanged revisions for successful no-ops.
The original infallible edits and their consumers retain their behavior. Dev-only
store test-support supplies the shared first-write gate and snapshot recording used
by service and UI persistence fixtures.

## Snippet library publication

`nocterm-snippets-store` owns the viewless library and exposes `SnippetsHandle`.
Prepared acquisition retains missing-versus-malformed distinctions and validates the
whole library; malformed or invalid input becomes an empty read-only projection with
its original shared diagnostic. The service admits checked record save/delete against
the shared Persisted driver: durable queue32 and immediate memory publication retain
expected-record conflict and final capture semantics. It has no native I/O, UUID
generation, view, settings policy or shutdown hook. Composition supplies its snapshot
writer and registers the same participant once. See
[ADR0055](../adr/0055-snippet-storage-boundaries.md).
