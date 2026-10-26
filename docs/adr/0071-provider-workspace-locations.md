# 0071. Workspace locations retain provider identity and session authority

Status: accepted

## Context

A workspace retains a folder, its terminal surface and chat identity. A folder
may belong to an ordinary host or a container. Requiring every saved location to
contain a host and a POSIX path would also make each additional resource provider
a change to the core catalog format.

## Decision

`nocterm-project-model` owns stable workspace identity and a checked location
envelope. A location is an ordinary host folder or a namespaced provider ID,
resource format version and bounded opaque resource reference. Resource references
are inert locators, not scripts, credentials or executable configuration.
Provenance remains separate from execution authority.

`nocterm-project-providers` owns the concrete container resource format and typed
host-backed preparation contracts. Its container identity includes the original
host and jump route, engine, full container ID and folder. Docker and Podman
discovery have independent results; one engine's failure does not hide the other.

`nocterm-project-runtime` implements preparation through an explicitly captured
host executor. It prepares selected tools and Git before catalog publication.
Cancellation and rollback may remove only a resource created by that operation,
after checking its exact engine, ID and ownership label. An already prepared
resource survives a failed catalog write so saving can be retried independently.
Closing a workspace or removing its metadata does not delete its resource.

Application composition supplies provider resolution explicitly. Resolution binds
the complete saved location to a retained terminal provider; opening a terminal
takes the original surface receipt rather than a new resource or folder. Host
authentication, credentials and ordered jump routes retain their existing owners.
Container sessions expose container-scoped execution and do not expose the host
filesystem as though it belonged to the container.

Catalog version 1 migrates to ordinary host folders. Unknown provider IDs and
resource versions retain their bounded references and workspace history while
connection capability is unavailable. They never fall back to an ordinary host.
Malformed catalog envelopes remain errors. The current runtime supports POSIX
folder sessions; the envelope establishes no plugin ABI or universal resource API.

This extends [ADR 0067](0067-project-tabs-with-inner-surfaces.md) and
[ADR 0070](0070-original-workspace-connection-recovery.md). Workspace chat identity
and the existing agent lifecycle remain governed by
[ADR 0069](0069-workspace-chat-launch-boundary.md).

## Alternatives

- A core variant for every engine or external service couples catalog ownership
  to provider-specific dependencies and validation.
- Reusing import provenance as a locator turns descriptive metadata into
  execution authority.
- Requiring every provider to expose a host executor excludes independently
  resolved resources and combines unrelated preparation contracts.
- Reusing a host filesystem after launching a container shell gives file
  operations the wrong authority.

## Consequences

Provider-specific rules and I/O remain outside the core catalog. New integrations
require their own adapter, credential integration and presentation. Unknown
resources remain visible without granting connection capability. Preparation and
catalog publication have explicit independent failure and retry boundaries.
