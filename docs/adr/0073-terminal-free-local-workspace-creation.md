# 0073. Terminal-free workspace creation uses explicitly injected local authority

Status: accepted

## Context

Workspace preparation needs a captured host executor and directory reader, but
local creation does not require a terminal. Requiring an ordinary connection made
This computer unavailable in an empty workspace. Reusing a synthetic session
receipt or falling back after failed remote capture would misrepresent authority.

## Decision

`ProjectCreationSource` retains an optional genuine ExpectedSession. Its checked
terminal constructor always records that receipt. Its distinct local constructor
fixes HostLocator::Local and retains an explicitly supplied HostExec without a
terminal receipt. Bootstrap injects the existing HostServices local executor into
Projects; shared directory readers provide local home discovery and browsing.

The current ordinary connection or active workspace takes precedence. A selected
remote connection, remote workspace or unknown provider that cannot capture its
source refuses creation. Absence of a selected server, or explicit local workspace
context, permits the injected local source. No remote capture error grants local
authority. The creation form retains the original source through discovery,
preparation and catalog publication.

Receipt absence never authorizes capture of another terminal. Saved local folder
and container workspaces use the existing provider resolver and ordinary terminal
lifecycle inside their original checked workspace surface. No hidden terminal,
new local transport or filesystem mutation fallback is introduced.

This extends [ADR 0070](0070-original-workspace-connection-recovery.md) and
[ADR 0071](0071-provider-workspace-locations.md). Directory reads retain
[ADR 0072](0072-shared-directory-selection.md) source semantics.

## Consequences

All three creation modes work on This computer before any terminal is open.
Remote creation remains bound to the original session. Terminal receipt handling
is explicit, and preparation capability remains separate from session ownership.
