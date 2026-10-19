# 0058. Agent protocol types stop at the ACP adapter

Status: accepted

## Context

Agent Client Protocol SDK values reached the AI domain, the runtime service, the
panel and the saved version-one transcript. Protocol upgrades therefore changed
domain records, service signatures and, through serialization, physical history
together. Earlier records deferred this step: ADR0010 noted that history still
stored protocol types, and ADR0022, ADR0037, ADR0040 and ADR0052 kept physical
history version one and protocol-backed.

## Decision

`nocterm-ai` declares neutral records for agent identity, session events, prompts,
tools, configuration, permissions and usage (`nocterm_ai::model`). Only
`nocterm-acp`, which speaks the live protocol, depends on the protocol SDK crates;
its `mapping` module is the single place that converts SDK values to neutral
records and back. `cargo xtask architecture` rejects that dependency of any kind,
including development and renamed dependencies, in every other package.

The agent command contract (`AgentCommands`), events (`AgentEvent`), errors
(`AgentError::Rpc(RemoteError)`, `Unsupported(ModelError)`) and the terminal
bridge registration (`BridgeRegistration::endpoint`) use only neutral records.
Unknown SDK notifications are dropped by the adapter, a permission request for
another session is answered cancelled, and a remote error displays its message, or
its code when the message is empty.

Saved transcripts stay at history version one. The persisted neutral records
(`Content`, `ToolRecord` and their parts) serialize byte-identically to the
version-one JSON — same tags, camelCase names, `_meta` keys and field order — and
decode with the same leniency (unknown tool kinds become `other`, invalid
annotations and metadata are dropped, invalid tool-content items are skipped).
The codec stays in `nocterm_ai::history`, no file is rewritten, and
`nocterm-agent-storage` needs no SDK. Golden fixtures pin the files, and the
adapter's equivalence tests pin the SDK-to-neutral correspondence.

## Alternatives

Wrapping protocol types in a versioned domain envelope still couples saved history
and domain rules to SDK releases. Converting in the runtime would make a service
depend on adapter protocol types. Introducing a second history version with a
separate grammar for old files would add a migration path and a fallback encoder
for no change a user can observe.

## Consequences

Protocol upgrades change only the ACP adapter's mapping. Runtime, panel and tests
construct neutral records. Neutral records mirror the part of the protocol Nocterm
uses; fields it never reads are not carried. This partially supersedes the
statements in ADR0010, ADR0022, ADR0037, ADR0040 and ADR0052 that saved history
remains protocol-backed; their ownership, revision and installation decisions
remain in force.
