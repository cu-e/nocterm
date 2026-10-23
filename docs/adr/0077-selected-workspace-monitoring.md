# 0077. Resource monitoring follows the selected workspace environment

Status: accepted

Extends [0067](0067-project-tabs-with-inner-surfaces.md) and
[0071](0071-provider-workspace-locations.md).

## Context

The footer is owned by the window, while a workspace has an inner terminal
surface. Observing only the outer tab can miss that surface and select a local
runner. Host-and-route identity alone also conflates different containers on the
same server. Running host `/proc` CPU and memory collection in a container does
not establish that those counters describe the container.

## Decision

The workspace provides the narrow immutable `MonitorSource` projection for this
consumer. A project surface retains inert checked target metadata and supplies
its selected live inner session. Source observation follows context and session
changes without broadening ordinary active-session or container-panel semantics.
Missing, disconnected or unknown project authority never substitutes another
surface or a local executor. Genuine local contexts keep the injected local policy.

The Monitor feature owns source resolution, bounded per-resource history and
generation-checked watch acceptance. Identity includes the original surface and
checked resource, while executor identity and connection state determine restarts.
Old platform probes and frames cannot revive a watch after a selection change.
Collection still uses one supplied `HostExec` and the existing bounded watch
stream. It establishes no connection and never unwraps a scoped container runner.

The Monitor domain adds a pure Linux container collection choice. Fixed recipes
resolve the process's own cgroup membership and mounted v1/v2 controllers, then
read cumulative CPU usage and memory current/optional limit. CPU is explicitly
percent of one CPU and may exceed 100%; it is not an inferred quota percentage.
Unobservable limits retain current bytes without a fabricated memory percentage.
Host-only CPU cores, load, physical disks, sensors and boot uptime remain absent.
Missing or ambiguous accounting stays unavailable rather than reading host totals.

New parsing, script construction and counter arithmetic remain pure domain rules;
the existing watch mechanism executes the request through supplied authority.
No helper, package installer, generic service registry or provider-specific
monitoring connection is introduced.

## Alternatives

- Changing shared active-session resolution redirects unrelated consumers and
  obscures which context grants execution authority.
- Host-only cache keys mix unrelated resource history.
- Local fallback hides a disconnected workspace behind plausible unrelated data.
- Treating host `/proc` totals as container usage gives incorrect resource scope.
- Engine stats require a separate original-host capability even when the retained
  workspace session already supplies a usable scoped executor.

## Consequences

Regular folder workspaces keep existing server collection. Container accounting
depends on accessible Linux cgroup controllers and compatible standard utilities;
unavailable facts remain visible as unavailable. The monitor describes the
observed process cgroup, without inventing effective inherited quotas or limits.
Future providers need an explicit supported scope before they can be monitored.
