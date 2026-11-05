# 0138. Monitor consumers share collection by captured executor

Status: accepted

Refines [ADR0077](0077-selected-workspace-monitoring.md). Selected-surface authority and
per-surface sampling remain unchanged.

## Context

Two windows observing the same actual executor previously started independent resource
scripts. Opening details or switching surfaces changed each script independently, even when
all consumers could use one raw reading. Sharing presentation histories would merge unrelated
surfaces and could confuse executor identity with a descriptive host name.

## Decision

The viewless service `nocterm-monitor-service` owns `MonitorWatches` and `MonitorWatch`.
The installation injects one registry. It groups requests by actual captured `HostExec`
identity and host/container collection scope. A weak executor identity prevents reuse of an
allocation from matching an old resource. Display names never establish identity or authority.

Each consumer subscribes with its metric set and cadence. One collection uses the union of
metrics and the fastest requested cadence. A changed aggregate plan cancels the old generation
and starts its replacement. Late results cannot advance the new generation. Removing the last
subscriber cancels native collection; the registry holds weak watch references.

The service retains one latest raw reading and typed status. `nocterm-monitor-ui` still owns
selected-surface checks, bounded histories, samplers, details state and formatting. It applies
a reading once per service sequence. Platform hints are reused only for their original
executor. A missing selected workspace capability never falls back to another resource.

## Alternatives rejected

- One script per window: it duplicates native work for the same resource.
- Share whole monitor models: it mixes surface-specific histories and UI lifetimes.
- Share by hostname or container name: those names do not identify captured authority.
- Put the registry in a feature global: the registry has background work and no views, and
  lifecycle belongs to the explicitly retained installation.

## Consequences

A service crate supplies a fitting owner without making features depend on each other.
Details in one window can increase collection frequency for another consumer, while that
consumer keeps its own bounded presentation history. Native scripts and wire parsing remain
in the existing monitor domain and session execution ports.
