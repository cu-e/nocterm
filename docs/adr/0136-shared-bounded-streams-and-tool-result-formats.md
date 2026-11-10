# 0136. Bounded byte collection and tool result formats have shared owners

Status: accepted

## Context

Several adapters collect already-admitted byte streams with the same limits,
while preserving different execution, closing and publication policies. Terminal
tools also produce the same display payloads in desktop and headless execution,
and their presentation consumers need the same schema. Settings pages repeat
binding typed sections to the existing field lifecycle.

## Decision

Foundation `nocterm-core::bounded_io` owns bounded in-memory byte sinks and
counting writers. Limits are supplied by consumers. Codec versions, model budgets,
structural allocation accounting and typed consumer errors remain with their
existing owners. A refused write does not change the accepted prefix.

Domain `nocterm-session` owns bounded collection from its existing execution and
remote-download streams. Strict execution collection reports overflow explicitly;
the existing prefix collection contract remains available. Download collection
validates chunk size and the total limit, but does not close the stream. Adapters
keep their distinct close-error policies, deadlines, cancellation and captured
filesystem or execution authority. No native I/O implementation moves into domain.

Domain `nocterm-ai` owns neutral terminal-tool result formats shared by desktop,
headless and presentation consumers. Result values describe observations; they
never authorize execution. Redaction, historical fallback parsing and presentation
labels retain their existing owners.

Shared UI binds typed settings sections through the existing text-field lifecycle.
Pages still own validators, labels and section policy. Shared field construction
neither introduces another settings document nor changes publication or teardown.

## Alternatives

Independent copies retain the risk of different bounds and drifting result keys.
A universal executor, filesystem walker or settings-page framework would absorb
consumer policy and require broader authority than these reusable mechanisms need.
Replacing structural memory accounting with serialized JSON length would lose its
separate retained-allocation contract.

## Consequences

Consumers share mechanics and formats without sharing execution authority or
lifecycle. Behavior checks must retain exact-limit, over-limit, cancellation,
closing and historical-result scenarios. Changes to the shared result format are
reviewed together with both producers and presentation consumers.
