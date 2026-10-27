# 0096. The hub contract lives outside the workspace and only the hub adapter speaks it

Status: accepted

## Context

A Nocterm hub is a separate product with its own repository. Client and server must agree on
the wire protocol, the signed-request format, the key formats and the client library that
speaks them. Two copies of that contract drift. A path dependency on a sibling checkout makes
this repository unbuildable without it, and a copy of the protocol files in this workspace
would be a second source of truth.

The protocol uses gRPC. `tonic` and `prost` values in domain records, service signatures or
views would tie every layer to protocol releases, the same problem ADR 0058 solved for the
agent protocol.

## Decision

The contract is a public repository, `https://github.com/cu-e/nocterm-contract`. It
publishes `nocterm-hub-client`, `nocterm-hub-grpc`, `nocterm-hub-model`, `nocterm-hub-proto`,
`nocterm-keys` and `nocterm-link`. This workspace depends on it by `git` and a full commit
`rev`, declared once in the root `[workspace.dependencies]`. Moving to a newer contract is a
change of that `rev`, reviewed with the code that needs it.

Only `nocterm-hub-io`, an adapter, depends on those packages, and only that crate depends on
`tonic`, `tonic-*`, `prost` or `prost-*`. It implements the neutral ports of
`nocterm-hubs-model` (`HubTrust`, `HubSignIn`, `HubSessions`) and converts protocol records to
the model's records and protocol failures to `HubError`. The original failure stays the
`source()` of the error; no variant carries text built from it.

`cargo xtask architecture` enforces this through the exact list `EXTERNAL_CONTRACTS`
(`xtask/src/external_contracts.rs`):

- a contract package used by any crate other than its listed consumer is an error;
- a dependency on one must name the contract repository with a 40-hex `rev`; a path, another
  repository, a branch or a tag is an error;
- `Cargo.lock` must resolve every contract package from that repository at a pinned commit,
  which catches a `[patch]` or path override;
- a workspace member may not take the name of a contract package;
- any other `nocterm-*` package without an architecture layer stays an error, so the list is
  the only exception and cannot grow by accident.

The layer rules of our own crates are not relaxed. The contract packages have no layer, so
they never appear in the dependency map.

The device key and tokens are held by the adapter in a private file per hub. The contract
supplies the signing and rotation (`SessionStore` with an inter-process lock); the adapter
supplies the storage, with the same private-publication primitives as the rest of the
application (ADR 0015).

## Alternatives

A path dependency on a sibling checkout breaks clean builds and CI. Vendoring the contract
into `vendor/` ties it to the vendor replay of ADR 0014 and forks the protocol. Letting the
domain or service crates use the generated types makes every protocol release a change in all
layers. A registry release was not chosen because the contract is not published to one yet; a
registry dependency would fit the same checker once it is.

## Consequences

The contract can be built, tested and versioned on its own, and both products pin each other
explicitly. Building this workspace needs network access to the contract repository (or a
vendored Cargo cache). Notices for the contract's dependencies are regenerated with the
lock file. Domain and feature code use neutral hub records only, and the hub adapter is the
single place that changes with the protocol. Testing the adapter against a real hub is a
gated test (`NOCTERM_REQUIRE_HUB`), like the SSH server tests.
