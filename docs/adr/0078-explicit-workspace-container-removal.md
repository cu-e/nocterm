# 0078. Explicit workspace container actions precede checked metadata removal

Status: accepted

Supersedes the unconditional metadata-only removal statements in
[0067](0067-project-tabs-with-inner-surfaces.md) and
[0071](0071-provider-workspace-locations.md) only for an explicit user-selected
container action. Closing a workspace and automatic preparation rollback retain
their existing rules.

## Context

Removing a saved Docker/Podman workspace sometimes also needs to stop or delete
its container. Imported provenance or a resource created in the past does not
express current permission. The resource and catalog cannot be updated atomically,
and a container-scoped executor cannot operate on the original host engine.

## Decision

The provider domain exposes the narrow `WorkspaceContainerActions` contract,
with Stop and Delete actions and a checked saved location paired with its original
unscoped host capability. Keep is metadata-only and makes no provider call.
Source capture validates the full host locator and ordered jump route; feature
submission checks the original terminal epoch and catalog binding. Selection or
focus changes do not resolve a different executor, and disconnected remotes never
fall back to local execution. Composition injects the adapter explicitly.

The runtime inspects the exact engine and full container ID. Already-stopped Stop
is idempotent. Delete stops a running container and removes that exact ID without
force, volumes, image deletion, pruning, host path deletion or name fallback.
Creation ownership labels are unrelated to this explicit authority. Missing or
mismatched identity refuses the action, and failures retain typed causes and known
partial effects. Timeouts cannot establish that a daemon operation did not happen.

The service owns bounded retained jobs, serializes pending actions by original
host/route, engine and full ID independently of project directory and profile alias,
and joins existing shutdown capture. A resource action must acknowledge success
before durable catalog removal. Original definition and binding are checked before
admission and at publication, including queued catalog application. Failed actions
retain the record and claims; failed publication retains acknowledged effects.
Retry repeats only catalog removal. No rollback recreates a stopped/deleted resource.
Dropped view receivers do not discard admitted work; shutdown freezes admission,
cancels native work and prevents late publication.

The native confirmation defaults to Keep, offers compact mutually exclusive
choices and emphasizes Delete as destructive. Consequences explain writable-layer
loss and effects on other workspaces. Images, volumes, bind-mounted host folders
and chat history remain. Busy blocks duplicate submit and dismissal; inline errors
show completed effects and a metadata-only retry. Direct and unknown locations
retain metadata-only behavior, including while offline.

## Consequences

Explicitly attached containers can be stopped or deleted with current user
consent. External clients and other saved workspaces using that same container
are affected. Existing open surfaces remain owned by their normal lifecycle;
stopping/deleting the container naturally disconnects its terminals. No generic
provider lifecycle ABI, persisted permission, automatic resource cleanup, extra
SSH reconnect path or global service lookup is introduced.
