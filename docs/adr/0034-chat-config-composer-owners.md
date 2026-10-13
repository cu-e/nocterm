# 0034. Per-chat configuration and composer transitions have private owners

## Context

Desired configuration and prompt model metadata belonged to AgentThread, while
reported options, modes and commands lived in the transcript reducer. Opening
requests carried attempted choices outside either owner. Panel consumers mutated
composer collections directly, making queue admission and editing invariants
hard to preserve across call sites.

## Decision

The feature layer owns ChatConfig and Composer with private state and named
transitions. ChatConfig owns desired choices, reported options and modes, slash
commands and their revision, model labels, bounded pre-open controls and the
current ticket's reconciliation attempts. The transcript reducer owns conversation
entries, timestamps, plan, usage, token counts and title.

Owner transitions perform no I/O or runtime lookups. AgentThread coordinates
persistence and requests in its existing session interpreter. Accepted durable
choices are saved before network dispatch. Opening reports preserve those choices
until reconciliation completes. Each available id/value and mode is attempted at
most once per ticket, and option responses reevaluate dependent choices. Stale
replies cannot modify a newer ticket's reports or attempts. Pre-open controls are
bounded to 64, deduplicated by actual session and kind, and drained only for the
session opened by the current ticket.

Restart snapshots retain desired choices, model labels and reported preview data;
they exclude pending controls and reconciliation. Command reports increment the
revision used by slash-command presentation caches. Model labels are recorded at
accepted prompt start rather than inferred from later reports.

Composer supplies immutable projections and named context, image, draft, editing
and queue transitions. Queue admission prepares a candidate without changing live
state. The coordinator validates structural and storage budgets, then commits the
candidate and associated draft/image changes. Rejected admission preserves the
queue, draft, images, editing defaults and next id. Editing retains the original
context separately and restores it when the edit ends.

Existing terminal approval, registration, attachment and original-epoch authority
continue through their narrow ports. Configuration extraction grants no terminal
authority and adds no background worker or persistence driver.

## Consequences

AgentThread remains a coordinator with other responsibilities awaiting extraction.
Existing ACP-backed configuration values and version-one history remain at their
current seams; this ownership decision introduces no protocol-specific domain API
and does not establish a protocol-neutral history format.
