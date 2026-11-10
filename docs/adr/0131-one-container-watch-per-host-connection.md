# 0131. One container watch per host connection, relisted only for listing changes

Status: accepted.

## Context

The containers panel kept its host current by following `docker events` (or `podman events`)
and listing every container and every image again after each burst of events. Each window
built its own model with its own watch, and the panel is registered in every window
([ADR 0104](0104-a-window-shows-one-environment.md)), so two windows on this computer ran two
event streams and two full listings for every event.

Most events do not change a listing. A container health check is an `exec_create`,
`exec_start` and `exec_die` on every probe. With health checks every few seconds, the desktop
listed every container and image several times a second. Listing images makes the daemon walk
them and compute layer usage, so `dockerd` and `containerd` used about half of a 12-thread
CPU while the containers themselves were idle.

## Decision

- **The domain decides what an event means.** `nocterm_containers::EventReader` reads the
  event stream (Docker's `{{json .}}` template, Podman's `json` format) and returns `Changes`,
  which says whether the container listing, the image listing or both are stale.
  - Commands run inside a container, attached terminals and copies change neither listing.
  - A health status changes the container listing only when it differs from the last status
    heard for that container.
  - Container life events change the container listing; a commit changes both.
  - Image events change the image listing. Tags, untags and removals change both, because a
    container may be shown by its image's name.
  - A line it cannot read changes both.
- **One watch per host connection.** The new service crate `nocterm-containers-service` owns
  `ContainerHosts`, a registry composed once in bootstrap
  (`nocterm_containers_ui::install`). It hands every model asking about the same `HostExec`
  the same `HostContainers` entity. That entity follows one event stream, keeps the engine,
  the snapshot and the listing status, and lists only what `Changes` names, once per 300 ms
  burst.
  - It lives while a model holds it, so the footer count of a hidden panel stays current.
  - When the last model lets go, its programs stop.
- **Actions and the refresh button ask; they do not restart.** `HostContainers::relist`
  passes `Changes` to the running watch instead of starting a new event stream. Only a
  watch that has stopped (no engine on the host) starts again and looks for the engine anew.
- The feature model keeps following the active host and owns the captured action intents. It
  reads engine, snapshot and status from the shared entity.

## Consequences

- An idle host with health checks runs no listing at all. Two windows on one host run one
  event stream and one listing per change.
- A filter on the daemon side (`--filter event=…`) was rejected. An allowlist that misses an
  action leaves the panel stale, and Docker and Podman name their actions differently. Reading
  the event in the client costs one JSON line per event.
- A host whose engine reports no events is still listed every 10 s. A host without an engine
  is asked again only when a window shows it again or the person refreshes.
- Two separate connections to the same machine are two watches: the identity of a watch is
  the executor, the same identity that captured action intents compare.
