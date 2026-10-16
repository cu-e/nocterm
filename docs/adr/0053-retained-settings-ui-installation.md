# 0053. Settings UI retains its supplied settings installation

## Context

The composition root already retains a settings service, but builtin settings pages,
text fields and diagnostics discovered its compatibility global. Deferred appearance
commands preferred that global over the supplied theme installation. Replacing a
compatibility owner could redirect existing page writes and callbacks.

## Decision

The feature returns Installed with two private immutable collaborators: the actual
SettingsHandle and prepared Deps containing SandboxProbe. Bootstrap creates it from
its already installed settings and retains it in the existing Booted settings slot.
Current/new windows and ordinary/deep-linked settings actions clone that installation.
The host, builtin page controls and fields read/edit/observe that exact owner. Native
probe work remains in the injected capability. Section owners register their schemas
before pages read them; installation does not provide a global or schema fallback.

A page retains its settings observation subscription with a weak view and original
window. Existing published-semantic-revision filtering and lazy guest page lifecycle
remain. Appearance commands and asynchronous theme selection changes capture their
original supplied settings, including deferred opening and native service completion;
no try-global-first redirect is available.

The existing text field algorithm remains: 600 ms debounce, Enter/blur and host lifecycle
flush; probe validation against current published document; saved text recorded before
the detached unchecked ordered replay. The service receipt is not a claim that the
field observed a physical result. This decision changes dependency authority without
adding checked edits or receipt-aware field completion semantics. Existing validators,
page identities/order, section ownership and typed persistence errors remain.

## Consequences

Multiple settings installations stay distinct; existing windows and newly opened
pages reuse their supplied original owner. The root gains no slot and Services stays
prepared inputs. No settings cache, persistence worker, global, quit handler, broad
workspace port or native fallback is introduced. SettingsStore compatibility remains
for other consumers; this decision does not remove its implementation. Field public
presentation state and builtin page ownership keep their existing boundaries.
