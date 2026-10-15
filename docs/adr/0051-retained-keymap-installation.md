# 0051. Keymap pages and directory actions retain their supplied installation

## Context

Keymap edits discovered a global owner while new settings pages and windows could
outlive their original composition. The keymap page exposed mutable presentation
fields, and application directory actions discovered paths and logging settings
through globals. Native keymap save failure intentionally leaves desired shortcuts
installed, unlike the durable settings publication contract.

## Decision

The UI-layer keymap boundary returns a cloneable KeymapHandle around one private
four-field entity. Bootstrap retains it and every lazy page factory and new window
receives that same handle. Named immutable queries and edit operations replace
ambient acquisition. A page retains its native local search/recording controls and
a weak owner observation subscription; closing a page releases its subscription
and interception without disposing the application owner.

Edits validate before mutation. The owner applies the existing desired user change
and synchronous native save, retains the original typed persistence cause through
SharedError, builds owned marked bindings, releases its entity borrow, and replaces
only its existing metadata classes in GPUI. Unrelated toolkit bindings survive;
defaults precede user entries and explicit NoAction unbindings remain installed.
Notification follows installation even when save fails. The next successful edit
persists the complete current desired keymap. Invalid input neither installs nor
clears an earlier diagnostic. This preserves the original concrete PersistError
and native source for the caller and every observing page
([ADR0046](0046-shared-concrete-error-causes.md)).

The root captures config and fallback state/logs paths when registering directory
actions. LogsDirectory reads the supplied SettingsHandle at dispatch time, retaining
published settings semantics, including Some(empty). A private two-action helper
accepts the existing native open operation; tests can record real action outputs
without changing native directory creation/opening or invoking unsupported test
platform program launching.

This refines the ambient acquisition consequences of [ADR0001](0001-settings-sections.md) and
[ADR0007](0007-bootstrap.md) without
changing typed section ownership or the common application/test bootstrap. Accepted
records remain historical.

## Consequences

Current, surviving and new windows share one actual keymap; deliberately separate
handles do not redirect each other's state. Stateful view fields remain private.
No global, registry, background worker or quit participant is introduced. Existing
native keymap loading and synchronous persistence remain explicit UI-layer migration
debt; this checkpoint does not move native policy into a domain crate or claim a
completed persistence service extraction. File grammar, toolkit marks, action names,
settings schemas and native directory lifecycle remain unchanged.
