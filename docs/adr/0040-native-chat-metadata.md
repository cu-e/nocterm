# 0040. Native chat identity and time are supplied through explicit host capabilities

## Context

The AI domain allocated UUIDs in SavedChat's convenience constructor and acquired
wall-clock timestamps inside its transcript reducer. Snapshot framing already
accepted existing identity and time, but genuine allocation and row creation
still performed hidden native operations. Moving those calls to a view helper
would leave the native boundary implicit; reading Runtime through a global would
introduce another hidden collaborator.

## Decision

AI defines ChatIdentity and the narrow ChatMetadataSource contract for allocating
one new identity with its creation time and observing Unix epoch seconds. Its
APIs remain infallible, preserving the existing UUID allocation semantics and
zero fallback for pre-epoch time. Agent-storage implements these native facts
with NativeChatMetadata. This capability owns no files, worker, scheduling or
shutdown lifecycle.

Application composition creates the source and retains it in Booted before
consuming AgentServices. Every window registration receives that same source.
The feature binds it to the originating weak Workspace in its private immutable
ChatHost dependency value. Panel and Thread replace their existing workspace
collaborator with this binding, so their field counts do not grow. ChatHost
provides only the originating workspace, timestamps and new-chat allocation; it
is neither a mutable registry nor a state-owner extraction.

Feature coordinators obtain new identities for genuine creation and forks and
supply scalar times to accepted durable transitions. ChatDocument, budget and
snapshot framing never call a native source. ThreadState receives the same
coordinator timestamp for row creation; aggregated chunks retain the first row's
time. Restore and restart retain existing identity and timestamps. Relative-time
presentation may query the source without changing durable state.

AI removes its native new-chat convenience and time module. UUID parsing remains
pure; its dependency declaration requests no entropy feature. Shared test support
provides an explicitly retained deterministic source, including across windows.
No global lookup, quit handler, additional parent field or new writer is added.

## Alternatives rejected

- Native clock and UUID calls in domain constructors or reducers hide effects.
- Feature-local native helpers bypass the injected adapter boundary.
- Runtime global lookup adds a dependency unrelated to runtime's scheduling.
- Adding a field to oversized Panel or Thread grows existing architecture debt.
- Timestamping framing, capture or import fabricates durable mutations.

## Consequences

This supersedes the retained native UUID/clock debt described in
[ADR0037](0037-chat-document-semantic-revisions.md), without changing its semantic
revision, admission, capture, retry or publication rules. Existing epoch seconds,
zero-as-unknown values and wall-clock rollback behavior remain intact.

The repository Rust APIs now require supplied identity/time and constructor
collaborators. Physical chat history remains version one and ACP-backed; its
serializer, limits, catalog and compatibility behavior are unchanged. Protocol
neutrality and version-two history remain separate decisions.
