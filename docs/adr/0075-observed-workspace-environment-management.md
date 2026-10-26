# 0075. Workspace tool changes retain explicit environment authority

Status: accepted

Supersedes the creation-only and custom-image tool restrictions in
[0074](0074-captured-host-image-search-and-language-versions.md).

## Context

Workspace settings need observed container and operating-system facts and explicit
language installation or updates. An image name cannot establish its actual OS,
libc, privileges or installed versions. Updating an existing environment must not
inherit creation's resource rollback, and a scoped container executor must not be
wrapped as a host executor a second time.

## Decision

`nocterm-project-providers` declares a separate `WorkspaceEnvironment` inspection
and installation contract. Its checked source retains the exact target, explicit
host or scoped execution, and optional original host authority for container
metadata. Observations and progress are ephemeral; catalog identity is unchanged.

`nocterm-project-runtime` probes the actual environment and selects Linux family,
architecture and libc strategies. One installation implementation serves new and
existing workspaces. Checked versions travel as arguments to fixed recipes.
Distribution defaults remain distinct from managed numeric versions; unavailable
combinations fail explicitly without substituting another version. Managed tools
use a private user namespace. Package installation requires existing privileges;
there is no implicit sudo or modification of user shell profiles.

`nocterm-project-service` admits and retains operations against their original
environment, serializes mutations, and participates in existing shutdown capture.
Management errors and cancellation preserve the environment and catalog. Partial
installation is reported; package changes are not represented as transactional.
Creation retains its existing exact owned-resource rollback.

The Projects feature captures an exact live workspace surface or original host
capability and separates tool changes from metadata Save. Disconnected remote
targets remain descriptive until ordinary connection recovery supplies authority.
Unknown providers never fall back to local execution.

An explicitly injected workspace session factory activates the fixed managed
paths in new folder workspace shells and their execution capability. Container
sessions apply the same activation inside their resource boundary. Ordinary
connection transports are unchanged. Existing running shells retain their own
environment; activation executes no mutable environment fragment or user profile.

Preparation progress carries neutral phase and optional observed byte counts.
The runtime decodes bounded engine output. The UI shows indeterminate progress
when no reliable total is available; a layer percentage is never presented as
the total pull percentage or a predicted completion time.

## Alternatives

- Inferring OS or language versions from an image tag produces false facts.
- Reusing creation cleanup can delete a user's existing container after a failed update.
- Reusing the active terminal can execute changes in a different environment.
- Expanding preparation into management couples unrelated lifecycle contracts.
- Running the same Debian script on every Linux image ignores musl and package differences.
- Editing shell startup files affects sessions outside the workspace.

## Consequences

Support depends on observed capabilities and upstream artifact availability.
Existing environment mutations may complete partially and require explicit retry.
Tool observations are refreshed rather than persisted as authoritative catalog
data. Domain values, adapter I/O, service lifetime, presentation and composition
retain separate owners.
