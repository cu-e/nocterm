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
