# Service layer

Part of the [architecture overview](../ARCHITECTURE.md).

Service crates own GPUI entities and background work but no views: they
depend on GPUI alone, never on the component library, the UI layer or features,
and features depend on them. `nocterm-agent-runtime` is the one service crate
([ADR 0005](../adr/0005-agent-runtime-service.md)).

## Agents

The ACP AI panel uses four layers: `nocterm-ai` declares runtime-neutral agent
contracts and bounded context/tool rules; `nocterm-acp` supplies subprocess and
authenticated local bridge adapters; the `nocterm-agent-runtime` service admits
connections, closes idle ones and writes chats; `nocterm-agent` supplies threads
and chat UI. App injects the adapter. The runtime knows no thread type: a thread
registers a `SessionClient` and applies the `SessionEvent`s the runtime emits,
synchronously and in order, while the runtime reads only a `ClientState`. The
host installs the `AiSettingsSource` the runtime reads settings through. Workspace exposes allowlisted `TerminalAccess`
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
Background startup resolves and validates that directory once, and the resulting
root is used for the process, sandbox and resumed or fresh ACP session. Dormant
documents have no maintenance timer. Workspace events revoke execution access;
an execution watchdog runs only while structured commands are active.

Quit freezes normal writers and captures final immutable documents and deletion
tombstones before windows disappear. Serialization, ordered disk writes and
process cleanup then run in the background without consulting the application.
Active and already-closing processes start cleanup concurrently. Persistence and
cleanup share a bounded shutdown deadline; failures and timeouts are logged.
