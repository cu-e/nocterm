# 0054. Native settings text owns checked publication receipts

## Context

Settings pages retain their original installation as defined by ADR 0053. Its text
algorithm recorded saved text before detached physical publication and ignored an
ordered validation rejection. A failed write had no field diagnostic and the same
literal could be skipped on retry. Moving that algorithm into a shared owner would
preserve those faults. Receipt-aware native text is reusable UI state; section
validators and page policy remain with their feature owners.

## Decision

Shared UI SettingsTextField exposes immutable presentation and named flush/lifetime
operations over a private TextField entity. Bindings provide typed document read/write
callbacks without mutable input/entity escape. Feature bindings clone the actual typed
section, preserve its original validation error through StoreError::Edit, and insert
only a successful section. Immediate feedback and ordered checked replay use the same
callback; the existing settings service atomically rejects invalid candidates.

Each distinct flush synchronously attempts checked admission to the existing
bounded queue, including while older work is pending. A waiting identical
literal is deduplicated only while its recorded admission semantic revision
still matches the published service revision. A subsequent publication can
precede UI receipt delivery; Enter then admits a fresh ordered intention, even
for the same literal. A harmless semantic no-op remains handled by the common
service. Settled dedup uses the current confirmed value, never an old submitted-
text cache. Returning to a prior baseline behind another candidate is a new
intention. Full, Closing and publication failures remain typed and retryable.
The field retains one latest token with generation, admission revision, literal
and waiter and one timer, never another queue or document cache. Dropping an
older waiter cannot cancel accepted callbacks in the service.

Only matching original completion may reconcile canonical native text or clear its
diagnostic. A newer draft/error survives older completion. Native changes clear only
obsolete local diagnostics; programmatic canonical updates do not feed back into
writes. Published observation and receipt completion capture the original weak owner
and window. Parent forms retain a narrow redraw subscription with each field's
membership, because a failed write need not publish settings. Dynamic removal retires
both handle and parent observation without a strong subscription cycle.

Hosts synchronously flush the latest eligible literal on close/deactivation before
releasing UI ownership. Explicitly admitted work survives view loss and shutdown
capture in the common service. An unflushed draft is outside the accepted snapshot
guarantee; no coordinator pre-capture phase, registry or additional quit hook is added.

## Consequences

Settings-ui keeps its actual validators, field order/IDs, labels and guest lifetimes.
UI owns native draft/receipt presentation without importing feature types. Concrete
semantic/backend causes remain typed until rendering. The original 600 ms timing,
Enter/blur behavior, queue capacity 16 and common shutdown lifecycle remain. This
supersedes only the text algorithm described in ADR 0053; its retained installation
and original dependency authority remain unchanged.
