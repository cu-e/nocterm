# 0046. Shared causes expose their original concrete errors

## Context

Cloneable diagnostics retained native errors in Arc fields. A derived source
for an Arc error could delegate directly to that error's own source and skip the
immediate concrete error. A native leaf then disappeared from standard source
traversal, even when destructuring the diagnostic still allowed a downcast.
The image codec already used a correct immutable holder for this boundary.

## Decision

Foundation's SharedError<E> owns one private Arc<E>. Manual Clone shares the
allocation without an E: Clone bound. Display and Debug delegate to E; typed
immutable access preserves E's classification. SharedCause is its erased alias
with immutable concrete downcasts. Owned and already-shared constructors retain
one payload without strings, serialization, mutation or Deref access.

Error::source returns the original error as the next frame, even for a leaf.
The original error then supplies its own deeper causes. Separate sized and erased
Error implementations in the same owner module support Rust's coercion rules;
they do not create separate holders or diagnostic payloads.

Store Edit and Backend use SharedCause with unchanged constructors, admission,
semantic checked-edit conversion and publication behavior. AI ImageCodecCause
is a public alias of SharedCause, retaining its name, constructor and downcasts.
Other source boundaries, including Store Joined, retain their representations.
The standard source chain remains linear; this holder does not define multi-error
selection or change diagnostic ownership in another boundary.

## Alternatives rejected

- Delegating to the original error's source skips its concrete classification.
- Copying a local holder at each layer duplicates a foundation responsibility.
- Requiring Clone on E excludes native errors and duplicates diagnostic payloads.
- Caching Display text loses typed classification and deeper native causes.
- Exposing mutable state or Deref adds capabilities unrelated to diagnosis.

## Consequences

A holder contributes one explicit source frame before the original error. Standard
source traversal and direct downcasts both retain the original classification;
clones observe the same allocation. Existing diagnostic messages, persistence
ordering, shutdown behavior, image limits and serialized formats are unchanged.
The Rust variant field types change for Store Edit and Backend; callers retain
immutable downcast access. No executor, protocol type or additional dependency
is introduced in the foundation layer.
