# 0004. Persisted files share one bounded write queue

## Context

Settings, connections and snippets each kept a pending list, a closing flag
and an in-flight counter for shutdown, with slightly different overflow
behaviour.

## Decision

`nocterm_core::persist::WriteQueue` holds that mechanism once, GUI-free and
tested once: a bounded queue of pending writes, rejection when full, and a
drain on shutdown. Each owner keeps its own publish step (`publish_atomic`).

## Alternatives rejected

- The plan's revision tickets and an embedded executor: owners already order
  their writes, and an executor would tie the foundation crate to a runtime.

## Consequences

- Overflow and shutdown semantics are the same for every persisted file.
- Connections' enqueue was split into size checks, rebasing and saving.
