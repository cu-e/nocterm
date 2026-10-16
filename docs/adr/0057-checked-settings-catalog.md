# 0057. Checked settings catalogs and explicit settings composition

Status: accepted

## Context

Settings controls belong to their feature owners, but the host still knew five
fixed builtin IDs and a numeric guest offset. A UI compatibility global obscured
the original settings service supplied to those owners and to test fixtures.

## Decision

Settings UI receives an immutable checked ordered catalog. Each registration
contains a page descriptor and an eager or lazy lifecycle. Construction validates
empty IDs, duplicates and the required default before invoking any page factory.
IDs retain their exact spelling. Typed catalog errors cross the feature boundary;
the composition root logs a failed startup catalog and quits before registering
application actions or opening the first window. No fallback catalog is installed.

The app retains the settings installation, ordinary catalog and optional startup
catalog in one SettingsPages owner in the existing Booted slot. Ordinary settings
registers Appearance, Terminal, local shell, SSH and AI as eager pages followed by
lazy Keymap, Monitor, Explorer and optional Vault. The startup catalog contains
the same eager pages followed only by Vault. Each defaults to Appearance; opening
the startup prompt selects Vault without changing an existing singleton's catalog.

The generic host constructs all eager pages in registration order for each Item,
then materializes the selected default if lazy. A valid changed selection flushes
all eager pages, deactivates the previously selected lazy page, constructs an
unseen lazy page and focuses it. Same-ID and unknown-ID opens preserve selection
and do not flush. Close flushes eager pages and closes each constructed page once;
hidden lazy pages are never constructed to close them. Factories share only their
original Item's settings context and feedback handle.

Native startup prepares one SettingsConfig before asynchronous agent hydration.
Bootstrap consumes it once, after hydration, to install the actual SettingsHandle.
UI initialization accepts that handle directly and returns a retained UiReady.
Features register schemas and retain their supplied handle. SettingsStore,
SettingsExt and witness rediscovery are removed, including fixture access.
Existing receipt, typed validation, original-window authority and persistence
semantics stay with their existing owners. Layout's one quit hook still captures
the same coordinator with its 180 ms shared deadline.

## Alternatives

A hardcoded generic host keeps application registration policy in a shared feature.
An unchecked mutable catalog permits duplicates or missing defaults after validation.
A global compatibility facade or an implicit in-memory fallback can redirect an
existing view's policy or create a second settings document. A general service
registry adds unrelated lookup and type-erasure responsibilities to navigation.

## Consequences

Catalog validation is side effect free, while each Settings Item owns independent
page instances. New windows reuse checked registrations and actual installations.
Production and tests construct competing installations explicitly. This supersedes
the compatibility-shell portions of ADR0013 and ADR0053, the fixed builtin host
slots of ADR0056, witness rediscovery described in ADR0007 and the global access
syntax described in ADR0001; their persistence,
authority, page ownership and initialization-order decisions remain in force.
