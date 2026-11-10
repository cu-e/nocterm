# 0079. Project terminal contracts live below the shell

Status: accepted

## Context

Project terminal providers and consumers shared contracts in the window shell.
Their signatures exposed ProjectSurface and ItemHandle, coupling feature-to-feature
requests to native presentation and preventing service consumers from naming them.

## Decision

`nocterm-workspace-ports` owns ProjectTerminalOpen, ProjectTerminalResolve,
ProjectTerminalSession, ProjectTerminalTarget, ContextSelectionError and
ProjectConnectionError. Consumers import these names directly; workspace provides
no compatibility reexports for them. The ports use GPUI core and checked project
identity, preserving typed original causes through nocterm-core SharedCause.

A terminal source supplies its original AnyView, ExpectedSession and advertised
TerminalCapabilities explicitly. Terminal checks both the supplied capabilities
and the actual parent view's identity, connection epoch and ordinary connected
session before retaining launch configuration. Descriptive metadata grants no
replacement-session authority.

The target retains one captured GPUI entity with a type-bound validator. Workspace
privately owns ProjectTerminalPlacement: the original ProjectSurface, selection
receipt, optional local destination and initial-request flag. Native membership,
selection, beside-anchor checks and initial hidden completion remain shell-owned.
The typed shell bridge validates its concrete placement before providers allocate
an Item and revalidates before registration. Its genuine stale placements close
only new unowned Items; registered and foreign-owned Items remain open.

A foreign erased destination refuses with StaleSurface and no publication or close.
It grants no membership information; a caller passing an already-created Item
retains cleanup responsibility. Ordinary providers reject such a destination before
allocating terminal resources through validate_project_terminal_target.

Projects retains its direct Workspace dependency for outer ProjectView admission,
Panel and action registration, focused-source capture, original General Item
enumeration and connection-directory projection. A subscription/opening host port
would need to reproduce this actual shell integration. Engine, Image and Container
remain domain DTO dependencies.

## Alternatives

Importing workspace or its toolkit into the service contracts violates layering.
Moving Item and LocalTerminal broadens unrelated scope. Generic erased Item
registration weakens checked admission. A broad ProjectSurfaceHost recreates the
shell, while opaque integer destinations lose original validation authority.

## Consequences

Providers name lower-layer contracts without exposing native dock mutation.
One short-lived destination entity retains each terminal intent and releases with
it. Workspace still owns placement and selection, including first hidden completion
and stale callback refusal. Provider installation and bootstrap composition remain
explicit; there are no new globals, services or external dependencies.

This extends [ADR0062](0062-terminal-contracts-below-the-workspace.md) and
[ADR0071](0071-provider-workspace-locations.md).
