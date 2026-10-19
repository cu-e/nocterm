# 0058. Agent protocol types stop at their adapters, and history is neutral

Status: accepted

## Context

Agent Client Protocol SDK values reached the AI domain, the runtime service, the
panel and the saved version-one transcript. Protocol upgrades therefore changed
domain records, service signatures and physical history together. Earlier records
deferred this step: ADR0010 noted that history still stored protocol types, and
ADR0022, ADR0037, ADR0040 and ADR0052 kept physical history version one and
protocol-backed. Decoding agent messages and saved transcripts also allocated
outside the budgets that bound a document's resident memory.

## Decision

`nocterm-ai` declares neutral records for agent identity, session events, tools,
configuration and history. Only `nocterm-acp`, which speaks the live protocol, and
`nocterm-agent-storage`, whose private grammar reads historical transcripts,
depend on the protocol SDK crates. `cargo xtask architecture` rejects that
dependency of any kind, including development dependencies, in every other package.

The runtime connects through `AgentConnector::connect_managed`. The adapter maps
protocol messages to neutral values and returns a managed connection whose command
handle, event pump and cleanup the runtime owns; an unconfirmed cleanup is a typed
error rather than text.

AI owns the version-two transcript codecs, catalog rules and the `ChatRepository`
operations `load_managed`, `catalog`, `save_owned` and `delete`. The storage adapter
reads version-one files through its private grammar. Publication and archived-draft
merges write version two. Only a chat loaded from version one whose version-two
encoding exceeds the file or structural limit is written back in that grammar,
within the same limits. Clean files, and merges that change nothing, are not
rewritten. Fixtures reach historical decoders only through the adapter's
`test-support` feature.

Native decoding, reading and conversion charge a document's retention ledger before
allocating, and refusals are typed values. Failure payloads on those paths stay
inline under reasoned `clippy::result_large_err` or `clippy::large_enum_variant`
expectations, because boxing them would allocate outside the ledger.

## Alternatives

Wrapping protocol types in a versioned domain envelope still couples saved history
and domain rules to SDK releases. Converting in the runtime would make a service
depend on adapter protocol types. Boxing large errors, or decoding with unbounded
serde values, allocates memory that no document budget accounts for.

## Consequences

Protocol upgrades change only the ACP adapter and, for saved data, the storage
adapter's historical grammar. Runtime, panel and tests construct neutral records.
Managed native path canonicalization is Unix-only; other platforms refuse with a
typed platform error. This supersedes the version-one codec placement in ADR0022
and the statements in ADR0010, ADR0037, ADR0040 and ADR0052 that saved history
remains protocol-backed version one; their ownership, revision and installation
decisions remain in force.
